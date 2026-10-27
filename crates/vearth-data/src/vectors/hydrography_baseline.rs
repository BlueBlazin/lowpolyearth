//! Hydrography regression baseline.
//!
//! The fixed tile deliberately separates the documented Version 4 contract,
//! extensions observed in immutable build 20260807, and synthetic recovery
//! cases. It freezes polygon output without admitting line or point water.

use super::*;
use crate::mvt::encode::{FeatureSpec, LayerSpec};
use crate::protomaps_observability::{
    SemanticDiscardReason, SemanticLayerFamily, SemanticTelemetry, SemanticValueClass,
    WaterSemanticOutcome,
};
use crate::protomaps_semantics::{
    LayerFacts, NormalizedValue, SemanticTile, SourceFeatureIdentity, WaterAdmissionCandidate,
    WaterAdmissionDecision, WaterGeometryRole, WaterKind, WaterKindDetail, WaterSurfaceDisposition,
    water_admission_decision,
};

const TILE_KEY: TileKey = (15, 16_384, 16_384);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Evidence {
    DocumentedV4,
    PinnedArchiveObservation,
    SyntheticRecovery,
}

const FIXTURE_EVIDENCE: &[(&str, Evidence)] = &[
    ("water/lake/ocean/playa/other", Evidence::DocumentedV4),
    (
        "basin/canal/ditch/dock/drain/lake/reservoir/river/riverbank/stream",
        Evidence::DocumentedV4,
    ),
    (
        "string bridge/tunnel; integer layer; boolean reservoir/alkaline/intermittent",
        Evidence::DocumentedV4,
    ),
    (
        "top-level river/stream/canal/drain/ditch",
        Evidence::PinnedArchiveObservation,
    ),
    (
        "water-layer swimming_pool/fountain polygons and points",
        Evidence::PinnedArchiveObservation,
    ),
    (
        "future kind and invalid boolean encoding",
        Evidence::SyntheticRecovery,
    ),
];

fn square(x: i32, y: i32) -> Vec<Vec<[i32; 2]>> {
    vec![vec![[x, y], [x + 80, y], [x + 80, y + 80], [x, y + 80]]]
}

fn line(x: i32, y: i32) -> Vec<Vec<[i32; 2]>> {
    vec![vec![[x, y], [x + 160, y + 40]]]
}

fn point(x: i32, y: i32) -> Vec<Vec<[i32; 2]>> {
    vec![vec![[x, y]]]
}

fn fixture_bytes() -> Vec<u8> {
    let values = [
        Value::Str("water".into()),         // 0
        Value::Str("lake".into()),          // 1
        Value::Str("ocean".into()),         // 2
        Value::Str("playa".into()),         // 3
        Value::Str("other".into()),         // 4
        Value::Str("reservoir".into()),     // 5
        Value::Str("basin".into()),         // 6
        Value::Str("dock".into()),          // 7
        Value::Str("riverbank".into()),     // 8
        Value::Str("river".into()),         // 9
        Value::Str("stream".into()),        // 10
        Value::Str("canal".into()),         // 11
        Value::Str("drain".into()),         // 12
        Value::Str("ditch".into()),         // 13
        Value::Str("yes".into()),           // 14
        Value::Str("future_water".into()),  // 15
        Value::Str("sometimes".into()),     // 16
        Value::Str("Fixture Lake".into()),  // 17
        Value::Bool(true),                  // 18
        Value::I64(-1),                     // 19
        Value::F64(8.0),                    // 20
        Value::Str("swimming_pool".into()), // 21
        Value::Str("fountain".into()),      // 22
        Value::I64(42),                     // 23
    ];
    let keys = [
        "kind",
        "kind_detail",
        "name",
        "layer",
        "bridge",
        "tunnel",
        "reservoir",
        "alkaline",
        "intermittent",
        "min_zoom",
        "sort_rank",
    ];
    let polygon = |id, index, kind, extras: &[(u32, u32)]| FeatureSpec {
        id: Some(id),
        geom_type: GeomType::Polygon,
        paths: square(100 + index * 120, 100),
        tags: std::iter::once((0, kind))
            .chain(extras.iter().copied())
            .collect(),
    };
    let line_feature = |id, index, kind, extras: &[(u32, u32)]| FeatureSpec {
        id: Some(id),
        geom_type: GeomType::Line,
        paths: line(100 + index * 180, 500),
        tags: std::iter::once((0, kind))
            .chain(extras.iter().copied())
            .collect(),
    };
    let point_feature = |id, index, kind, extras: &[(u32, u32)]| FeatureSpec {
        id: Some(id),
        geom_type: GeomType::Point,
        paths: point(100 + index * 180, 900),
        tags: std::iter::once((0, kind))
            .chain(extras.iter().copied())
            .collect(),
    };
    let features = vec![
        polygon(100, 0, 0, &[]),
        polygon(101, 1, 1, &[(2, 17), (9, 20), (10, 23)]),
        polygon(102, 2, 2, &[]),
        polygon(103, 3, 3, &[(7, 18), (8, 18)]),
        polygon(104, 4, 4, &[]),
        polygon(105, 5, 0, &[(1, 5), (6, 18)]),
        polygon(106, 6, 0, &[(1, 6)]),
        polygon(107, 7, 0, &[(1, 7)]),
        polygon(108, 8, 0, &[(1, 8)]),
        polygon(109, 9, 15, &[]),
        polygon(110, 10, 1, &[(8, 16)]),
        polygon(111, 11, 21, &[]),
        polygon(112, 12, 22, &[]),
        line_feature(200, 0, 9, &[(1, 9)]),
        line_feature(201, 1, 10, &[(1, 10), (8, 14)]),
        line_feature(202, 2, 11, &[(1, 11)]),
        line_feature(203, 3, 12, &[(1, 12)]),
        line_feature(204, 4, 13, &[(1, 13)]),
        line_feature(205, 5, 9, &[(1, 9), (4, 14)]),
        line_feature(206, 6, 12, &[(1, 12), (5, 14), (3, 19)]),
        point_feature(300, 0, 0, &[(1, 1), (2, 17)]),
        point_feature(301, 1, 21, &[]),
        point_feature(302, 2, 22, &[]),
    ];
    mvt::encode::tile_layers(&[LayerSpec {
        name: "water",
        extent: 4096,
        keys: &keys,
        values: &values,
        features: &features,
    }])
}

fn fixture_tile() -> SemanticTile {
    SemanticTile::normalize_at(mvt::decode_tile(&fixture_bytes()).unwrap(), TILE_KEY)
}

#[test]
fn fixture_has_explicit_contract_observation_and_recovery_evidence() {
    assert!(
        FIXTURE_EVIDENCE
            .iter()
            .any(|(_, evidence)| *evidence == Evidence::DocumentedV4)
    );
    assert!(
        FIXTURE_EVIDENCE
            .iter()
            .any(|(_, evidence)| *evidence == Evidence::PinnedArchiveObservation)
    );
    assert!(
        FIXTURE_EVIDENCE
            .iter()
            .any(|(_, evidence)| *evidence == Evidence::SyntheticRecovery)
    );
}

#[test]
fn fixture_preserves_geometry_identity_names_and_exact_flag_types() {
    let tile = fixture_tile();
    let layer = tile.semantic_layer("water").unwrap();
    assert_eq!(layer.features.len(), 23);
    assert_eq!(
        layer
            .features
            .iter()
            .filter(|feature| feature.geometry_type == GeomType::Polygon)
            .count(),
        13
    );
    assert_eq!(
        layer
            .features
            .iter()
            .filter(|feature| feature.geometry_type == GeomType::Line)
            .count(),
        7
    );
    assert_eq!(
        layer
            .features
            .iter()
            .filter(|feature| feature.geometry_type == GeomType::Point)
            .count(),
        3
    );
    assert!(
        layer
            .features
            .iter()
            .all(|feature| feature.source_id.is_some())
    );

    let facts = |index: usize| match &layer.features[index].facts {
        LayerFacts::Water(facts) => facts,
        _ => panic!("fixture feature must carry water facts"),
    };
    assert_eq!(
        tile.source_text(layer, facts(1).physical.kind.unwrap()),
        Some("lake")
    );
    assert_eq!(
        layer.features[1]
            .common
            .names
            .iter()
            .find_map(|name| tile.source_text(layer, name.value)),
        Some("Fixture Lake")
    );
    assert_eq!(
        layer.features[1].common.min_zoom.unwrap().normalized,
        NormalizedValue::Number(8.0)
    );
    assert_eq!(
        layer.features[1].common.sort_rank.unwrap().normalized,
        NormalizedValue::Number(42.0)
    );
    assert_eq!(
        tile.source_value(layer, facts(3).physical.alkaline.unwrap().source),
        Some(&Value::Bool(true))
    );
    assert_eq!(
        tile.source_value(layer, facts(5).physical.reservoir.unwrap().source),
        Some(&Value::Bool(true))
    );
    assert_eq!(
        facts(10).physical.intermittent.unwrap().normalized,
        NormalizedValue::Invalid
    );
    assert_eq!(
        facts(14).physical.intermittent.unwrap().normalized,
        NormalizedValue::Boolean(true)
    );
    assert_eq!(
        tile.source_value(layer, facts(14).physical.intermittent.unwrap().source,),
        Some(&Value::Str("yes".into()))
    );
    assert_eq!(
        tile.source_value(layer, facts(18).physical.bridge.unwrap().source),
        Some(&Value::Str("yes".into()))
    );
    assert_eq!(
        tile.source_value(layer, facts(19).physical.tunnel.unwrap().source),
        Some(&Value::Str("yes".into()))
    );
    assert_eq!(
        tile.source_value(layer, facts(19).physical.layer.unwrap().source),
        Some(&Value::I64(-1))
    );
}

#[test]
fn typed_water_classifies_polygons_and_accepted_line_footprints() {
    let tile = fixture_tile();
    let telemetry = SemanticTelemetry::new_enabled("hydrography-h0", 3);
    telemetry.record_normalized_tile(&tile);
    let mut pack = FeaturePack::empty();
    let mut report = telemetry.consumer_report();
    append_current_tile_features(&mut pack, &tile, TILE_KEY, 15, (0.0, 0.0), &mut report);
    telemetry.record_consumer(&report);

    assert_eq!(pack.water.len(), 12);
    assert_eq!(pack.watercourses.len(), 6);
    assert_eq!(
        pack.watercourses
            .iter()
            .map(|watercourse| watercourse.kind)
            .collect::<Vec<_>>(),
        vec![
            WatercourseKind::River,
            WatercourseKind::Stream,
            WatercourseKind::Canal,
            WatercourseKind::Drain,
            WatercourseKind::Ditch,
            WatercourseKind::River,
        ]
    );
    assert!(
        pack.watercourses
            .iter()
            .all(|watercourse| watercourse.authority_zoom == 15)
    );
    assert!(
        pack.watercourses
            .iter()
            .all(|watercourse| watercourse.width_m == watercourse.kind.fallback_width_m())
    );
    assert!(pack.watercourses[1].intermittent);
    assert!(pack.watercourses[5].is_bridge);
    assert_eq!(
        pack.water.iter().filter(|water| water.is_ocean()).count(),
        1
    );
    assert!(pack.water.iter().all(|water| !water.rings.is_empty()));
    assert_eq!(
        pack.water
            .iter()
            .map(|water| water.kind)
            .collect::<Vec<_>>(),
        vec![
            WaterRegionKind::InlandWater,
            WaterRegionKind::Lake,
            WaterRegionKind::Ocean,
            WaterRegionKind::Other,
            WaterRegionKind::Reservoir,
            WaterRegionKind::Basin,
            WaterRegionKind::Dock,
            WaterRegionKind::River,
            WaterRegionKind::CompatibilityUnknown,
            WaterRegionKind::Lake,
            WaterRegionKind::Decorative,
            WaterRegionKind::Decorative,
        ]
    );
    let snapshot = telemetry.snapshot();
    assert_eq!(snapshot.layer_source(SemanticLayerFamily::Water), 23);
    assert_eq!(snapshot.layer_consumed(SemanticLayerFamily::Water), 18);
    assert_eq!(snapshot.layer_rejected(SemanticLayerFamily::Water), 0);
    assert_eq!(
        snapshot.discard_count(
            SemanticLayerFamily::Water,
            SemanticDiscardReason::ContextOnly,
        ),
        3
    );
    assert_eq!(
        snapshot.discard_count(
            SemanticLayerFamily::Water,
            SemanticDiscardReason::SurfaceIneligible,
        ),
        1
    );
    assert_eq!(
        snapshot.discard_count(
            SemanticLayerFamily::Water,
            SemanticDiscardReason::DeferredPhysical,
        ),
        1
    );
    assert_eq!(snapshot.layer_discarded(SemanticLayerFamily::Water), 5);
    assert_eq!(
        snapshot.water_outcome_count(
            WaterKind::Stream,
            WaterGeometryRole::Watercourse,
            WaterSemanticOutcome::Consumed,
        ),
        1
    );
    assert_eq!(
        snapshot.water_outcome_count(
            WaterKind::Fountain,
            WaterGeometryRole::Label,
            WaterSemanticOutcome::ContextOnly,
        ),
        1
    );
    assert_eq!(
        snapshot.water_outcome_count(
            WaterKind::Unknown,
            WaterGeometryRole::Area,
            WaterSemanticOutcome::Unknown,
        ),
        1
    );
    assert_eq!(
        snapshot.water_outcome_count(
            WaterKind::Lake,
            WaterGeometryRole::Area,
            WaterSemanticOutcome::Invalid,
        ),
        1
    );
    assert_eq!(
        snapshot.value_count(
            SemanticLayerFamily::Water,
            false,
            SemanticValueClass::Unknown,
        ),
        1
    );
}

#[test]
fn watercourse_mapping_accepts_exact_classes_and_defers_ambiguous_lines() {
    let values = [
        Value::Str("river".into()),
        Value::Str("water".into()),
        Value::Str("canal".into()),
        Value::Str("stream".into()),
        Value::Str("drain".into()),
        Value::Str("ditch".into()),
        Value::Str("bay".into()),
        Value::Str("strait".into()),
    ];
    let specs = [
        (0, vec![(0, 0)]),
        (1, vec![(0, 1), (1, 0)]),
        (2, vec![(0, 2)]),
        (3, vec![(0, 1), (1, 2)]),
        (4, vec![(0, 3)]),
        (5, vec![(0, 4)]),
        (6, vec![(0, 5)]),
        (7, vec![(0, 6)]),
        (8, vec![(0, 7)]),
        (9, vec![(0, 1)]),
    ];
    let features = specs
        .into_iter()
        .map(|(index, tags)| FeatureSpec {
            id: Some(700 + index),
            geom_type: GeomType::Line,
            paths: line(100 + index as i32 * 180, 500),
            tags,
        })
        .collect::<Vec<_>>();
    let encoded = mvt::encode::tile_layers(&[LayerSpec {
        name: "water",
        extent: 4096,
        keys: &["kind", "kind_detail"],
        values: &values,
        features: &features,
    }]);
    let tile = SemanticTile::normalize_at(mvt::decode_tile(&encoded).unwrap(), TILE_KEY);
    let telemetry = SemanticTelemetry::new_enabled("hydrography-h3-mapping", 3);
    let mut pack = FeaturePack::empty();
    let mut report = telemetry.consumer_report();
    append_current_tile_features(&mut pack, &tile, TILE_KEY, 15, (0.0, 0.0), &mut report);
    telemetry.record_consumer(&report);

    assert_eq!(
        pack.watercourses
            .iter()
            .map(|watercourse| watercourse.kind)
            .collect::<Vec<_>>(),
        vec![
            WatercourseKind::River,
            WatercourseKind::River,
            WatercourseKind::Canal,
            WatercourseKind::Canal,
            WatercourseKind::Stream,
            WatercourseKind::Drain,
            WatercourseKind::Ditch,
        ]
    );
    assert_eq!(
        telemetry.snapshot().discard_count(
            SemanticLayerFamily::Water,
            SemanticDiscardReason::DeferredPhysical,
        ),
        3
    );
}

#[test]
fn watercourse_fragment_reconciliation_is_zoom_preferred_and_seam_stable() {
    let make = |identity, source_tile, source_feature_index, point, style_identity| Watercourse {
        paths: vec![vec![point, [point[0] + 1.0, point[1] + 1.0]]],
        kind: WatercourseKind::River,
        width_m: WatercourseKind::River.fallback_width_m(),
        identity,
        style_identity,
        source_tile,
        authority_zoom: 15,
        source_feature_index,
        source_min_zoom: None,
        fragment_count: 1,
        intermittent: false,
        is_bridge: false,
    };
    let mut watercourses = vec![
        make(
            SourceFeatureIdentity::Provided(42),
            (12, 100, 200),
            8,
            [0.0, 0.0],
            42,
        ),
        make(
            SourceFeatureIdentity::Provided(42),
            (13, 200, 400),
            4,
            [1.0, 0.0],
            42,
        ),
        make(
            SourceFeatureIdentity::Provided(42),
            (13, 201, 400),
            7,
            [2.0, 0.0],
            42,
        ),
        make(
            SourceFeatureIdentity::Provided(42),
            (13, 201, 400),
            7,
            [2.0, 0.0],
            42,
        ),
        make(
            SourceFeatureIdentity::DeterministicFallback(7),
            (13, 202, 400),
            3,
            [3.0, 0.0],
            WatercourseKind::River.style_tag(),
        ),
        make(
            SourceFeatureIdentity::DeterministicFallback(8),
            (13, 203, 400),
            2,
            [4.0, 0.0],
            WatercourseKind::River.style_tag(),
        ),
    ];
    let mut reversed = watercourses.clone();
    reversed.reverse();

    reconcile_watercourse_fragments(&mut watercourses);
    reconcile_watercourse_fragments(&mut reversed);

    assert_eq!(watercourses, reversed);
    assert_eq!(watercourses.len(), 3);
    let provided = watercourses
        .iter()
        .find(|watercourse| watercourse.identity == SourceFeatureIdentity::Provided(42))
        .unwrap();
    assert_eq!(provided.source_tile.0, 13);
    assert_eq!(provided.paths.len(), 2);
    assert_eq!(provided.fragment_count, 2);
    let fallbacks = watercourses
        .iter()
        .filter(|watercourse| {
            matches!(
                watercourse.identity,
                SourceFeatureIdentity::DeterministicFallback(_)
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(fallbacks.len(), 2);
    assert_eq!(fallbacks[0].style_identity, fallbacks[1].style_identity);
}

#[test]
fn watercourse_fallback_widths_and_lod_are_conservative_and_ordered() {
    let expected = [
        (WatercourseKind::River, 12.0, 14),
        (WatercourseKind::Canal, 6.0, 15),
        (WatercourseKind::Stream, 2.5, 16),
        (WatercourseKind::Drain, 1.5, 17),
        (WatercourseKind::Ditch, 1.0, 18),
    ];
    for (kind, width, level) in expected {
        assert_eq!(kind.fallback_width_m(), width);
        assert_eq!(kind.minimum_terrain_level(), level);
        assert!(vearth_core::cell::sample_pitch_m(level) <= f64::from(width * 2.0));
        assert!(vearth_core::cell::sample_pitch_m(level - 1) > f64::from(width * 2.0));
    }
    assert!(expected.windows(2).all(|pair| pair[0].1 > pair[1].1));
    assert!(expected.windows(2).all(|pair| pair[0].2 <= pair[1].2));
}

#[test]
fn watercourse_budget_rejects_the_whole_line_channel_and_marks_incomplete() {
    let telemetry = SemanticTelemetry::new_enabled("hydrography-h3-budget", 3);
    let mut report = telemetry.consumer_report();
    report.consumed(SemanticLayerFamily::Water);
    report.water_outcome(
        WaterKind::River,
        WaterGeometryRole::Watercourse,
        WaterSemanticOutcome::Consumed,
    );
    let mut pack = FeaturePack::empty();
    pack.watercourses.push(Watercourse {
        paths: vec![vec![[0.0, 0.0], [1.0, 1.0]]],
        kind: WatercourseKind::River,
        width_m: WatercourseKind::River.fallback_width_m(),
        identity: SourceFeatureIdentity::Provided(99),
        style_identity: 99,
        source_tile: TILE_KEY,
        authority_zoom: 15,
        source_feature_index: 0,
        source_min_zoom: None,
        fragment_count: 1,
        intermittent: false,
        is_bridge: false,
    });

    assert!(enforce_watercourse_budget(&mut pack, 0, &mut report, 0));
    telemetry.record_consumer(&report);

    assert!(pack.watercourses.is_empty());
    assert!(!pack.complete);
    let snapshot = telemetry.snapshot();
    assert_eq!(snapshot.layer_consumed(SemanticLayerFamily::Water), 0);
    assert_eq!(snapshot.layer_discarded(SemanticLayerFamily::Water), 1);
    assert_eq!(
        snapshot.discard_count(
            SemanticLayerFamily::Water,
            SemanticDiscardReason::ConsumerBudget,
        ),
        1
    );
    assert_eq!(
        snapshot.water_outcome_count(
            WaterKind::River,
            WaterGeometryRole::Watercourse,
            WaterSemanticOutcome::Filtered,
        ),
        1
    );
}

#[test]
fn reef_is_typed_context_and_never_becomes_inland_surface_water() {
    let encoded = mvt::encode::tile(
        "water",
        4096,
        &["kind", "kind_detail"],
        &[Value::Str("reef".into()), Value::Str("rock".into())],
        &[FeatureSpec {
            id: Some(400),
            geom_type: GeomType::Polygon,
            paths: square(100, 100),
            tags: vec![(0, 0), (1, 1)],
        }],
    );
    let tile = SemanticTile::normalize_at(mvt::decode_tile(&encoded).unwrap(), TILE_KEY);
    let telemetry = SemanticTelemetry::new_enabled("hydrography-h2-reef", 3);
    telemetry.record_normalized_tile(&tile);
    let mut pack = FeaturePack::empty();
    let mut report = telemetry.consumer_report();
    append_current_tile_features(&mut pack, &tile, TILE_KEY, 15, (0.0, 0.0), &mut report);
    telemetry.record_consumer(&report);

    assert!(pack.water.is_empty());
    let snapshot = telemetry.snapshot();
    assert_eq!(snapshot.layer_consumed(SemanticLayerFamily::Water), 0);
    assert_eq!(snapshot.layer_discarded(SemanticLayerFamily::Water), 1);
    assert_eq!(
        snapshot.water_outcome_count(
            WaterKind::Reef,
            WaterGeometryRole::Area,
            WaterSemanticOutcome::Filtered,
        ),
        1
    );
}

#[test]
fn wet_polygon_retains_identity_and_hydrological_flags() {
    let encoded = mvt::encode::tile(
        "water",
        4096,
        &["kind", "reservoir", "intermittent", "alkaline"],
        &[
            Value::Str("lake".into()),
            Value::Bool(true),
            Value::Str("yes".into()),
        ],
        &[FeatureSpec {
            id: Some(401),
            geom_type: GeomType::Polygon,
            paths: square(100, 100),
            tags: vec![(0, 0), (1, 1), (2, 2), (3, 1)],
        }],
    );
    let tile = SemanticTile::normalize_at(mvt::decode_tile(&encoded).unwrap(), TILE_KEY);
    let mut pack = FeaturePack::empty();
    let mut report = SemanticConsumerReport::default();
    append_current_tile_features(&mut pack, &tile, TILE_KEY, 15, (0.0, 0.0), &mut report);

    assert_eq!(pack.water.len(), 1);
    let water = &pack.water[0];
    assert_eq!(water.kind, WaterRegionKind::Reservoir);
    assert_eq!(water.identity, SourceFeatureIdentity::Provided(401));
    assert!(water.intermittent && water.alkaline);
}

#[test]
fn every_known_top_level_kind_has_an_exact_polygon_role() {
    let values = crate::protomaps_semantics::KNOWN_WATER_KINDS
        .iter()
        .map(|value| Value::Str((*value).to_string()))
        .collect::<Vec<_>>();
    let features = values
        .iter()
        .enumerate()
        .map(|(index, _)| FeatureSpec {
            id: Some(500 + index as u64),
            geom_type: GeomType::Polygon,
            paths: square(100 + index as i32 * 100, 100),
            tags: vec![(0, index as u32)],
        })
        .collect::<Vec<_>>();
    let encoded = mvt::encode::tile_layers(&[LayerSpec {
        name: "water",
        extent: 4096,
        keys: &["kind"],
        values: &values,
        features: &features,
    }]);
    let tile = SemanticTile::normalize_at(mvt::decode_tile(&encoded).unwrap(), TILE_KEY);
    let mut pack = FeaturePack::empty();
    let mut report = SemanticConsumerReport::default();
    append_current_tile_features(&mut pack, &tile, TILE_KEY, 15, (0.0, 0.0), &mut report);

    assert_eq!(
        pack.water
            .iter()
            .map(|water| water.kind)
            .collect::<Vec<_>>(),
        vec![
            WaterRegionKind::InlandWater,
            WaterRegionKind::Lake,
            WaterRegionKind::Ocean,
            WaterRegionKind::Other,
            WaterRegionKind::Basin,
            WaterRegionKind::InlandWater,
            WaterRegionKind::Canal,
            WaterRegionKind::Drainage,
            WaterRegionKind::Dock,
            WaterRegionKind::Drainage,
            WaterRegionKind::Decorative,
            WaterRegionKind::River,
            WaterRegionKind::InlandWater,
            WaterRegionKind::InlandWater,
            WaterRegionKind::Stream,
            WaterRegionKind::Decorative,
        ]
    );
}

#[test]
fn polygon_gameplay_and_ecology_roles_are_explicit() {
    for kind in [
        WaterRegionKind::Ocean,
        WaterRegionKind::InlandWater,
        WaterRegionKind::Lake,
        WaterRegionKind::Reservoir,
        WaterRegionKind::River,
    ] {
        assert_eq!(
            kind.gameplay_role(),
            WaterGameplayRole::PotentiallyNavigable
        );
        assert!(kind.contributes_ecological_proximity());
    }
    for kind in [WaterRegionKind::Stream, WaterRegionKind::Drainage] {
        assert_eq!(kind.gameplay_role(), WaterGameplayRole::NonNavigable);
    }
    assert_eq!(
        WaterRegionKind::Decorative.gameplay_role(),
        WaterGameplayRole::NonNavigable
    );
    assert!(WaterRegionKind::Decorative.creates_surface_water());
    assert!(!WaterRegionKind::Decorative.contributes_ecological_proximity());
    assert_eq!(
        WaterRegionKind::Playa.gameplay_role(),
        WaterGameplayRole::DrySeasonal
    );
    assert_eq!(
        WaterRegionKind::Reef.gameplay_role(),
        WaterGameplayRole::UnderwaterContext
    );
    assert!(!WaterRegionKind::Playa.creates_surface_water());
    assert!(!WaterRegionKind::Reef.creates_surface_water());
    assert_eq!(
        WaterRegionKind::CompatibilityUnknown.gameplay_role(),
        WaterGameplayRole::CompatibilityUnknown
    );
    assert!(WaterRegionKind::CompatibilityUnknown.creates_surface_water());
}

#[test]
fn legacy_rollback_retains_the_complete_polygon_mask() {
    let tile = fixture_tile();
    let telemetry = SemanticTelemetry::new_enabled("hydrography-h2-legacy", 3);
    let mut pack = FeaturePack::empty();
    pack.water_polygon_mode = WaterPolygonMode::LegacyUndifferentiated;
    let mut report = telemetry.consumer_report();
    append_current_tile_features(&mut pack, &tile, TILE_KEY, 15, (0.0, 0.0), &mut report);
    telemetry.record_consumer(&report);

    assert_eq!(pack.water.len(), 13);
    assert_eq!(pack.watercourses.len(), 6);
    assert_eq!(pack.water[3].kind, WaterRegionKind::Playa);
    assert!(!pack.water[3].creates_surface_water());
    assert_eq!(
        telemetry
            .snapshot()
            .layer_consumed(SemanticLayerFamily::Water),
        19
    );
}

#[test]
fn fixture_classifies_exact_water_families_roles_and_surface_eligibility() {
    let tile = fixture_tile();
    let layer = tile.semantic_layer("water").unwrap();
    let facts = |index: usize| match &layer.features[index].facts {
        LayerFacts::Water(facts) => facts,
        _ => panic!("fixture feature must carry water facts"),
    };

    assert_eq!(facts(0).class, Some(WaterKind::Water));
    assert_eq!(facts(1).class, Some(WaterKind::Lake));
    assert_eq!(facts(2).class, Some(WaterKind::Ocean));
    assert_eq!(facts(3).class, Some(WaterKind::Playa));
    assert_eq!(facts(4).class, Some(WaterKind::Other));
    assert_eq!(facts(9).class, Some(WaterKind::Unknown));
    assert_eq!(facts(11).class, Some(WaterKind::SwimmingPool));
    assert_eq!(facts(12).class, Some(WaterKind::Fountain));
    assert_eq!(facts(5).detail_class, Some(WaterKindDetail::Reservoir));
    assert_eq!(facts(8).detail_class, Some(WaterKindDetail::Riverbank));
    assert_eq!(facts(13).class, Some(WaterKind::River));
    assert_eq!(facts(14).class, Some(WaterKind::Stream));
    assert_eq!(facts(15).class, Some(WaterKind::Canal));
    assert_eq!(facts(16).class, Some(WaterKind::Drain));
    assert_eq!(facts(17).class, Some(WaterKind::Ditch));

    assert!(layer.features[..13]
        .iter()
        .all(|feature| matches!(&feature.facts, LayerFacts::Water(facts) if facts.geometry_role == WaterGeometryRole::Area && facts.surface_disposition == WaterSurfaceDisposition::Candidate)));
    assert!(layer.features[13..19]
        .iter()
        .all(|feature| matches!(&feature.facts, LayerFacts::Water(facts) if facts.geometry_role == WaterGeometryRole::Watercourse && facts.surface_disposition == WaterSurfaceDisposition::Candidate)));
    assert_eq!(facts(19).geometry_role, WaterGeometryRole::Watercourse);
    assert_eq!(
        facts(19).surface_disposition,
        WaterSurfaceDisposition::SurfaceIneligible
    );
    assert!(layer.features[20..]
        .iter()
        .all(|feature| matches!(&feature.facts, LayerFacts::Water(facts) if facts.geometry_role == WaterGeometryRole::Label && facts.surface_disposition == WaterSurfaceDisposition::ContextOnly)));
}

#[test]
fn water_admission_is_zoom_preferred_and_fragment_companion_safe() {
    let base = WaterAdmissionCandidate {
        identity: SourceFeatureIdentity::Provided(42),
        geometry_role: WaterGeometryRole::Watercourse,
        source_tile: (12, 100, 200),
        source_feature_index: 8,
    };
    assert_eq!(
        water_admission_decision(
            base,
            WaterAdmissionCandidate {
                source_tile: (13, 200, 400),
                ..base
            },
        ),
        WaterAdmissionDecision::ReplaceExisting
    );
    assert_eq!(
        water_admission_decision(
            base,
            WaterAdmissionCandidate {
                source_tile: (12, 101, 200),
                ..base
            },
        ),
        WaterAdmissionDecision::KeepBothFragments
    );
    assert_eq!(
        water_admission_decision(
            base,
            WaterAdmissionCandidate {
                geometry_role: WaterGeometryRole::Area,
                ..base
            },
        ),
        WaterAdmissionDecision::KeepBothCompanions
    );
    assert_eq!(
        water_admission_decision(
            base,
            WaterAdmissionCandidate {
                source_feature_index: 7,
                ..base
            },
        ),
        WaterAdmissionDecision::ReplaceExisting
    );
    assert_eq!(
        water_admission_decision(
            base,
            WaterAdmissionCandidate {
                identity: SourceFeatureIdentity::DeterministicFallback(99),
                ..base
            },
        ),
        WaterAdmissionDecision::KeepBothDistinct
    );
}

#[test]
fn equivalent_source_bytes_normalize_identically_for_blocking_and_async_publication() {
    let bytes = fixture_bytes();
    let blocking = SemanticTile::normalize_at(mvt::decode_tile(&bytes).unwrap(), TILE_KEY);
    let asynchronous = SemanticTile::normalize_at(mvt::decode_tile(&bytes).unwrap(), TILE_KEY);
    assert_eq!(blocking.semantic_layers(), asynchronous.semantic_layers());
    assert_eq!(blocking.raw_heap_bytes(), asynchronous.raw_heap_bytes());
    assert_eq!(
        blocking.semantic_heap_bytes(),
        asynchronous.semantic_heap_bytes()
    );
}
