//! Export the active source footprints for metric city-catalog authoring.
use serde_json::json;
use vearth_data::{ElevationSource, vectors::VectorSource};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let lat: f64 = args.get(1).ok_or("expected LAT LON OUTPUT.json")?.parse()?;
    let lon: f64 = args.get(2).ok_or("expected longitude")?.parse()?;
    let out = args.get(3).ok_or("expected output path")?;
    if !lat.is_finite() || !lon.is_finite() || lat.abs() > 85.0 || lon.abs() > 180.0 {
        return Err("survey anchor is outside the supported geographic range".into());
    }
    let option = |name: &str| -> Result<Option<&str>, Box<dyn std::error::Error>> {
        args.iter()
            .position(|arg| arg == name)
            .map(|index| {
                args.get(index + 1)
                    .map(String::as_str)
                    .ok_or_else(|| format!("{name} requires a value").into())
            })
            .transpose()
    };
    let query_half_extent_m = option("--extent")?
        .map(str::parse::<f64>)
        .transpose()?
        .unwrap_or(240.0);
    if !query_half_extent_m.is_finite() || !(1.0..=10_000.0).contains(&query_half_extent_m) {
        return Err("--extent must be in [1, 10000] metres".into());
    }
    let offsets = if let Some(path) = option("--ground-points")? {
        let offsets: Vec<[f64; 2]> = serde_json::from_slice(&std::fs::read(path)?)?;
        if offsets.len() > 16_641
            || offsets
                .iter()
                .flatten()
                .any(|v| !v.is_finite() || v.abs() > 10_000.0)
        {
            return Err(
                "ground points must contain at most 16641 finite EN offsets within 10 km".into(),
            );
        }
        offsets
    } else if let Some(value) = option("--ground-grid")? {
        let count: usize = value.parse()?;
        if !(2..=129).contains(&count) {
            return Err("--ground-grid expects 2 to 129 samples per axis".into());
        }
        (0..count)
            .flat_map(|north| {
                (0..count).map(move |east| {
                    [east, north]
                        .map(|i| query_half_extent_m * (2.0 * i as f64 / (count - 1) as f64 - 1.0))
                })
            })
            .collect()
    } else {
        Vec::new()
    };
    let landmark_name = option("--landmark-ground")?;
    let landmark = landmark_name
        .map(|name| {
            vearth_gen::landmarks::LANDMARKS
                .iter()
                .find(|landmark| landmark.name == name)
                .ok_or("unknown --landmark-ground name")
        })
        .transpose()?;
    let sources = vearth_app::sources::build_registry("cache", 9);
    if sources.dataset_tag.starts_with("terrain-") {
        return Err("source survey requires the production provider registry".into());
    }
    let ground = sources
        .registry
        .sample_elevation(lat.to_radians(), lon.to_radians(), 8.0);
    if !ground.complete {
        return Err("source elevation incomplete; surveys require VEARTH_ASYNC_PROVIDERS=0 and available tiles".into());
    }
    let ground_m = ground
        .value
        .filter(|h| h.is_finite())
        .ok_or("source elevation unavailable")?;
    let phi = lat.to_radians();
    let lambda = lon.to_radians();
    let origin = vearth_core::ellipsoid::geodetic_to_ecef(phi, lambda, 0.0);
    let east = glam::DVec3::new(-lambda.sin(), lambda.cos(), 0.0);
    let north = glam::DVec3::new(
        -phi.sin() * lambda.cos(),
        -phi.sin() * lambda.sin(),
        phi.cos(),
    );
    let active_landmark = if let Some(landmark) = landmark {
        use vearth_core::{
            cell::{CellId, ChunkAddr},
            frame::ChunkFrame,
        };
        let anchor = vearth_core::ellipsoid::geodetic_to_ecef(
            landmark.lat_deg.to_radians(),
            landmark.lon_deg.to_radians(),
            0.0,
        );
        let cell = CellId::from_dir(anchor, 19);
        let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
        let complete = std::cell::Cell::new(true);
        let active = vearth_gen::landmarks::active_for(cell, &frame, &|a, b| {
            let sample = sources.registry.sample_elevation(a, b, 8.0);
            let height = sample.value.filter(|h| h.is_finite());
            if !sample.complete || height.is_none() {
                complete.set(false);
            }
            height.unwrap_or(f32::NAN)
        })
        .into_iter()
        .find(|active| active.lm.name == landmark.name)
        .ok_or("landmark is inactive at its own anchor")?;
        if !complete.get() {
            return Err("landmark datum elevation incomplete".into());
        }
        Some(active)
    } else {
        None
    };
    let ground_samples: Vec<_> = offsets
        .iter()
        .map(|&[e, n]| {
            let (sample_lat, sample_lon, _) =
                vearth_core::ellipsoid::ecef_to_geodetic(origin + east * e + north * n);
            let sample = sources
                .registry
                .sample_elevation(sample_lat, sample_lon, 8.0);
            if !sample.complete {
                return Err("source grid elevation incomplete");
            }
            let height = sample
                .value
                .filter(|h| h.is_finite())
                .ok_or("source grid elevation unavailable")?;
            let mut record = json!({"east_m":e, "north_m":n,
                "lat":sample_lat.to_degrees(), "lon":sample_lon.to_degrees(), "ground_m":height});
            if let Some(active) = &active_landmark {
                let local = active.column_at(sample_lat, sample_lon, 0.0);
                let corrected = local
                    .and_then(|(x, z)| active.flattened_terrain(x, z, height))
                    .map_or(height, |(height, _)| height);
                record["corrected_ground_m"] = json!(corrected);
                record["model_xz_m"] = json!(local.map(|(x, z)| [x, z]));
                record["model_base_m"] = json!(local.map(|(x, z)| active.model_base_geo(x, z)));
            }
            Ok(record)
        })
        .collect::<Result<_, _>>()?;
    if args.iter().any(|arg| arg == "--ground-only") {
        let result = json!({
            "lat": lat, "lon": lon, "dataset": sources.dataset_tag,
            "ground_m": ground_m,
            "ground_samples": ground_samples,
            "landmark_ground": landmark_name,
            "ground_sample_pitch_m": 8.0,
        });
        std::fs::write(out, serde_json::to_vec_pretty(&result)?)?;
        return Ok(());
    }
    let pack = sources.registry.features(
        lat.to_radians(),
        lon.to_radians(),
        query_half_extent_m,
        query_half_extent_m,
    );
    if !pack.complete || pack.buildings.is_empty() {
        return Err("source footprint survey incomplete".into());
    }
    let buildings: Vec<_> = pack
        .buildings
        .iter()
        .map(|b| {
            json!({
                "rings": b.rings, "height": b.height_m, "min_height": b.min_height_m,
                "seed": b.seed.to_string(), "kind": format!("{:?}", b.kind),
                "location": b.location_rad.map(|p| p.map(f64::to_degrees)),
            })
        })
        .collect();
    let roads: Vec<_> = pack
        .roads
        .iter()
        .map(|r| {
            json!({
                "points":r.pts,"class":format!("{:?}",r.class),"is_bridge":r.is_bridge,
            })
        })
        .collect();
    let water: Vec<_> = pack
        .water
        .iter()
        .map(|w| {
            json!({
                "rings": w.rings, "kind": format!("{:?}", w.kind),
            })
        })
        .collect();
    let watercourses: Vec<_> = pack
        .watercourses
        .iter()
        .map(|w| {
            json!({
                "paths": w.paths, "width_m": w.width_m, "kind": format!("{:?}", w.kind),
            })
        })
        .collect();
    let result = json!({"lat":lat,"lon":lon,"dataset":sources.dataset_tag,
        "ground_m":ground_m,"ground_samples":ground_samples,"query_half_extent_m":query_half_extent_m,
        "landmark_ground":landmark_name,"ground_sample_pitch_m":8.0,
        "attribution":"© OpenStreetMap contributors (ODbL), via Protomaps", "buildings":buildings,"roads":roads,
        "water":water,"watercourses":watercourses});
    std::fs::write(out, serde_json::to_vec_pretty(&result)?)?;
    Ok(())
}
