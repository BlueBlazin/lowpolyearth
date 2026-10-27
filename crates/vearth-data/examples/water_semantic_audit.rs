//! Audit water geometry, kinds/details, and physical flag encodings in a
//! cached immutable Protomaps source revision.

use rusqlite::{Connection, params};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use vearth_data::mvt::{GeomType, Value};
use vearth_data::protomaps_semantics::{
    LayerFacts, SemanticTile, SourceFact, WaterSurfaceDisposition,
};
use vearth_data::vectors::{WaterRegionKind, Watercourse, WatercourseKind};

const EXAMPLE_LIMIT: usize = 4;

#[derive(Debug, Default)]
struct Stats {
    features: usize,
    tiles: BTreeSet<(u8, u32, u32)>,
    sample: Option<(u8, u32, u32, f64, f64)>,
}

#[derive(Debug, Default)]
struct LineGeometryStats {
    records: usize,
    paths: usize,
    points: usize,
}

impl LineGeometryStats {
    fn projected_bytes(&self) -> usize {
        self.records * std::mem::size_of::<Watercourse>()
            + self.paths * std::mem::size_of::<Vec<[f32; 2]>>()
            + self.points * std::mem::size_of::<[f32; 2]>()
    }
}

fn feature_sample(
    key: (u8, u32, u32),
    extent: u32,
    paths: &[Vec<[i32; 2]>],
) -> Option<(u8, u32, u32, f64, f64)> {
    let points = paths.iter().flatten().collect::<Vec<_>>();
    if points.is_empty() || extent == 0 {
        return None;
    }
    let (sum_x, sum_y) = points.iter().fold((0.0, 0.0), |(x, y), point| {
        (x + f64::from(point[0]), y + f64::from(point[1]))
    });
    let local_x = sum_x / points.len() as f64 / f64::from(extent);
    let local_y = sum_y / points.len() as f64 / f64::from(extent);
    let scale = 2.0_f64.powi(i32::from(key.0));
    let world_x = (f64::from(key.1) + local_x) / scale;
    let world_y = (f64::from(key.2) + local_y) / scale;
    let lon = world_x * 360.0 - 180.0;
    let lat = (std::f64::consts::PI * (1.0 - 2.0 * world_y))
        .sinh()
        .atan()
        .to_degrees();
    Some((key.0, key.1, key.2, lat, lon))
}

fn geometry_label(geometry: GeomType) -> &'static str {
    match geometry {
        GeomType::Point => "point",
        GeomType::Line => "line",
        GeomType::Polygon => "polygon",
    }
}

fn value_label(value: Option<&Value>) -> String {
    match value {
        None => "missing".into(),
        Some(Value::Str(value)) => format!("string:{value}"),
        Some(Value::Bool(value)) => format!("bool:{value}"),
        Some(Value::I64(value)) => format!("i64:{value}"),
        Some(Value::F64(value)) => format!("f64:{value}"),
    }
}

fn fact_value<'a>(
    tile: &'a SemanticTile,
    layer: &'a vearth_data::protomaps_semantics::SemanticLayer,
    fact: Option<SourceFact>,
) -> Option<&'a Value> {
    fact.and_then(|fact| tile.source_value(layer, fact.source))
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
    let mut source_tiles = 0usize;
    let mut water_tiles = BTreeSet::new();
    let mut rows_by_kind: BTreeMap<(String, String, &'static str), Stats> = BTreeMap::new();
    let mut flags: BTreeMap<(&'static str, String), usize> = BTreeMap::new();
    let mut geometry_totals: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut typed_totals: BTreeMap<(&'static str, &'static str, &'static str), usize> =
        BTreeMap::new();
    let mut polygon_roles: BTreeMap<WaterRegionKind, usize> = BTreeMap::new();
    let mut source_ids = 0usize;
    let mut missing_ids = 0usize;
    let mut line_source_ids: BTreeMap<u64, BTreeSet<(u8, u32, u32)>> = BTreeMap::new();
    let mut line_source_zooms: BTreeMap<u64, BTreeSet<u8>> = BTreeMap::new();
    let mut line_id_tiles_by_zoom: BTreeMap<(u64, u8), BTreeSet<(u32, u32)>> = BTreeMap::new();
    let mut polygon_source_ids = BTreeSet::new();
    let mut line_missing_ids = 0usize;
    let mut line_paths = 0usize;
    let mut line_points = 0usize;
    let mut line_edge_endpoints = 0usize;
    let mut line_outside_or_edge_endpoints = 0usize;
    let mut line_near_edge_endpoints = 0usize;
    let mut line_explicit_widths: BTreeMap<String, usize> = BTreeMap::new();
    let mut accepted_line_geometry: BTreeMap<(u8, u32, u32), LineGeometryStats> = BTreeMap::new();
    while let Some(row) = rows.next()? {
        let key = (
            row.get::<_, u8>(0)?,
            row.get::<_, u32>(1)?,
            row.get::<_, u32>(2)?,
        );
        let payload = row.get::<_, Vec<u8>>(3)?;
        let tile = SemanticTile::normalize_at(vearth_data::mvt::decode_tile(&payload)?, key);
        source_tiles += 1;
        let Some(layer) = tile.semantic_layer("water") else {
            continue;
        };
        water_tiles.insert(key);
        for feature in &layer.features {
            let LayerFacts::Water(facts) = &feature.facts else {
                continue;
            };
            *typed_totals
                .entry((
                    facts
                        .class
                        .map_or("invalid-or-missing", |kind| kind.label()),
                    facts.geometry_role.label(),
                    facts.surface_disposition.label(),
                ))
                .or_default() += 1;
            if feature.geometry_type == GeomType::Polygon {
                if let Some(id) = feature.source_id {
                    polygon_source_ids.insert(id);
                }
                let reservoir = facts
                    .physical
                    .reservoir
                    .and_then(|fact| tile.source_boolean(layer, fact))
                    .unwrap_or(false);
                let region = WaterRegionKind::from_semantic(facts, reservoir);
                *polygon_roles.entry(region).or_default() += 1;
            }
            if feature.source_id.is_some() {
                source_ids += 1;
            } else {
                missing_ids += 1;
            }
            *geometry_totals
                .entry(geometry_label(feature.geometry_type))
                .or_default() += 1;
            let kind = facts
                .physical
                .kind
                .and_then(|fact| tile.source_text(layer, fact))
                .unwrap_or("<missing-or-invalid>");
            let detail = facts
                .physical
                .kind_detail
                .and_then(|fact| tile.source_text(layer, fact))
                .unwrap_or("<none-or-invalid>");
            let source = tile.source_feature(layer, feature);
            if feature.geometry_type == GeomType::Line {
                match feature.source_id {
                    Some(id) => {
                        line_source_ids.entry(id).or_default().insert(key);
                        line_source_zooms.entry(id).or_default().insert(key.0);
                        line_id_tiles_by_zoom
                            .entry((id, key.0))
                            .or_default()
                            .insert((key.1, key.2));
                    }
                    None => line_missing_ids += 1,
                }
                if let Some(source) = source {
                    line_paths += source.paths.len();
                    line_points += source.paths.iter().map(Vec::len).sum::<usize>();
                    let extent = i32::try_from(layer.extent).unwrap_or(i32::MAX);
                    line_edge_endpoints += source
                        .paths
                        .iter()
                        .flat_map(|path| [path.first(), path.last()])
                        .flatten()
                        .filter(|point| {
                            point[0] == 0
                                || point[1] == 0
                                || point[0] == extent
                                || point[1] == extent
                        })
                        .count();
                    line_outside_or_edge_endpoints += source
                        .paths
                        .iter()
                        .flat_map(|path| [path.first(), path.last()])
                        .flatten()
                        .filter(|point| {
                            point[0] <= 0
                                || point[1] <= 0
                                || point[0] >= extent
                                || point[1] >= extent
                        })
                        .count();
                    let edge_band = 256;
                    line_near_edge_endpoints += source
                        .paths
                        .iter()
                        .flat_map(|path| [path.first(), path.last()])
                        .flatten()
                        .filter(|point| {
                            point[0] <= edge_band
                                || point[1] <= edge_band
                                || point[0] >= extent - edge_band
                                || point[1] >= extent - edge_band
                        })
                        .count();
                    if let Some(width) = tile
                        .source_layer(layer)
                        .and_then(|source_layer| source.get(source_layer, "width"))
                    {
                        *line_explicit_widths
                            .entry(value_label(Some(width)))
                            .or_default() += 1;
                    }
                    if facts.surface_disposition == WaterSurfaceDisposition::Candidate
                        && WatercourseKind::from_semantic(facts).is_some()
                    {
                        let stats = accepted_line_geometry.entry(key).or_default();
                        stats.records += 1;
                        stats.paths += source.paths.len();
                        stats.points += source.paths.iter().map(Vec::len).sum::<usize>();
                    }
                }
            }
            let entry = rows_by_kind
                .entry((
                    kind.to_string(),
                    detail.to_string(),
                    geometry_label(feature.geometry_type),
                ))
                .or_default();
            entry.features += 1;
            entry.tiles.insert(key);
            if entry.sample.is_none() {
                entry.sample =
                    source.and_then(|source| feature_sample(key, layer.extent, &source.paths));
            }
            for (field, fact) in [
                ("bridge", facts.physical.bridge),
                ("tunnel", facts.physical.tunnel),
                ("layer", facts.physical.layer),
                ("intermittent", facts.physical.intermittent),
                ("reservoir", facts.physical.reservoir),
                ("alkaline", facts.physical.alkaline),
            ] {
                let value = fact_value(&tile, layer, fact);
                if value.is_some() {
                    *flags.entry((field, value_label(value))).or_default() += 1;
                }
            }
        }
    }
    println!(
        "revision={revision} source_tiles={source_tiles} water_tiles={} ids={} missing_ids={}",
        water_tiles.len(),
        source_ids,
        missing_ids
    );
    println!(
        "geometry point={} line={} polygon={} current_consumed_polygons={} current_nonpolygon_without_physical_output={}",
        geometry_totals.get("point").copied().unwrap_or(0),
        geometry_totals.get("line").copied().unwrap_or(0),
        geometry_totals.get("polygon").copied().unwrap_or(0),
        geometry_totals.get("polygon").copied().unwrap_or(0),
        geometry_totals.get("point").copied().unwrap_or(0)
            + geometry_totals.get("line").copied().unwrap_or(0),
    );
    println!(
        "line_topology unique_provided_ids={} missing_ids={} ids_spanning_tiles={} same_zoom_multitile_groups={} ids_spanning_zooms={} line_polygon_companion_ids={} paths={} points={} exact_edge_endpoints={} outside_or_edge_endpoints={} near_edge_endpoints={} explicit_width_features={}",
        line_source_ids.len(),
        line_missing_ids,
        line_source_ids
            .values()
            .filter(|tiles| tiles.len() > 1)
            .count(),
        line_id_tiles_by_zoom
            .values()
            .filter(|tiles| tiles.len() > 1)
            .count(),
        line_source_zooms
            .values()
            .filter(|zooms| zooms.len() > 1)
            .count(),
        line_source_ids
            .keys()
            .filter(|id| polygon_source_ids.contains(id))
            .count(),
        line_paths,
        line_points,
        line_edge_endpoints,
        line_outside_or_edge_endpoints,
        line_near_edge_endpoints,
        line_explicit_widths.values().sum::<usize>(),
    );
    for (width, count) in line_explicit_widths {
        println!("line_width\t{width}\t{count}");
    }
    let accepted_records = accepted_line_geometry
        .values()
        .map(|stats| stats.records)
        .sum::<usize>();
    let accepted_points = accepted_line_geometry
        .values()
        .map(|stats| stats.points)
        .sum::<usize>();
    let max_tile_bytes = accepted_line_geometry
        .values()
        .map(LineGeometryStats::projected_bytes)
        .max()
        .unwrap_or(0);
    println!(
        "accepted_line_geometry records={accepted_records} points={accepted_points} max_single_tile_projected_bytes={max_tile_bytes} nine_tile_upper_bound_bytes={}",
        max_tile_bytes * 9,
    );
    println!("typed_kind\tgeometry_role\tsurface_disposition\tfeatures");
    for ((kind, role, disposition), count) in typed_totals {
        println!("{kind}\t{role}\t{disposition}\t{count}");
    }
    println!("polygon_role\tfeatures\ttyped_surface_water\tecological_proximity\tgameplay");
    for (kind, count) in polygon_roles {
        println!(
            "{}\t{count}\t{}\t{}\t{:?}",
            kind.label(),
            kind.creates_surface_water(),
            kind.contributes_ecological_proximity(),
            kind.gameplay_role()
        );
    }
    println!("kind\tdetail\tgeometry\tfeatures\ttiles\texamples\tsample");
    for ((kind, detail, geometry), stats) in rows_by_kind {
        let examples = stats
            .tiles
            .iter()
            .take(EXAMPLE_LIMIT)
            .map(|(z, x, y)| format!("{z}/{x}/{y}"))
            .collect::<Vec<_>>()
            .join(",");
        let sample = stats.sample.map_or_else(
            || "-".to_string(),
            |(z, x, y, lat, lon)| format!("{z}/{x}/{y}@{lat:.6},{lon:.6}"),
        );
        println!(
            "{kind}\t{detail}\t{geometry}\t{}\t{}\t{examples}\t{sample}",
            stats.features,
            stats.tiles.len(),
        );
    }
    println!("flags");
    for ((field, value), count) in flags {
        println!("{field}\t{value}\t{count}");
    }
    Ok(())
}
