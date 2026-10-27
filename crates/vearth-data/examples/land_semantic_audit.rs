//! Audit detailed landuse and regional landcover kinds in a cached immutable
//! Protomaps source revision. The report attributes every value to its source
//! layer and includes bounded tile examples for fixture selection.

use rusqlite::{Connection, params};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use vearth_data::mvt::GeomType;
use vearth_data::protomaps_semantics::{LayerFacts, SemanticTile};
use vearth_data::vectors::{MAX_REGIONAL_LANDCOVER_PROJECTED_BYTES, RegionalLandcoverRegion};

const EXAMPLE_LIMIT: usize = 4;

#[derive(Debug, Default)]
struct KindStats {
    features: usize,
    polygons: usize,
    non_polygons: usize,
    tiles: BTreeSet<(u8, u32, u32)>,
    sample: Option<(u8, u32, u32, f64, f64)>,
}

fn polygon_sample(
    key: (u8, u32, u32),
    extent: u32,
    paths: &[Vec<[i32; 2]>],
) -> Option<(u8, u32, u32, f64, f64)> {
    let ring = paths.first()?;
    if ring.is_empty() || extent == 0 {
        return None;
    }
    let (sum_x, sum_y) = ring.iter().fold((0.0, 0.0), |(x, y), point| {
        (x + f64::from(point[0]), y + f64::from(point[1]))
    });
    let local_x = sum_x / ring.len() as f64 / f64::from(extent);
    let local_y = sum_y / ring.len() as f64 / f64::from(extent);
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
    let mut stats: BTreeMap<(String, String, String), KindStats> = BTreeMap::new();
    let mut regional_projected_by_tile: BTreeMap<(u8, u32, u32), usize> = BTreeMap::new();
    let mut regional_points = 0usize;
    let mut regional_features = 0usize;
    while let Some(row) = rows.next()? {
        let key = (
            row.get::<_, u8>(0)?,
            row.get::<_, u32>(1)?,
            row.get::<_, u32>(2)?,
        );
        let payload = row.get::<_, Vec<u8>>(3)?;
        let tile = SemanticTile::normalize_at(vearth_data::mvt::decode_tile(&payload)?, key);
        source_tiles += 1;
        for layer_name in ["landuse", "landcover"] {
            let Some(layer) = tile.semantic_layer(layer_name) else {
                continue;
            };
            for feature in &layer.features {
                let (kind, class) = match &feature.facts {
                    LayerFacts::Landuse(facts) => (facts.kind, format!("{:?}", facts.class)),
                    LayerFacts::Landcover(facts) => (facts.kind, format!("{:?}", facts.class)),
                    _ => (None, "None".into()),
                };
                let kind = kind
                    .and_then(|fact| tile.source_text(layer, fact))
                    .unwrap_or("<missing-or-invalid>");
                let sample = tile
                    .source_feature(layer, feature)
                    .and_then(|source| polygon_sample(key, layer.extent, &source.paths));
                if layer_name == "landcover"
                    && feature.geometry_type == GeomType::Polygon
                    && let LayerFacts::Landcover(facts) = &feature.facts
                    && facts.class.is_some_and(|class| {
                        class != vearth_data::protomaps_semantics::LandcoverKind::Unknown
                    })
                    && let Some(source) = tile.source_feature(layer, feature)
                {
                    let points = source.paths.iter().map(Vec::len).sum::<usize>();
                    let projected = std::mem::size_of::<RegionalLandcoverRegion>()
                        + source.paths.len() * std::mem::size_of::<Vec<[f32; 2]>>()
                        + points * std::mem::size_of::<[f32; 2]>();
                    *regional_projected_by_tile.entry(key).or_default() += projected;
                    regional_points += points;
                    regional_features += 1;
                }
                let entry = stats
                    .entry((layer_name.to_string(), kind.to_string(), class))
                    .or_default();
                entry.features += 1;
                if feature.geometry_type == GeomType::Polygon {
                    entry.polygons += 1;
                } else {
                    entry.non_polygons += 1;
                }
                entry.tiles.insert(key);
                if entry.sample.is_none() {
                    entry.sample = sample;
                }
            }
        }
    }
    if source_tiles == 0 {
        return Err("no present vector tiles matched the requested revision".into());
    }

    println!("revision={revision} source_tiles={source_tiles}");
    println!("layer\tkind\tclass\tfeatures\tpolygons\tnon_polygons\ttiles\texamples\tsample");
    for ((layer, kind, class), counts) in stats {
        let examples = counts
            .tiles
            .iter()
            .take(EXAMPLE_LIMIT)
            .map(|(z, x, y)| format!("{z}/{x}/{y}"))
            .collect::<Vec<_>>()
            .join(",");
        let sample = counts.sample.map_or_else(
            || "-".to_string(),
            |(z, x, y, lat, lon)| format!("{z}/{x}/{y}@{lat:.6},{lon:.6}"),
        );
        println!(
            "{layer}\t{kind}\t{class}\t{}\t{}\t{}\t{}\t{examples}\t{sample}",
            counts.features,
            counts.polygons,
            counts.non_polygons,
            counts.tiles.len(),
        );
    }
    let projected_total = regional_projected_by_tile.values().sum::<usize>();
    let projected_max = regional_projected_by_tile
        .values()
        .copied()
        .max()
        .unwrap_or(0);
    let projected_average = projected_total
        .checked_div(regional_projected_by_tile.len())
        .unwrap_or(0);
    println!(
        "regional_projected tiles={} features={} points={} total_bytes={} avg_tile_bytes={} max_tile_bytes={} feature_pack_budget_bytes={}",
        regional_projected_by_tile.len(),
        regional_features,
        regional_points,
        projected_total,
        projected_average,
        projected_max,
        MAX_REGIONAL_LANDCOVER_PROJECTED_BYTES,
    );
    Ok(())
}
