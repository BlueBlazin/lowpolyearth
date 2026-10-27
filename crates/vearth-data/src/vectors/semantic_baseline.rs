//! Semantic-decoding regression baseline.
//!
//! The fixture is constructed from the test-only MVT encoder so its bytes are
//! deterministic and reviewable beside the assertions. It deliberately mixes
//! the official Version 4 contract, pinned-archive observations, and synthetic
//! recovery cases; `FIXTURE_FACTS` keeps those evidence classes explicit.

use super::*;
use crate::mvt::encode::{FeatureSpec, LayerSpec};
use crate::protomaps_observability::{SemanticLayerFamily, SemanticTelemetry, SemanticValueClass};
use crate::protomaps_semantics::{LandcoverKind, NameRole, NormalizedValue, SourceFeatureIdentity};
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Evidence {
    DocumentedV4,
    PinnedArchiveObservation,
    SyntheticRecovery,
}

#[derive(Debug, Clone, Copy)]
struct FixtureFact {
    layer: &'static str,
    fact: &'static str,
    evidence: Evidence,
}

const FIXTURE_FACTS: &[FixtureFact] = &[
    FixtureFact {
        layer: "common",
        fact: "name and name:en",
        evidence: Evidence::DocumentedV4,
    },
    FixtureFact {
        layer: "common",
        fact: "pgf:name:en and script",
        evidence: Evidence::DocumentedV4,
    },
    FixtureFact {
        layer: "common",
        fact: "sort_rank and min_zoom",
        evidence: Evidence::DocumentedV4,
    },
    FixtureFact {
        layer: "buildings",
        fact: "building polygon with height/min_height",
        evidence: Evidence::DocumentedV4,
    },
    FixtureFact {
        layer: "buildings",
        fact: "address point and addr_housenumber",
        evidence: Evidence::DocumentedV4,
    },
    FixtureFact {
        layer: "buildings",
        fact: "layer and sort_rank",
        evidence: Evidence::DocumentedV4,
    },
    FixtureFact {
        layer: "buildings",
        fact: "roof-only detail",
        evidence: Evidence::PinnedArchiveObservation,
    },
    FixtureFact {
        layer: "buildings",
        fact: "numeric height encoded as a unit string",
        evidence: Evidence::SyntheticRecovery,
    },
    FixtureFact {
        layer: "roads",
        fact: "string kind_detail",
        evidence: Evidence::DocumentedV4,
    },
    FixtureFact {
        layer: "roads",
        fact: "ref, network, shield_text, service and oneway",
        evidence: Evidence::DocumentedV4,
    },
    FixtureFact {
        layer: "roads",
        fact: "boolean is_bridge and is_tunnel",
        evidence: Evidence::DocumentedV4,
    },
    FixtureFact {
        layer: "roads",
        fact: "surface and width",
        evidence: Evidence::PinnedArchiveObservation,
    },
    FixtureFact {
        layer: "roads",
        fact: "unknown ferry terrain kind",
        evidence: Evidence::DocumentedV4,
    },
    FixtureFact {
        layer: "roads",
        fact: "out-of-range property value index",
        evidence: Evidence::SyntheticRecovery,
    },
    FixtureFact {
        layer: "water",
        fact: "polygon, line and point geometries",
        evidence: Evidence::DocumentedV4,
    },
    FixtureFact {
        layer: "water",
        fact: "string kind_detail, bridge and tunnel",
        evidence: Evidence::DocumentedV4,
    },
    FixtureFact {
        layer: "water",
        fact: "layer, intermittent, reservoir and alkaline",
        evidence: Evidence::DocumentedV4,
    },
    FixtureFact {
        layer: "water",
        fact: "unknown polygon kind",
        evidence: Evidence::SyntheticRecovery,
    },
    FixtureFact {
        layer: "landuse",
        fact: "forest, park and farmland polygons",
        evidence: Evidence::DocumentedV4,
    },
    FixtureFact {
        layer: "landuse",
        fact: "sport context",
        evidence: Evidence::DocumentedV4,
    },
    FixtureFact {
        layer: "landuse",
        fact: "unknown scrub compatibility case",
        evidence: Evidence::DocumentedV4,
    },
    FixtureFact {
        layer: "landcover",
        fact: "coarse farmland and forest polygons",
        evidence: Evidence::DocumentedV4,
    },
    FixtureFact {
        layer: "pois",
        fact: "open-ended kinds, names, rank and identity",
        evidence: Evidence::DocumentedV4,
    },
    FixtureFact {
        layer: "pois",
        fact: "tree kind",
        evidence: Evidence::PinnedArchiveObservation,
    },
    FixtureFact {
        layer: "places",
        fact: "kind_detail, capital and population",
        evidence: Evidence::DocumentedV4,
    },
    FixtureFact {
        layer: "places",
        fact: "Wikidata stable identity",
        evidence: Evidence::DocumentedV4,
    },
    FixtureFact {
        layer: "boundaries",
        fact: "integer kind_detail",
        evidence: Evidence::DocumentedV4,
    },
    FixtureFact {
        layer: "boundaries",
        fact: "disputed and maritime flags",
        evidence: Evidence::DocumentedV4,
    },
    FixtureFact {
        layer: "earth",
        fact: "land polygon",
        evidence: Evidence::DocumentedV4,
    },
    FixtureFact {
        layer: "earth",
        fact: "cliff line candidate",
        evidence: Evidence::PinnedArchiveObservation,
    },
    FixtureFact {
        layer: "natural",
        fact: "named point used by location resolution",
        evidence: Evidence::PinnedArchiveObservation,
    },
    FixtureFact {
        layer: "physical_point",
        fact: "named point used by location resolution",
        evidence: Evidence::PinnedArchiveObservation,
    },
];

fn polygon(x: i32, y: i32) -> Vec<Vec<[i32; 2]>> {
    vec![vec![[x, y], [x + 120, y], [x + 120, y + 120], [x, y + 120]]]
}

fn line(x: i32, y: i32) -> Vec<Vec<[i32; 2]>> {
    vec![vec![[x, y], [x + 160, y + 80], [x + 320, y]]]
}

fn point(x: i32, y: i32) -> Vec<Vec<[i32; 2]>> {
    vec![vec![[x, y]]]
}

fn fixture_tile() -> mvt::VectorTile {
    let building_keys = [
        "kind",
        "height",
        "min_height",
        "addr_housenumber",
        "layer",
        "sort_rank",
        "kind_detail",
        "roof:shape",
    ];
    let building_values = [
        Value::Str("building".into()),
        Value::Str("60 m".into()),
        Value::I64(3),
        Value::Str("10A".into()),
        Value::I64(2),
        Value::I64(180),
        Value::Str("roof".into()),
        Value::Str("gabled".into()),
        Value::Str("address".into()),
    ];
    let building_features = [
        FeatureSpec {
            id: Some(100),
            geom_type: GeomType::Polygon,
            paths: polygon(100, 100),
            tags: (0..8).map(|index| (index, index)).collect(),
        },
        FeatureSpec {
            id: Some(101),
            geom_type: GeomType::Point,
            paths: point(180, 180),
            tags: vec![(0, 8), (3, 3), (4, 4), (5, 5)],
        },
    ];

    let road_keys = [
        "kind",
        "kind_detail",
        "ref",
        "network",
        "service",
        "oneway",
        "is_bridge",
        "is_tunnel",
        "surface",
        "width",
        "shield_text",
    ];
    let road_values = [
        Value::Str("major_road".into()),
        Value::Str("primary".into()),
        Value::Str("A1".into()),
        Value::Str("US:I".into()),
        Value::Str("driveway".into()),
        Value::Str("yes".into()),
        Value::Bool(true),
        Value::Bool(false),
        Value::Str("asphalt".into()),
        Value::Str("7.5 m".into()),
        Value::Bool(true),
        Value::Str("ferry".into()),
        Value::Str("A1".into()),
    ];
    let road_features = [
        FeatureSpec {
            id: Some(200),
            geom_type: GeomType::Line,
            paths: line(300, 300),
            tags: (0..10)
                .map(|index| (index, index))
                .chain(std::iter::once((10, 12)))
                .collect(),
        },
        FeatureSpec {
            id: Some(201),
            geom_type: GeomType::Line,
            paths: line(300, 500),
            tags: vec![(0, 0), (7, 10)],
        },
        FeatureSpec {
            id: Some(202),
            geom_type: GeomType::Line,
            paths: line(300, 700),
            tags: vec![(0, 11)],
        },
        FeatureSpec {
            id: Some(203),
            geom_type: GeomType::Line,
            paths: line(300, 900),
            tags: vec![(0, 999)],
        },
    ];

    let water_keys = [
        "kind",
        "kind_detail",
        "bridge",
        "tunnel",
        "layer",
        "name",
        "min_zoom",
        "intermittent",
        "reservoir",
        "alkaline",
    ];
    let water_values = [
        Value::Str("ocean".into()),
        Value::Str("river".into()),
        Value::Str("mystery_water".into()),
        Value::Str("canal".into()),
        Value::Str("yes".into()),
        Value::Str("no".into()),
        Value::I64(-1),
        Value::Str("Fixture River".into()),
        Value::F64(6.0),
        Value::Bool(true),
        Value::Bool(false),
    ];
    let water_features = [
        FeatureSpec {
            id: Some(300),
            geom_type: GeomType::Polygon,
            paths: polygon(800, 100),
            tags: vec![(0, 0), (5, 7)],
        },
        FeatureSpec {
            id: Some(301),
            geom_type: GeomType::Line,
            paths: line(800, 350),
            tags: vec![
                (0, 1),
                (1, 3),
                (2, 4),
                (3, 5),
                (4, 6),
                (5, 7),
                (6, 8),
                (7, 9),
                (8, 10),
                (9, 10),
            ],
        },
        FeatureSpec {
            id: Some(302),
            geom_type: GeomType::Point,
            paths: point(900, 550),
            tags: vec![(0, 1), (5, 7)],
        },
        FeatureSpec {
            id: Some(303),
            geom_type: GeomType::Polygon,
            paths: polygon(800, 700),
            tags: vec![(0, 2)],
        },
    ];

    let landuse_keys = ["kind", "sport", "name"];
    let landuse_values = [
        Value::Str("forest".into()),
        Value::Str("park".into()),
        Value::Str("farmland".into()),
        Value::Str("scrub".into()),
        Value::Str("soccer".into()),
        Value::Str("Fixture Land".into()),
    ];
    let landuse_features = [
        FeatureSpec {
            id: Some(400),
            geom_type: GeomType::Polygon,
            paths: polygon(1300, 100),
            tags: vec![(0, 0)],
        },
        FeatureSpec {
            id: Some(401),
            geom_type: GeomType::Polygon,
            paths: polygon(1300, 300),
            tags: vec![(0, 1), (1, 4)],
        },
        FeatureSpec {
            id: Some(402),
            geom_type: GeomType::Polygon,
            paths: polygon(1300, 500),
            tags: vec![(0, 2)],
        },
        FeatureSpec {
            id: Some(403),
            geom_type: GeomType::Polygon,
            paths: polygon(1300, 700),
            tags: vec![(0, 3), (2, 5)],
        },
        FeatureSpec {
            id: Some(404),
            geom_type: GeomType::Point,
            paths: point(1350, 900),
            tags: vec![(0, 1)],
        },
    ];

    let landcover_keys = ["kind"];
    let landcover_values = [Value::Str("farmland".into()), Value::Str("forest".into())];
    let landcover_features = [
        FeatureSpec {
            id: Some(500),
            geom_type: GeomType::Polygon,
            paths: polygon(1600, 100),
            tags: vec![(0, 0)],
        },
        FeatureSpec {
            id: Some(501),
            geom_type: GeomType::Polygon,
            paths: polygon(1600, 300),
            tags: vec![(0, 1)],
        },
    ];

    let poi_keys = ["kind", "name", "min_zoom", "sort_rank", "wikidata"];
    let poi_values = [
        Value::Str("viewpoint".into()),
        Value::Str("Fixture View".into()),
        Value::F64(15.0),
        Value::I64(10),
        Value::Str("Q100".into()),
        Value::Str("restaurant".into()),
        Value::Str("tree".into()),
    ];
    let poi_features = [
        FeatureSpec {
            id: Some(600),
            geom_type: GeomType::Point,
            paths: point(160, 160),
            tags: vec![(0, 0), (1, 1), (2, 2), (3, 3), (4, 4)],
        },
        FeatureSpec {
            id: Some(601),
            geom_type: GeomType::Point,
            paths: point(1800, 300),
            tags: vec![(0, 5), (1, 1)],
        },
        FeatureSpec {
            id: Some(602),
            geom_type: GeomType::Point,
            paths: point(1800, 500),
            tags: vec![(0, 6)],
        },
    ];

    let place_keys = [
        "kind",
        "kind_detail",
        "name",
        "name:en",
        "pgf:name:en",
        "min_zoom",
        "sort_rank",
        "population",
        "capital",
        "wikidata",
        "script",
    ];
    let place_values = [
        Value::Str("locality".into()),
        Value::Str("city".into()),
        Value::Str("München".into()),
        Value::Str("Munich".into()),
        Value::Str("Munich".into()),
        Value::F64(5.0),
        Value::I64(20),
        Value::I64(1_500_000),
        Value::Str("yes".into()),
        Value::Str("Q1726".into()),
        Value::Str("Latn".into()),
    ];
    let place_features = [FeatureSpec {
        id: Some(700),
        geom_type: GeomType::Point,
        paths: point(2100, 300),
        tags: (0..11).map(|index| (index, index)).collect(),
    }];

    let boundary_keys = [
        "kind",
        "kind_detail",
        "disputed",
        "maritime",
        "name",
        "min_zoom",
    ];
    let boundary_values = [
        Value::Str("region".into()),
        Value::I64(4),
        Value::Bool(true),
        Value::Bool(false),
        Value::Str("Fixture Boundary".into()),
        Value::F64(0.0),
    ];
    let boundary_features = [FeatureSpec {
        id: Some(800),
        geom_type: GeomType::Line,
        paths: line(2300, 300),
        tags: (0..6).map(|index| (index, index)).collect(),
    }];

    let earth_keys = ["kind", "name"];
    let earth_values = [
        Value::Str("land".into()),
        Value::Str("cliff".into()),
        Value::Str("Fixture Scarp".into()),
    ];
    let earth_features = [
        FeatureSpec {
            id: Some(900),
            geom_type: GeomType::Polygon,
            paths: polygon(2600, 100),
            tags: vec![(0, 0)],
        },
        FeatureSpec {
            id: Some(901),
            geom_type: GeomType::Line,
            paths: line(2600, 350),
            tags: vec![(0, 1), (1, 2)],
        },
    ];

    let natural_keys = ["kind", "name", "min_zoom"];
    let natural_values = [
        Value::Str("peak".into()),
        Value::Str("Fixture Peak".into()),
        Value::F64(9.0),
    ];
    let natural_features = [FeatureSpec {
        id: Some(1000),
        geom_type: GeomType::Point,
        paths: point(2900, 200),
        tags: vec![(0, 0), (1, 1), (2, 2)],
    }];

    let physical_keys = ["kind", "name", "min_zoom"];
    let physical_values = [
        Value::Str("island".into()),
        Value::Str("Fixture Island".into()),
        Value::F64(8.0),
    ];
    let physical_features = [FeatureSpec {
        id: Some(1100),
        geom_type: GeomType::Point,
        paths: point(3100, 200),
        tags: vec![(0, 0), (1, 1), (2, 2)],
    }];

    let encoded = mvt::encode::tile_layers(&[
        LayerSpec {
            name: "buildings",
            extent: 4096,
            keys: &building_keys,
            values: &building_values,
            features: &building_features,
        },
        LayerSpec {
            name: "roads",
            extent: 4096,
            keys: &road_keys,
            values: &road_values,
            features: &road_features,
        },
        LayerSpec {
            name: "water",
            extent: 4096,
            keys: &water_keys,
            values: &water_values,
            features: &water_features,
        },
        LayerSpec {
            name: "landuse",
            extent: 4096,
            keys: &landuse_keys,
            values: &landuse_values,
            features: &landuse_features,
        },
        LayerSpec {
            name: "landcover",
            extent: 4096,
            keys: &landcover_keys,
            values: &landcover_values,
            features: &landcover_features,
        },
        LayerSpec {
            name: "pois",
            extent: 4096,
            keys: &poi_keys,
            values: &poi_values,
            features: &poi_features,
        },
        LayerSpec {
            name: "places",
            extent: 4096,
            keys: &place_keys,
            values: &place_values,
            features: &place_features,
        },
        LayerSpec {
            name: "boundaries",
            extent: 4096,
            keys: &boundary_keys,
            values: &boundary_values,
            features: &boundary_features,
        },
        LayerSpec {
            name: "earth",
            extent: 4096,
            keys: &earth_keys,
            values: &earth_values,
            features: &earth_features,
        },
        LayerSpec {
            name: "natural",
            extent: 4096,
            keys: &natural_keys,
            values: &natural_values,
            features: &natural_features,
        },
        LayerSpec {
            name: "physical_point",
            extent: 4096,
            keys: &physical_keys,
            values: &physical_values,
            features: &physical_features,
        },
    ]);
    mvt::decode_tile(&encoded).expect("constructed semantic baseline must decode")
}

#[test]
fn semantic_fixture_facts_have_explicit_evidence_classes() {
    assert!(
        FIXTURE_FACTS
            .iter()
            .any(|fact| fact.evidence == Evidence::DocumentedV4)
    );
    assert!(
        FIXTURE_FACTS
            .iter()
            .any(|fact| fact.evidence == Evidence::PinnedArchiveObservation)
    );
    assert!(
        FIXTURE_FACTS
            .iter()
            .any(|fact| fact.evidence == Evidence::SyntheticRecovery)
    );

    let mut labels = HashSet::new();
    for fact in FIXTURE_FACTS {
        assert!(
            labels.insert((fact.layer, fact.fact)),
            "duplicate fixture fact: {fact:?}"
        );
    }
}

#[test]
fn semantic_fixture_preserves_layer_scoped_types_and_context() {
    let tile = fixture_tile();
    assert_eq!(tile.layers.len(), 11);

    let buildings = tile.layer("buildings").unwrap();
    assert_eq!(buildings.features[1].geom_type, GeomType::Point);
    assert!(
        matches!(buildings.features[0].get(buildings, "height"), Some(Value::Str(value)) if value == "60 m")
    );
    assert!(
        matches!(buildings.features[1].get(buildings, "addr_housenumber"), Some(Value::Str(value)) if value == "10A")
    );

    let roads = tile.layer("roads").unwrap();
    assert!(
        matches!(roads.features[0].get(roads, "kind_detail"), Some(Value::Str(value)) if value == "primary")
    );
    assert!(
        matches!(roads.features[0].get(roads, "oneway"), Some(Value::Str(value)) if value == "yes")
    );
    assert_eq!(
        roads.features[0].get(roads, "is_bridge"),
        Some(&Value::Bool(true))
    );
    assert_eq!(
        roads.features[0].get(roads, "is_tunnel"),
        Some(&Value::Bool(false))
    );
    assert_eq!(
        roads.features[3].get(roads, "kind"),
        None,
        "bad value index is recoverable"
    );

    let water = tile.layer("water").unwrap();
    assert_eq!(water.features[0].geom_type, GeomType::Polygon);
    assert_eq!(water.features[1].geom_type, GeomType::Line);
    assert_eq!(water.features[2].geom_type, GeomType::Point);
    assert!(
        matches!(water.features[1].get(water, "bridge"), Some(Value::Str(value)) if value == "yes")
    );
    assert!(
        matches!(water.features[1].get(water, "tunnel"), Some(Value::Str(value)) if value == "no")
    );

    let boundaries = tile.layer("boundaries").unwrap();
    assert_eq!(
        boundaries.features[0].get(boundaries, "kind_detail"),
        Some(&Value::I64(4))
    );

    let places = tile.layer("places").unwrap();
    assert_eq!(english_name(&places.features[0], places), Some("Munich"));
    assert!(
        matches!(places.features[0].get(places, "capital"), Some(Value::Str(value)) if value == "yes")
    );
    assert!(
        matches!(places.features[0].get(places, "wikidata"), Some(Value::Str(value)) if value == "Q1726")
    );

    let earth = tile.layer("earth").unwrap();
    assert_eq!(earth.features[0].geom_type, GeomType::Polygon);
    assert_eq!(earth.features[1].geom_type, GeomType::Line);
}

#[test]
fn normalized_fixture_preserves_typed_physical_and_context_facts() {
    let tile = SemanticTile::normalize_at(fixture_tile(), (15, 16_384, 16_384));

    let buildings = tile.semantic_layer("buildings").unwrap();
    let LayerFacts::Buildings(building) = &buildings.features[0].facts else {
        panic!("building facts")
    };
    assert_eq!(
        building.physical.height.unwrap().normalized,
        NormalizedValue::Number(60.0)
    );
    assert_eq!(
        building.physical.min_height.unwrap().normalized,
        NormalizedValue::Number(3.0)
    );
    assert!(building.physical.roof_shape.is_some());
    let LayerFacts::Buildings(address) = &buildings.features[1].facts else {
        panic!("building address facts")
    };
    assert!(address.context.addr_housenumber.is_some());

    let roads = tile.semantic_layer("roads").unwrap();
    let LayerFacts::Roads(road) = &roads.features[0].facts else {
        panic!("road facts")
    };
    assert_eq!(
        road.physical.width.unwrap().normalized,
        NormalizedValue::Number(7.5)
    );
    assert!(road.physical.surface.is_some());
    assert!(road.context.reference.is_some());
    assert_eq!(road.context.networks.len(), 1);
    assert_eq!(road.context.shield_texts.len(), 1);
    let LayerFacts::Roads(invalid_road) = &roads.features[3].facts else {
        panic!("invalid road facts")
    };
    assert_eq!(
        invalid_road.physical.kind.unwrap().normalized,
        NormalizedValue::Invalid
    );

    let water = tile.semantic_layer("water").unwrap();
    let LayerFacts::Water(course) = &water.features[1].facts else {
        panic!("water facts")
    };
    assert_eq!(
        course.physical.intermittent.unwrap().normalized,
        NormalizedValue::Boolean(true)
    );
    assert_eq!(
        course.physical.reservoir.unwrap().normalized,
        NormalizedValue::Boolean(false)
    );
    assert_eq!(
        course.physical.alkaline.unwrap().normalized,
        NormalizedValue::Boolean(false)
    );
    let LayerFacts::Water(unknown) = &water.features[3].facts else {
        panic!("unknown water facts")
    };
    assert_eq!(
        unknown.physical.kind.unwrap().normalized,
        NormalizedValue::UnknownText
    );

    let places = tile.semantic_layer("places").unwrap();
    assert_eq!(places.features[0].common.names.len(), 4);
    assert!(
        places.features[0]
            .common
            .names
            .iter()
            .any(|name| name.role == NameRole::Local)
    );
    assert!(
        places.features[0]
            .common
            .names
            .iter()
            .any(|name| name.role == NameRole::Localized)
    );
    assert!(
        places.features[0]
            .common
            .names
            .iter()
            .any(|name| name.role == NameRole::Preferred)
    );
    assert!(
        places.features[0]
            .common
            .names
            .iter()
            .any(|name| name.role == NameRole::Script)
    );

    let earth = tile.semantic_layer("earth").unwrap();
    let cliff = &earth.features[1];
    assert_eq!(cliff.identity, SourceFeatureIdentity::Provided(901));
    assert_eq!(cliff.common.names.len(), 1);
    assert_eq!(
        tile.source_text(earth, cliff.common.names[0].value),
        Some("Fixture Scarp")
    );

    let boundaries = tile.semantic_layer("boundaries").unwrap();
    let LayerFacts::Boundaries(boundary) = &boundaries.features[0].facts else {
        panic!("boundary facts")
    };
    assert_eq!(
        boundary.maritime.unwrap().normalized,
        NormalizedValue::Boolean(false)
    );
    assert_eq!(
        boundary.kind_detail.unwrap().normalized,
        NormalizedValue::Number(4.0)
    );

    assert!(matches!(
        tile.semantic_layer("natural").unwrap().features[0].facts,
        LayerFacts::Natural(_)
    ));
    assert!(matches!(
        tile.semantic_layer("physical_point").unwrap().features[0].facts,
        LayerFacts::PhysicalPoint(_)
    ));
    assert_eq!(
        buildings.features[0].identity,
        SourceFeatureIdentity::Provided(100)
    );
    assert!(tile.diagnostics().iter().any(|diagnostic| {
        diagnostic.layer == "water"
            && diagnostic.field == "kind"
            && diagnostic.kind == crate::protomaps_semantics::SemanticDiagnosticKind::UnknownText
    }));
}

#[test]
fn semantic_location_facts_preserve_legacy_name_rank_population_and_identity() {
    let tile_key = (10, 512, 512);
    let raw = fixture_tile();
    let raw_places = raw.layer("places").unwrap();
    let raw_feature = &raw_places.features[0];
    let tile = SemanticTile::normalize_at(fixture_tile(), tile_key);
    let places = tile.semantic_layer("places").unwrap();
    let semantic = &places.features[0];

    assert_eq!(
        semantic_english_name(&tile, places, semantic),
        english_name(raw_feature, raw_places)
    );
    assert_eq!(
        semantic_feature_min_zoom(&tile, places, semantic, tile_key.0),
        number_any(raw_feature, raw_places, &["min_zoom"]).unwrap()
    );
    let population = number_any(raw_feature, raw_places, &["population"]).unwrap();
    let expected_radius = (1_500.0 + population.sqrt() * 20.0).clamp(2_000.0, 60_000.0);
    assert_eq!(
        semantic_point_influence_radius_m(&tile, places, semantic, tile_key.0),
        expected_radius
    );
    assert!(!semantic_administrative_label_point(
        &tile, places, semantic
    ));
    let name = semantic_english_name(&tile, places, semantic).unwrap();
    let mut expected_id = fnv1a(0xcbf2_9ce4_8422_2325, b"places");
    expected_id = fnv1a(expected_id, &[0]);
    expected_id = fnv1a(expected_id, &raw_feature.id.unwrap().to_le_bytes());
    assert_eq!(
        semantic_location_id(semantic.source_id, "places", name),
        expected_id
    );
}

#[test]
fn semantic_observability_reconciles_the_terrain_fixture() {
    let tile_key = (15, 16_384, 16_384);
    let tile = SemanticTile::normalize_at(fixture_tile(), tile_key);
    let telemetry = SemanticTelemetry::new_enabled("fixture", 0x1234);
    telemetry.record_normalized_tile(&tile);

    let mut pack = FeaturePack::empty();
    let mut detail = SemanticConsumerReport::default();
    append_current_tile_features(&mut pack, &tile, tile_key, 15, (0.0, 0.0), &mut detail);
    telemetry.record_consumer(&detail);
    let mut landcover = SemanticConsumerReport::default();
    append_semantic_land_regions(
        &mut pack,
        &tile,
        "landcover",
        15,
        16_384,
        16_384,
        0.0,
        0.0,
        &mut landcover,
    );
    telemetry.record_consumer(&landcover);
    let snapshot = telemetry.snapshot();

    for layer in [
        SemanticLayerFamily::Buildings,
        SemanticLayerFamily::Roads,
        SemanticLayerFamily::Water,
        SemanticLayerFamily::Landuse,
        SemanticLayerFamily::Landcover,
        SemanticLayerFamily::Pois,
    ] {
        assert_eq!(
            snapshot.layer_source(layer),
            snapshot.layer_normalized(layer)
        );
        assert_eq!(
            snapshot.layer_source(layer),
            snapshot.layer_consumed(layer)
                + snapshot.layer_rejected(layer)
                + snapshot.layer_discarded(layer),
            "source-to-consumer reconciliation for {}",
            layer.label()
        );
        assert_eq!(
            snapshot.layer_admitted(layer),
            snapshot.layer_consumed(layer)
        );
    }
    assert_eq!(snapshot.layer_consumed(SemanticLayerFamily::Buildings), 1);
    assert_eq!(snapshot.layer_rejected(SemanticLayerFamily::Roads), 3);
    assert_eq!(snapshot.layer_consumed(SemanticLayerFamily::Water), 2);
    assert_eq!(snapshot.layer_consumed(SemanticLayerFamily::Landuse), 4);
    assert_eq!(snapshot.layer_consumed(SemanticLayerFamily::Landcover), 2);
    assert_eq!(snapshot.layer_consumed(SemanticLayerFamily::Pois), 2);
    assert_eq!(
        snapshot.value_count(
            SemanticLayerFamily::Water,
            false,
            SemanticValueClass::Unknown,
        ),
        1
    );
    assert_eq!(
        snapshot.known_value_count(SemanticLayerFamily::Roads, "kind", "major_road"),
        2
    );
    assert_eq!(
        snapshot.known_value_count(SemanticLayerFamily::Landuse, "kind", "farmland"),
        1
    );
    assert_eq!(
        snapshot.known_value_count(SemanticLayerFamily::Earth, "kind", "cliff"),
        1
    );
    assert_eq!(
        snapshot.value_count(
            SemanticLayerFamily::Roads,
            false,
            SemanticValueClass::Invalid,
        ),
        1
    );
    assert!(snapshot.unknown_examples.len() >= 2);
    let developer_report = snapshot.to_string();
    assert!(developer_report.contains("build=fixture semantic=v6 metadata=0000000000001234"));
    assert!(developer_report.contains("unknown water.kind=mystery_water"));
    assert!(developer_report.contains("invalid_value="));
}

#[test]
fn semantic_feature_pack_preserves_foundation_output_plus_reviewed_natural_delta() {
    let tile_key = (15, 16_384, 16_384);
    let tile = SemanticTile::normalize_at(fixture_tile(), tile_key);
    let mut pack = FeaturePack::empty();
    let mut detail_report = SemanticConsumerReport::default();
    append_current_tile_features(
        &mut pack,
        &tile,
        tile_key,
        15,
        (0.0, 0.0),
        &mut detail_report,
    );
    let mut landcover_report = SemanticConsumerReport::default();
    append_semantic_land_regions(
        &mut pack,
        &tile,
        "landcover",
        15,
        16_384,
        16_384,
        0.0,
        0.0,
        &mut landcover_report,
    );

    let mut legacy = FeaturePack::empty();
    append_legacy_tile_features(
        &mut legacy,
        tile.decoded(),
        tile_key.0,
        tile_key.1,
        tile_key.2,
        0.0,
        0.0,
    );
    let regional_delta = std::mem::take(&mut pack.regional_landcover);
    assert_eq!(regional_delta.len(), 2);
    assert_eq!(regional_delta[0].kind, LandcoverKind::Farmland);
    assert_eq!(regional_delta[0].source_id, Some(500));
    assert_eq!(regional_delta[1].kind, LandcoverKind::Forest);
    assert_eq!(regional_delta[1].source_id, Some(501));
    // The legacy adapter collapsed z7 farmland into detailed flora. Remove
    // that one legacy-only representation before comparing unchanged output.
    legacy.flora.remove(1);
    let natural_delta = pack
        .ground
        .iter()
        .position(|(kind, _)| *kind == GroundKind::Scrub)
        .map(|index| (index, pack.ground.remove(index)))
        .expect("semantic fixture must emit its scrub delta");
    let exact_tree_delta = std::mem::take(&mut pack.exact_trees);
    let cliff_delta = std::mem::take(&mut pack.cliffs);
    assert_feature_pack_parity(&pack, &legacy);
    pack.ground.insert(natural_delta.0, natural_delta.1);
    pack.regional_landcover = regional_delta;
    pack.exact_trees = exact_tree_delta;
    pack.cliffs = cliff_delta;

    assert_eq!(
        pack.buildings.len(),
        1,
        "address points remain context-only"
    );
    assert!((pack.buildings[0].height_m - 60.0).abs() < 0.01);
    assert!((pack.buildings[0].min_height_m - 3.0).abs() < 0.01);
    assert_eq!(
        pack.observation_hints.len(),
        1,
        "restaurant stays non-rendering while the viewpoint remains a hint"
    );
    assert_eq!(
        pack.observation_hints[0].kind,
        ObservationHintKind::Viewpoint
    );
    assert_eq!(pack.exact_trees.len(), 1, "tree POI becomes physical input");
    assert_eq!(
        pack.exact_trees[0].identity,
        SourceFeatureIdentity::Provided(602)
    );
    assert_eq!(
        pack.cliffs.len(),
        1,
        "earth cliff line becomes physical input"
    );
    assert_eq!(
        pack.cliffs[0].identity,
        SourceFeatureIdentity::Provided(901)
    );
    assert_eq!(pack.cliffs[0].pts.len(), 3);

    assert_eq!(
        pack.roads.len(),
        1,
        "tunnel, ferry and invalid-kind lines are dropped"
    );
    assert_eq!(pack.roads[0].class, RoadClass::Major);
    assert!(pack.roads[0].is_bridge);

    assert_eq!(
        pack.water.len(),
        2,
        "all non-empty polygons are emitted; lines and labels are dropped"
    );
    assert_eq!(
        pack.water.iter().filter(|water| water.is_ocean()).count(),
        1
    );
    assert_eq!(
        pack.forest.len(),
        1,
        "only landuse forest/wood enters the forest output"
    );
    assert_eq!(
        pack.ground.len(),
        2,
        "foundation park plus the reviewed scrub delta enter ground"
    );
    assert_eq!(pack.ground[0].0, GroundKind::Green);
    assert_eq!(pack.ground[1].0, GroundKind::Scrub);
    assert_eq!(
        pack.flora.len(),
        1,
        "only detailed landuse farmland enters detailed flora"
    );
    assert!(
        pack.flora
            .iter()
            .all(|region| region.kind == FloraRegionKind::Farmland)
    );
    assert_eq!(pack.regional_landcover.len(), 2);
    assert!(pack.complete);

    assert!(
        tile.decoded().layer("places").is_some(),
        "context facts survive MVT decode but not FeaturePack"
    );
    assert!(
        tile.decoded().layer("boundaries").is_some(),
        "boundary context is currently dropped"
    );
    assert!(tile.decoded().layer("earth").is_some());
}

#[test]
fn exact_tree_seed_is_source_identity_owned_across_tile_copies() {
    let extract = |tile_key| {
        let tile = SemanticTile::normalize_at(fixture_tile(), tile_key);
        let mut pack = FeaturePack::empty();
        append_current_tile_features(
            &mut pack,
            &tile,
            tile_key,
            15,
            (0.0, 0.0),
            &mut SemanticConsumerReport::default(),
        );
        assert_eq!(pack.exact_trees.len(), 1);
        pack.exact_trees[0]
    };
    let first = extract((15, 16_384, 16_384));
    let adjacent = extract((15, 16_385, 16_384));
    assert_eq!(first.identity, adjacent.identity);
    assert_eq!(first.seed, adjacent.seed);
}

fn assert_feature_pack_parity(actual: &FeaturePack, expected: &FeaturePack) {
    assert_eq!(actual.complete, expected.complete);
    assert_eq!(actual.bridge_authority, expected.bridge_authority);
    assert_eq!(actual.buildings.len(), expected.buildings.len());
    for (actual, expected) in actual.buildings.iter().zip(&expected.buildings) {
        assert_eq!(actual.rings, expected.rings);
        assert_eq!(actual.height_m, expected.height_m);
        assert_eq!(actual.min_height_m, expected.min_height_m);
        assert_eq!(actual.seed, expected.seed);
        assert_eq!(actual.kind, expected.kind);
    }
    assert_eq!(
        actual.observation_hints.len(),
        expected.observation_hints.len()
    );
    for (actual, expected) in actual
        .observation_hints
        .iter()
        .zip(&expected.observation_hints)
    {
        assert_eq!(actual.point, expected.point);
        assert_eq!(actual.kind, expected.kind);
        assert_eq!(actual.seed, expected.seed);
    }
    assert_eq!(actual.exact_trees, expected.exact_trees);
    assert_eq!(actual.cliffs, expected.cliffs);
    assert_eq!(actual.roads.len(), expected.roads.len());
    for (actual, expected) in actual.roads.iter().zip(&expected.roads) {
        assert_eq!(actual.pts, expected.pts);
        assert_eq!(actual.class, expected.class);
        assert_eq!(actual.is_bridge, expected.is_bridge);
    }
    assert_eq!(actual.water.len(), expected.water.len());
    for (actual, expected) in actual.water.iter().zip(&expected.water) {
        assert_eq!(actual.rings, expected.rings);
        assert_eq!(actual.is_ocean(), expected.is_ocean());
    }
    assert_eq!(actual.forest, expected.forest);
    assert_eq!(actual.ground, expected.ground);
    assert_eq!(actual.flora.len(), expected.flora.len());
    for (actual, expected) in actual.flora.iter().zip(&expected.flora) {
        assert_eq!(actual.rings, expected.rings);
        assert_eq!(actual.kind, expected.kind);
        assert_eq!(actual.seed, expected.seed);
    }
}

#[test]
fn semantic_night_building_adapter_matches_legacy_fixture_output() {
    let tile_key = (14, 8_192, 8_192);
    let tile = SemanticTile::normalize_at(fixture_tile(), tile_key);
    let mut semantic = FeaturePack::empty();
    let mut semantic_report = SemanticConsumerReport::default();
    append_semantic_buildings(
        &mut semantic,
        &tile,
        tile_key,
        (0.0, 0.0),
        Some(NIGHT_BUILDING_MIN_HEIGHT_M),
        &mut semantic_report,
    );
    let mut legacy = FeaturePack::empty();
    append_buildings(
        &mut legacy,
        tile.decoded(),
        tile_key,
        (0.0, 0.0),
        Some(NIGHT_BUILDING_MIN_HEIGHT_M),
    );
    assert_feature_pack_parity(&semantic, &legacy);
}
