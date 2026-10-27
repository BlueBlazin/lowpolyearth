//! Warm-cache CPU benchmark of source-to-design resolution, including geography.
//! Run the same binary profile and sample count on both revisions, without other workloads.
use std::hint::black_box;
use std::time::Instant;
use vearth_data::vectors::{Building, BuildingKind, BuildingUse};
use vearth_gen::regional_architecture::building_design;
use vearth_gen::tall_buildings::TallBuildingShape;

fn main() {
    let samples: u64 = std::env::args()
        .nth(1)
        .map_or(20_000, |s| s.parse().expect("positive sample count"));
    assert!(samples > 0);
    eprintln!("GEN_VERSION={}, samples={samples}", vearth_gen::GEN_VERSION);
    for (name, lat, lon) in [
        ("Sydney", -33.8677_f64, 151.2110_f64),
        ("Manhattan", 40.7545, -73.9800),
        ("Pimlico", 51.4887, -0.1395),
        ("Copenhagen", 55.6761, 12.5683),
        ("Helsinki", 60.1700, 24.9400),
        ("Canary Wharf", 51.505, -0.020),
    ] {
        for (use_kind, height, area) in [
            (BuildingUse::Unknown, 8.0, 64.0),
            (BuildingUse::Unknown, 120.0, 1600.0),
            (BuildingUse::Commercial, 120.0, 1600.0),
            (BuildingUse::Industrial, 12.0, 1600.0),
        ] {
            let mut building = Building {
                location_rad: Some([lat.to_radians(), lon.to_radians()]),
                rings: vec![],
                height_m: height,
                min_height_m: 0.0,
                seed: 0,
                kind: BuildingKind::Building,
                roof_shape: None,
                use_kind,
            };
            let shape = TallBuildingShape {
                area_m2: area,
                aspect: 1.0,
                compactness: 0.78,
                vertex_count: 4,
                fill_ratio: 1.0,
            };
            // Warm lazy atlases before measuring. Jitter samples across a local
            // grid so a future exact-anchor cache cannot turn this into one hit.
            black_box(building_design(&building, (0.0, 0.0), shape));
            let mut times = [0.0; 5];
            for time in &mut times {
                let start = Instant::now();
                for seed in 0..samples {
                    building.seed = seed;
                    building.location_rad = Some([
                        (lat + (seed % 31) as f64 * 0.00001).to_radians(),
                        (lon + (seed % 29) as f64 * 0.00001).to_radians(),
                    ]);
                    black_box(building_design(black_box(&building), (0.0, 0.0), shape));
                }
                *time = start.elapsed().as_nanos() as f64 / samples as f64;
            }
            times.sort_by(f64::total_cmp);
            println!("{name},{use_kind:?},{height},{:.0}", times[2]);
        }
    }
}
