//! Emit catalog identity and source specimens for the in-game finish gallery.
use serde_json::json;
use vearth_data::vectors::{Building, BuildingKind, BuildingUse};
use vearth_gen::architecture_resolver::{ArchitectureContext, resolve_building_design};
use vearth_gen::tall_buildings::TallBuildingShape;

fn main() {
    let entries: Vec<_> = vearth_gen::regional_finishes::reviewed_finishes().iter().map(|entry| {
        let location = (entry.latitude_deg.to_radians(), entry.longitude_deg.to_radians());
        let context = ArchitectureContext::at(location);
        let samples: Vec<_> = (0..36).map(|seed| {
            let x = 4.0 * (0.85 + (seed % 3) as f32 * 0.15);
            let y = 4.0 * (0.85 + ((seed / 3) % 3) as f32 * 0.25);
            let height = 5.0 + (seed % 4) as f32 * 2.5;
            let building = Building { location_rad: Some([location.0, location.1]),
                roof_shape: None, use_kind: BuildingUse::Unknown,
                rings: vec![vec![[-x, -y], [x, -y], [x, y], [-x, y]]],
                height_m: height, min_height_m: 0.0, seed, kind: BuildingKind::Building };
            let shape = TallBuildingShape { area_m2: 4.0*x*y, aspect: x.max(y)/x.min(y),
                compactness: std::f32::consts::PI*x*y/(x+y).powi(2), vertex_count: 4, fill_ratio: 1.0 };
            let result = resolve_building_design(&building, context, shape);
            let d = result.design;
            json!({"seed":seed,"height_m":height,"width_m":x*2.0,"depth_m":y*2.0,
                "source":format!("{:?}",result.source),"archetype":result.archetype,"style":d.style.key(),"expression":d.expression,
                "roof_shape":format!("{:?}",d.inferred_roof),"wall":d.wall,"roof":d.roof,
                "wall_material":d.wall_material,"wall_variant":d.wall_variant,
                "roof_material":d.roof_material,"roof_variant":d.roof_variant})
        }).collect();
        json!({"key":entry.key,"label":entry.label,"region":entry.world_region,
            "lat":entry.latitude_deg,"lon":entry.longitude_deg,
            "settlement":format!("{:?}",context.settlement),"samples":samples,
            "stage":format!("buildings:finish:{}",entry.key),
            "detail_stage":format!("buildings:finish-detail:{}",entry.key)})
    }).collect();
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({"gen_version":vearth_gen::GEN_VERSION,"entries":entries})
        )
        .unwrap()
    );
}
