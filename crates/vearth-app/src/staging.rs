//! Lightweight, world-independent visual staging workspace.
//!
//! The stage uploads only a neutral grid and the selected catalogue asset.
//! Regional building models are independent design studies; world buildings
//! are generated from source footprints through the production facade system.
//! It does not construct providers, start generation workers, stream world
//! cells, or touch the chunk cache. Additions to the catalog are ordinary
//! `StageSource` builders, so every catalog mesh shares the same inspection
//! camera and screenshot path.

use crate::gfx::{Gfx, PlayerDraw, Renderer, create_depth};
use crate::platform_paths::WINDOW_TITLE;
use crate::solar::{SolarClock, SolarStart};
use crate::weather::{WeatherConfig, WeatherMode, WeatherModel};
use glam::{DMat4, DVec3};
use std::collections::HashSet;
use std::sync::Arc;
use vearth_core::mesh::{MeshBuffers, SurfaceVertex, encode_surface_normal};
use vearth_data::vectors::RoofShape;
use vearth_gen::SurfaceAppearance;
use vearth_gen::architecture::{
    ARCHITECTURE_PROFILES, ArchitectureProfile, ArchitectureRegion, BuildingGrammar,
};
use vearth_gen::tall_buildings::{
    TALL_BUILDING_PROFILES, TallBuildingProfile, TallBuildingStyle, TallMassing,
    tall_building_profile,
};
use vearth_gen::vegetation::{FoliageSeasonClass, TREE_PROFILES, TreeProfileId};
use vearth_platform::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

// Equinox noon over the stage’s equatorial Greenwich origin.
const STAGE_UTC_SECONDS: f64 = 1_774_008_000.0;
const DEFAULT_ASSET: &str = "tree:banyan";
const DEFAULT_FOV_Y: f64 = 48.0_f64.to_radians();

#[derive(Clone, Debug)]
pub struct StageOptions {
    pub asset: String,
    pub instances: usize,
    pub azimuth_deg: f64,
    pub elevation_deg: f64,
    pub distance_m: Option<f64>,
    pub sun_hour_utc: f64,
}

impl Default for StageOptions {
    fn default() -> Self {
        Self {
            asset: DEFAULT_ASSET.into(),
            instances: 1,
            azimuth_deg: 35.0,
            elevation_deg: 24.0,
            distance_m: None,
            sun_hour_utc: 12.0,
        }
    }
}

impl StageOptions {
    fn solar_utc_seconds(&self) -> f64 {
        STAGE_UTC_SECONDS + (self.sun_hour_utc - 12.0) * 3600.0
    }
}

/// Parse the native staging flags. `--stage` optionally accepts an asset id;
/// `--stage-asset` is the unambiguous form used by scripts.
pub fn parse_options(argv: &[String]) -> Result<Option<StageOptions>, String> {
    let Some(stage_index) = argv.iter().position(|arg| arg == "--stage") else {
        return Ok(None);
    };
    let mut options = StageOptions::default();
    if let Some(value) = argv
        .get(stage_index + 1)
        .filter(|value| !value.starts_with('-'))
    {
        options.asset = value.clone();
    }
    let value_after = |flag: &str| {
        argv.iter()
            .position(|arg| arg == flag)
            .and_then(|index| argv.get(index + 1))
    };
    if let Some(asset) = value_after("--stage-asset") {
        options.asset = asset.clone();
    }
    if let Some(value) = value_after("--stage-instances") {
        options.instances = value
            .parse::<usize>()
            .map_err(|_| "--stage-instances expects an integer from 1 to 25".to_string())?;
        if !(1..=25).contains(&options.instances) {
            return Err("--stage-instances expects an integer from 1 to 25".into());
        }
    }
    if let Some(value) = value_after("--stage-azimuth") {
        options.azimuth_deg = value
            .parse::<f64>()
            .map_err(|_| "--stage-azimuth expects degrees".to_string())?;
    }
    if let Some(value) = value_after("--stage-elevation") {
        options.elevation_deg = value
            .parse::<f64>()
            .map_err(|_| "--stage-elevation expects degrees".to_string())?;
    }
    if let Some(value) = value_after("--stage-distance") {
        let distance = value
            .parse::<f64>()
            .map_err(|_| "--stage-distance expects a positive number of metres".to_string())?;
        if !distance.is_finite() || distance <= 0.0 {
            return Err("--stage-distance expects a positive number of metres".into());
        }
        options.distance_m = Some(distance);
    }
    if argv.iter().any(|arg| arg == "--stage-sun-hour") {
        let value = value_after("--stage-sun-hour")
            .ok_or("--stage-sun-hour expects a UTC hour from 0 up to 24")?;
        options.sun_hour_utc = value
            .parse::<f64>()
            .map_err(|_| "--stage-sun-hour expects a UTC hour from 0 up to 24")?;
        if !options.sun_hour_utc.is_finite() || !(0.0..24.0).contains(&options.sun_hour_utc) {
            return Err("--stage-sun-hour expects a UTC hour from 0 up to 24".into());
        }
    }
    if !options.azimuth_deg.is_finite() || !options.elevation_deg.is_finite() {
        return Err("stage camera angles must be finite".into());
    }

    let catalog = stage_catalog();
    let descriptor = resolve_descriptor(&catalog, &options.asset).ok_or_else(|| {
        format!(
            "unknown stage asset {:?}; use buildings:<view>, drone[:<concept>], vehicle:<name>, tree:<name>, or landmark:<name>",
            options.asset
        )
    })?;
    options.asset = descriptor.id.clone();
    Ok(Some(options))
}

#[derive(Clone, Copy)]
enum StageSource {
    ConstructionFinish {
        material: u8,
        tint: [u8; 3],
        variant: u8,
    },
    RoofPigments,
    RoadMaterials,
    RoadPlaces,
    RoadFinish {
        material: u8,
        tint: [u8; 3],
    },
    BuildingCatalogue(BuildingScale),
    RegionalBuilding(ArchitectureRegion, BuildingScale),
    PlaceBuildings(&'static vearth_gen::architecture_places::ArchitecturalPlace),
    ReviewedBuildings {
        lat: f64,
        lon: f64,
        count: u64,
    },
    ProductionHighrise(vearth_data::vectors::BuildingUse),
    TallBuildingCatalogue,
    TallBuilding(TallBuildingStyle),
    ObservationTower,
    Drone,
    Hoverbike,
    SplitArrow,
    PhaseManta,
    Tree(TreeProfileId),
    Landmark(&'static vearth_gen::landmarks::Landmark),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BuildingScale {
    ShortHouse,
    MediumBuilding,
}

#[derive(Clone)]
struct StageDescriptor {
    id: String,
    label: String,
    category: &'static str,
    source: StageSource,
}

fn stage_catalog() -> Vec<StageDescriptor> {
    let mut catalog = vec![
        StageDescriptor {
            id: "roads:materials".into(),
            label: "Road finishes: asphalt, concrete, brick, setts, gravel, earth, cobbles, slabs"
                .into(),
            category: "Roads",
            source: StageSource::RoadMaterials,
        },
        StageDescriptor {
            id: "roads:places".into(),
            label: "Documented local paving treatments".into(),
            category: "Roads",
            source: StageSource::RoadPlaces,
        },
        StageDescriptor {
            id: "buildings:all-houses".into(),
            label: "All regional short houses".into(),
            category: "Regional architecture",
            source: StageSource::BuildingCatalogue(BuildingScale::ShortHouse),
        },
        StageDescriptor {
            id: "buildings:all-medium".into(),
            label: "All regional medium buildings".into(),
            category: "Regional architecture",
            source: StageSource::BuildingCatalogue(BuildingScale::MediumBuilding),
        },
        StageDescriptor {
            id: "buildings:all-tall".into(),
            label: "Legacy tall-building shape studies".into(),
            category: "Legacy tall architecture",
            source: StageSource::TallBuildingCatalogue,
        },
        StageDescriptor {
            id: "buildings:observation-tower".into(),
            label: "Generic mapped observation tower".into(),
            category: "Tall architecture",
            source: StageSource::ObservationTower,
        },
        StageDescriptor {
            id: "drone".into(),
            label: "Kestrel quadrotor".into(),
            category: "Vehicle",
            source: StageSource::Drone,
        },
        StageDescriptor {
            id: "vehicle:sparrow-hoverbike".into(),
            label: "Sparrow hoverbike".into(),
            category: "Vehicle",
            source: StageSource::Hoverbike,
        },
        StageDescriptor {
            id: "drone:split-arrow".into(),
            label: "Split Arrow phase drone".into(),
            category: "Vehicle",
            source: StageSource::SplitArrow,
        },
        StageDescriptor {
            id: "drone:phase-manta".into(),
            label: "Phase Manta phase drone".into(),
            category: "Vehicle",
            source: StageSource::PhaseManta,
        },
    ];
    for (slug, label, material, tint) in road_finish_studies() {
        for before in [false, true] {
            catalog.push(StageDescriptor {
                id: format!("roads:{slug}{}", if before { ":before" } else { "" }),
                label: if before {
                    format!("{label} · generic path assignment (controlled baseline)")
                } else {
                    format!("{label} · material close-up")
                },
                category: "Road close-ups",
                source: StageSource::RoadFinish {
                    material: if before {
                        vearth_gen::material::PATH
                    } else {
                        material
                    },
                    tint: if before {
                        vearth_gen::palette::infrastructure::PATH.map(|v| v.round() as u8)
                    } else {
                        tint
                    },
                },
            });
        }
    }
    for (index, sample) in vearth_gen::road_materials::local_samples()
        .into_iter()
        .enumerate()
    {
        catalog.push(StageDescriptor {
            id: format!("roads:place:{index}"),
            label: format!("{} · {}", sample.label, sample.region),
            category: "Local paving",
            source: StageSource::RoadFinish {
                material: sample.appearance.material,
                tint: sample.appearance.tint,
            },
        });
    }
    let mut place_styles = HashSet::new();
    for finish in vearth_gen::regional_finishes::reviewed_finishes() {
        catalog.push(StageDescriptor {
            id: format!("buildings:finish:{}", finish.key),
            label: format!("{} · ordinary building finishes", finish.label),
            category: "Regional finish studies",
            source: StageSource::ReviewedBuildings {
                lat: finish.latitude_deg.to_radians(),
                lon: finish.longitude_deg.to_radians(),
                count: 36,
            },
        });
        catalog.push(StageDescriptor {
            id: format!("buildings:finish-detail:{}", finish.key),
            label: format!("{} · four close-up specimens", finish.label),
            category: "Regional finish details",
            source: StageSource::ReviewedBuildings {
                lat: finish.latitude_deg.to_radians(),
                lon: finish.longitude_deg.to_radians(),
                count: 4,
            },
        });
    }
    catalog.push(StageDescriptor {
        id: "materials:roof-pigments".into(),
        label: "Roof pigment comparison · taupe, neutral and cool greys".into(),
        category: "Construction materials",
        source: StageSource::RoofPigments,
    });
    for (slug, label, material, tint) in construction_finish_studies() {
        catalog.push(StageDescriptor {
            id: format!("materials:{slug}"),
            label: format!("{label} · four-metre finish study"),
            category: "Construction materials",
            source: StageSource::ConstructionFinish {
                material,
                tint,
                variant: 0,
            },
        });
    }
    for (slug, label, material, tint) in construction_variant_studies() {
        catalog.push(StageDescriptor {
            id: format!("materials:{slug}"),
            label: format!("{label} · four-metre finish study"),
            category: "Construction materials",
            source: StageSource::ConstructionFinish {
                material,
                tint,
                variant: 1,
            },
        });
    }
    for (slug, material) in [
        ("matte-corrugated", vearth_gen::material::METAL_ROOF),
        ("matte-corrugated-wall", vearth_gen::material::PANEL_WALL),
    ] {
        catalog.push(StageDescriptor {
            id: format!("materials:{slug}"),
            label: "Matte corrugated sheet · four-metre finish study".into(),
            category: "Construction materials",
            source: StageSource::ConstructionFinish {
                material,
                tint: [117, 139, 143],
                variant: 2,
            },
        });
    }
    for (slug, label, tint, variant) in [
        (
            "cement-sheet",
            "Corrugated mineral roof sheet",
            [130, 132, 125],
            1,
        ),
        ("turf", "Living turf roof", [98, 120, 59], 2),
        ("bitumen-roof", "Bitumen roof covering", [62, 65, 61], 3),
    ] {
        catalog.push(StageDescriptor {
            id: format!("materials:{slug}"),
            label: format!("{label} · four-metre finish study"),
            category: "Construction materials",
            source: StageSource::ConstructionFinish {
                material: vearth_gen::material::ROOF,
                tint,
                variant,
            },
        });
    }
    for (slug, label, tint, variant) in [
        ("azulejo", "Glazed ceramic wall tiles", [41, 135, 148], 1),
        (
            "azulejo-pattern",
            "Patterned glazed ceramic wall tiles",
            [32, 78, 149],
            2,
        ),
    ] {
        catalog.push(StageDescriptor {
            id: format!("materials:{slug}"),
            label: format!("{label} · four-metre finish study"),
            category: "Construction materials",
            source: StageSource::ConstructionFinish {
                material: vearth_gen::material::NAMAKO_WALL,
                tint,
                variant,
            },
        });
    }
    catalog.push(StageDescriptor {
        id: "materials:woven-panel".into(),
        label: "Woven organic wall panel · four-metre finish study".into(),
        category: "Construction materials",
        source: StageSource::ConstructionFinish {
            material: vearth_gen::material::TIMBER_WALL,
            tint: [190, 184, 150],
            variant: 2,
        },
    });
    catalog.push(StageDescriptor {
        id: "materials:tile-metal".into(),
        label: "Tile-stamped coated metal · four-metre finish study".into(),
        category: "Construction materials",
        source: StageSource::ConstructionFinish {
            material: vearth_gen::material::METAL_ROOF,
            tint: [128, 65, 52],
            variant: 3,
        },
    });
    catalog.extend(
        vearth_gen::architecture_places::places()
            .iter()
            .filter_map(|place| {
                place_styles
                    .insert(place.style_key)
                    .then(|| StageDescriptor {
                        id: format!("buildings:place:{}", place.style_key),
                        label: format!("{} · generated buildings", place.source_name),
                        category: "Local architecture",
                        source: StageSource::PlaceBuildings(place),
                    })
            }),
    );
    catalog.extend(ARCHITECTURE_PROFILES.iter().flat_map(|profile| {
        [
            StageDescriptor {
                id: format!("buildings:house:{}", profile.slug),
                label: format!("{} · short house", profile.label),
                category: "Regional house",
                source: StageSource::RegionalBuilding(profile.id, BuildingScale::ShortHouse),
            },
            StageDescriptor {
                id: format!("buildings:medium:{}", profile.slug),
                label: format!("{} · medium building", profile.label),
                category: "Regional medium building",
                source: StageSource::RegionalBuilding(profile.id, BuildingScale::MediumBuilding),
            },
        ]
    }));
    catalog.extend(
        TALL_BUILDING_PROFILES
            .iter()
            .map(|profile| StageDescriptor {
                id: format!("buildings:tall:{}", profile.slug),
                label: format!("Legacy study · {} · {}", profile.label, profile.period),
                category: "Legacy tall building",
                source: StageSource::TallBuilding(profile.id),
            }),
    );
    for (key, label, usage) in [
        (
            "office",
            "Production office towers",
            vearth_data::vectors::BuildingUse::Commercial,
        ),
        (
            "residential",
            "Production residential towers",
            vearth_data::vectors::BuildingUse::Residential,
        ),
    ] {
        catalog.push(StageDescriptor {
            id: format!("buildings:highrise:{key}"),
            label: label.into(),
            category: "Production tall architecture",
            source: StageSource::ProductionHighrise(usage),
        });
    }
    catalog.extend(TREE_PROFILES.iter().map(|profile| StageDescriptor {
        id: format!("tree:{}", profile.name),
        label: title_case(profile.name),
        category: "Tree",
        source: StageSource::Tree(profile.id),
    }));
    catalog.extend(
        vearth_gen::landmarks::LANDMARKS
            .iter()
            .map(|landmark| StageDescriptor {
                id: format!("landmark:{}", landmark.name),
                label: title_case(landmark.name),
                category: "Landmark",
                source: StageSource::Landmark(landmark),
            }),
    );
    catalog
}

fn resolve_descriptor<'a>(
    catalog: &'a [StageDescriptor],
    query: &str,
) -> Option<&'a StageDescriptor> {
    let normalized = query.trim().to_ascii_lowercase().replace(' ', "-");
    catalog
        .iter()
        .find(|descriptor| descriptor.id == normalized)
        .or_else(|| {
            let mut matches = catalog.iter().filter(|descriptor| {
                descriptor
                    .id
                    .split_once(':')
                    .is_some_and(|(_, name)| name == normalized)
            });
            let first = matches.next()?;
            matches.next().is_none().then_some(first)
        })
}

fn title_case(id: &str) -> String {
    id.split('-')
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_uppercase().collect::<String>() + chars.as_str()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Clone, Copy, Debug)]
struct Bounds {
    min: DVec3,
    max: DVec3,
}

impl Bounds {
    fn from_mesh(mesh: &MeshBuffers) -> Self {
        let mut min = DVec3::splat(f64::INFINITY);
        let mut max = DVec3::splat(f64::NEG_INFINITY);
        for vertex in &mesh.vertices {
            let point = DVec3::new(
                f64::from(vertex.pos[0]),
                f64::from(vertex.pos[1]),
                f64::from(vertex.pos[2]),
            );
            min = min.min(point);
            max = max.max(point);
        }
        if mesh.vertices.is_empty() {
            min = DVec3::ZERO;
            max = DVec3::ONE;
        }
        Self { min, max }
    }

    fn size(self) -> DVec3 {
        self.max - self.min
    }

    fn centre(self) -> DVec3 {
        (self.min + self.max) * 0.5
    }
}

struct BuiltAsset {
    mesh: MeshBuffers,
    uvs: Option<Vec<[f32; 2]>>,
}

fn build_asset(descriptor: &StageDescriptor) -> BuiltAsset {
    match descriptor.source {
        StageSource::ConstructionFinish {
            material,
            tint,
            variant,
        } => {
            let mut asset = build_construction_finish(SurfaceAppearance { material, tint });
            for vertex in &mut asset.mesh.vertices {
                vertex.face_ao |= u32::from(variant) << 13;
            }
            asset
        }
        StageSource::RoofPigments => build_roof_pigments(),
        StageSource::RoadMaterials => build_road_materials(),
        StageSource::RoadPlaces => build_road_places(),
        StageSource::RoadFinish { material, tint } => {
            build_road_finish(SurfaceAppearance { material, tint })
        }
        StageSource::BuildingCatalogue(scale) => build_building_catalogue(scale),
        StageSource::RegionalBuilding(region, scale) => build_regional_building(
            &ARCHITECTURE_PROFILES[region as usize],
            scale,
            region as usize,
        ),
        StageSource::PlaceBuildings(place) => build_place_buildings(place),
        StageSource::ReviewedBuildings { lat, lon, count } => {
            build_generated_building_samples(lat, lon, None, count)
        }
        StageSource::ProductionHighrise(usage) => {
            build_generated_buildings(40.75_f64.to_radians(), -73.98_f64.to_radians(), Some(usage))
        }
        StageSource::TallBuildingCatalogue => build_tall_building_catalogue(),
        StageSource::TallBuilding(style) => build_tall_building(style),
        StageSource::ObservationTower => build_observation_tower(),
        StageSource::Drone => build_vehicle_stage(crate::vehicles::VehicleKind::Kestrel),
        StageSource::Hoverbike => build_vehicle_stage(crate::vehicles::VehicleKind::Sparrow),
        StageSource::SplitArrow => build_vehicle_stage(crate::vehicles::VehicleKind::SplitArrow),
        StageSource::PhaseManta => build_vehicle_stage(crate::vehicles::VehicleKind::PhaseManta),
        StageSource::Tree(profile) => build_tree(profile),
        StageSource::Landmark(landmark) => build_landmark(landmark),
    }
}

fn construction_finish_studies() -> [(&'static str, &'static str, u8, [u8; 3]); 16] {
    use vearth_gen::material as m;
    [
        ("brick", "Fired brick", m::BRICK_WALL, [167, 98, 73]),
        ("stone", "Dressed stone", m::STONE_WALL, [181, 174, 155]),
        ("plaster", "Lime plaster", m::PLASTER_WALL, [231, 225, 207]),
        ("timber", "Timber boards", m::TIMBER_WALL, [126, 92, 64]),
        ("tile", "Clay roof tile", m::TILE_ROOF, [189, 82, 44]),
        ("slate", "Roof slate", m::SLATE_ROOF, [86, 93, 99]),
        ("panel", "Facade panels", m::PANEL_WALL, [181, 188, 184]),
        (
            "metal",
            "Standing seam metal",
            m::METAL_ROOF,
            [117, 139, 143],
        ),
        ("siding", "Painted siding", m::SIDING_WALL, [187, 205, 197]),
        (
            "shingle",
            "Composition shingles",
            m::SHINGLE_ROOF,
            [108, 103, 96],
        ),
        ("flat", "Flat roof membrane", m::FLAT_ROOF, [142, 148, 147]),
        ("thatch", "Thatch bundles", m::THATCH_ROOF, [155, 132, 84]),
        (
            "namako",
            "Namako plaster joints",
            m::NAMAKO_WALL,
            [86, 91, 92],
        ),
        (
            "limestone",
            "Weathered limestone",
            m::WEATHERED_LIMESTONE,
            [205, 195, 168],
        ),
        ("marble", "Veined marble", m::VEINED_MARBLE, [224, 221, 208]),
        (
            "earth",
            "Earthen plaster",
            m::EARTHEN_PLASTER,
            [179, 132, 98],
        ),
    ]
}

fn construction_variant_studies() -> [(&'static str, &'static str, u8, [u8; 3]); 6] {
    use vearth_gen::material as m;
    [
        (
            "corrugated",
            "Corrugated metal roof",
            m::METAL_ROOF,
            [117, 139, 143],
        ),
        (
            "corrugated-wall",
            "Corrugated metal cladding",
            m::PANEL_WALL,
            [147, 47, 39],
        ),
        (
            "plain-tile",
            "Flat plain clay tiles",
            m::TILE_ROOF,
            [189, 82, 44],
        ),
        (
            "rubble",
            "Uncoursed rubble stone",
            m::STONE_WALL,
            [181, 174, 155],
        ),
        (
            "painted-boards",
            "Painted vertical timber",
            m::TIMBER_WALL,
            [147, 47, 39],
        ),
        (
            "wood-shingle",
            "Wood roof shingles",
            m::SHINGLE_ROOF,
            [124, 108, 82],
        ),
    ]
}

// Identical geometry and physical scale isolate the material response. Two
// upright faces and a sloped roof exercise the production projection paths.
fn build_construction_finish(surface: SurfaceAppearance) -> BuiltAsset {
    let mut builder = StageSurfaceBuilder::default();
    builder.box_surface(
        DVec3::new(-2.0, 0.0, -2.0),
        DVec3::new(2.0, 4.0, 2.0),
        surface,
    );
    builder.faces.push(StageFace {
        points: [
            DVec3::new(-2.0, 4.0, 2.0),
            DVec3::new(2.0, 4.0, 2.0),
            DVec3::new(2.0, 5.5, -2.0),
            DVec3::new(-2.0, 5.5, -2.0),
        ],
        appearance: surface,
    });
    BuiltAsset {
        mesh: builder.mesh(1.0, DVec3::ZERO),
        uvs: None,
    }
}

// Row-major from the rear left: actual emitted Bled taupes, neutral grey,
// then cool grey, charcoal, and navy. Pigments bypass no production shader;
// equal geometry and a common sun expose the resulting displayed colours.
const ROOF_PIGMENTS: [[u8; 3]; 6] = [
    [124, 119, 111],
    [114, 109, 100],
    [100, 100, 100],
    [92, 102, 110],
    [70, 76, 80],
    [47, 63, 82],
];

fn build_roof_pigments() -> BuiltAsset {
    let mut builder = StageSurfaceBuilder::default();
    for (index, tint) in ROOF_PIGMENTS.into_iter().enumerate() {
        let offset = DVec3::new(
            (index % 3) as f64 * 7.0 - 7.0,
            0.0,
            (index / 3) as f64 * 8.0 - 4.0,
        );
        builder.box_surface(
            offset + DVec3::new(-2.0, 0.0, -2.0),
            offset + DVec3::new(2.0, 2.0, 2.0),
            SurfaceAppearance {
                material: vearth_gen::material::PLASTER_WALL,
                tint: [218, 218, 207],
            },
        );
        builder.faces.push(StageFace {
            points: [
                DVec3::new(-2.0, 2.0, 2.0),
                DVec3::new(2.0, 2.0, 2.0),
                DVec3::new(2.0, 3.5, -2.0),
                DVec3::new(-2.0, 3.5, -2.0),
            ]
            .map(|point| point + offset),
            appearance: SurfaceAppearance {
                material: vearth_gen::material::TILE_ROOF,
                tint,
            },
        });
    }
    BuiltAsset {
        mesh: builder.mesh(1.0, DVec3::ZERO),
        uvs: None,
    }
}

fn road_finish_studies() -> [(&'static str, &'static str, u8, [u8; 3]); 8] {
    let labels = [
        ("asphalt", "Asphalt"),
        ("concrete", "Scored concrete"),
        ("brick", "Fired-clay brick"),
        ("setts", "Dressed stone setts"),
        ("gravel", "Compacted gravel"),
        ("earth", "Earth trail"),
        ("cobblestone", "Rounded cobblestone"),
        ("stone-slabs", "Stone flags"),
    ];
    let samples = vearth_gen::road_materials::samples();
    std::array::from_fn(|index| {
        let (slug, label) = labels[index];
        let surface = samples[index];
        (slug, label, surface.material, surface.tint)
    })
}

fn road_panel(mesh: &mut MeshBuffers, surface: SurfaceAppearance, bounds: [f32; 4]) {
    let [x0, z0, x1, z1] = bounds;
    let first = mesh.vertices.len() as u32;
    for [x, z] in [[x0, z0], [x0, z1], [x1, z1], [x1, z0]] {
        mesh.vertices.push(SurfaceVertex {
            pos: [x, 0.06, z],
            face_ao: encode_surface_normal(
                2 | (3 << 3) | (u32::from(surface.material) << 5),
                [0.0, 1.0, 0.0],
            ),
            rgba: [surface.tint[0], surface.tint[1], surface.tint[2], 255],
        });
    }
    mesh.indices
        .extend([first, first + 1, first + 2, first, first + 2, first + 3]);
}

fn build_road_finish(surface: SurfaceAppearance) -> BuiltAsset {
    let mut mesh = MeshBuffers::default();
    road_panel(&mut mesh, surface, [-4.0, -5.0, 4.0, 5.0]);
    BuiltAsset { mesh, uvs: None }
}

fn build_road_materials() -> BuiltAsset {
    let mut mesh = MeshBuffers::default();
    for (index, (_, _, material, tint)) in road_finish_studies().into_iter().enumerate() {
        let x = (index % 4) as f32 * 6.0 - 9.0;
        let z = (index / 4) as f32 * 8.0 - 4.0;
        road_panel(
            &mut mesh,
            SurfaceAppearance { material, tint },
            [x - 2.5, z - 3.5, x + 2.5, z + 3.5],
        );
    }
    BuiltAsset { mesh, uvs: None }
}

fn build_road_places() -> BuiltAsset {
    let samples = vearth_gen::road_materials::local_samples();
    let rows = samples.len().div_ceil(4);
    let mut mesh = MeshBuffers::default();
    for (index, sample) in samples.into_iter().enumerate() {
        let x = (index % 4) as f32 * 6.0 - 9.0;
        let z = ((index / 4) as f32 - (rows as f32 - 1.0) * 0.5) * 8.0;
        road_panel(
            &mut mesh,
            sample.appearance,
            [x - 2.5, z - 3.5, x + 2.5, z + 3.5],
        );
    }
    BuiltAsset { mesh, uvs: None }
}

fn append_mesh(target: &mut MeshBuffers, source: &MeshBuffers, offset: DVec3) {
    let first = target.vertices.len() as u32;
    target
        .vertices
        .extend(source.vertices.iter().copied().map(|mut vertex| {
            vertex.pos[0] += offset.x as f32;
            vertex.pos[1] += offset.y as f32;
            vertex.pos[2] += offset.z as f32;
            vertex
        }));
    target
        .indices
        .extend(source.indices.iter().map(|index| first + index));
}

fn transform_mesh(mesh: &mut MeshBuffers, scale: f32, offset: DVec3) {
    for vertex in &mut mesh.vertices {
        vertex.pos[0] = vertex.pos[0] * scale + offset.x as f32;
        vertex.pos[1] = vertex.pos[1] * scale + offset.y as f32;
        vertex.pos[2] = vertex.pos[2] * scale + offset.z as f32;
    }
}

// Exercise the production geographic resolver and surface pipeline on ordinary
// untagged source footprints. This view includes the resolver's stable infill.
fn build_place_buildings(
    place: &vearth_gen::architecture_places::ArchitecturalPlace,
) -> BuiltAsset {
    build_generated_buildings(
        place.areas[0].representative_center().0.to_radians(),
        place.areas[0].representative_center().1.to_radians(),
        None,
    )
}

fn build_generated_buildings(
    lat: f64,
    lon: f64,
    usage: Option<vearth_data::vectors::BuildingUse>,
) -> BuiltAsset {
    build_generated_building_samples(lat, lon, usage, 4)
}

fn build_generated_building_samples(
    lat: f64,
    lon: f64,
    usage: Option<vearth_data::vectors::BuildingUse>,
    count: u64,
) -> BuiltAsset {
    use vearth_core::{cell::CellId, ellipsoid::geodetic_to_ecef, frame::ChunkFrame};
    use vearth_data::{
        FnElevation,
        vectors::{Building, BuildingKind, FeaturePack, VectorSource},
    };
    struct Samples(
        u64,
        Option<vearth_data::vectors::BuildingUse>,
        [f64; 2],
        bool,
    );
    impl VectorSource for Samples {
        fn features(&self, _: f64, _: f64, _: f64, _: f64) -> FeaturePack {
            let mut pack = FeaturePack::empty();
            pack.buildings.push(Building {
                location_rad: Some(self.2),
                roof_shape: None,
                use_kind: self.1.unwrap_or_default(),
                rings: vec![
                    vec![[-4.0, -4.0], [4.0, -4.0], [4.0, 4.0], [-4.0, 4.0]]
                        .into_iter()
                        .map(|p| {
                            if self.1.is_some() {
                                [p[0] * 2.5, p[1] * 2.5]
                            } else if self.3 {
                                [
                                    p[0] * (0.85 + (self.0 % 3) as f32 * 0.15),
                                    p[1] * (0.85 + ((self.0 / 3) % 3) as f32 * 0.25),
                                ]
                            } else {
                                p
                            }
                        })
                        .collect(),
                ],
                height_m: if self.1.is_some() {
                    128.0
                } else if self.3 {
                    5.0 + (self.0 % 4) as f32 * 2.5
                } else {
                    8.0
                },
                min_height_m: 0.0,
                seed: self.0,
                kind: BuildingKind::Building,
            });
            pack
        }
    }

    let cell = CellId::from_dir(geodetic_to_ecef(lat, lon, 0.0).normalize(), 19);
    let up = DVec3::new(lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin());
    let east = DVec3::new(-lon.sin(), lon.cos(), 0.0);
    let north = up.cross(east);
    let anchor = ChunkFrame::of(vearth_core::cell::ChunkAddr { cell, k: 0 }).anchor;
    let mut mesh = MeshBuffers::default();
    let columns = (count as f64).sqrt().ceil() as u64;
    for seed in 0..count {
        let spacing = if usage.is_some() {
            35.0
        } else if count > 4 {
            16.0
        } else {
            12.0
        };
        let offset = DVec3::new(
            (seed % columns) as f64 * spacing,
            0.0,
            (seed / columns) as f64 * spacing,
        );
        let surface = vearth_gen::generate_surface_cell(
            cell,
            &vearth_gen::Sources {
                elevation: &FnElevation(|_, _| 0.0),
                surface: None,
                imagery: None,
                vectors: Some(&Samples(seed, usage, [lat, lon], count > 4)),
            },
        );
        for batch in surface.batches {
            let frame = ChunkFrame::of(batch.addr);
            for triangle in batch.mesh.indices.chunks_exact(3) {
                let material =
                    ((batch.mesh.vertices[triangle[0] as usize].face_ao >> 5) & 255) as u8;
                if !vearth_gen::material::is_architecture_wall(material)
                    && !vearth_gen::material::is_architecture_roof(material)
                    && !matches!(
                        material,
                        vearth_gen::material::WINDOW | vearth_gen::material::LIT_WINDOW
                    )
                {
                    continue;
                }
                let mut vertices =
                    std::array::from_fn::<_, 3, _>(|i| batch.mesh.vertices[triangle[i] as usize]);
                let points = vertices.map(|v| {
                    let d = frame.local_to_ecef(DVec3::from_array(v.pos.map(f64::from))) - anchor;
                    DVec3::new(d.dot(east), d.dot(up), -d.dot(north)) + offset
                });
                let normal = (points[1] - points[0])
                    .cross(points[2] - points[0])
                    .normalize_or_zero();
                if normal.length_squared() < 0.5 {
                    continue;
                }
                for i in 0..3 {
                    vertices[i].pos = points[i].as_vec3().to_array();
                    vertices[i].face_ao = encode_surface_normal(
                        vertices[i].face_ao & 0xffff,
                        normal.as_vec3().to_array(),
                    );
                }
                let first = mesh.vertices.len() as u32;
                mesh.vertices.extend(vertices);
                mesh.indices.extend([first, first + 1, first + 2]);
            }
        }
    }
    let bounds = Bounds::from_mesh(&mesh);
    transform_mesh(
        &mut mesh,
        1.0,
        DVec3::new(-bounds.centre().x, -bounds.min.y, -bounds.centre().z),
    );
    BuiltAsset { mesh, uvs: None }
}

fn build_vehicle_stage(vehicle: crate::vehicles::VehicleKind) -> BuiltAsset {
    let (mut mesh, uvs) = crate::vehicles::build_stage_surface(vehicle);
    let low = Bounds::from_mesh(&mesh).min.y;
    transform_mesh(&mut mesh, 1.0, DVec3::new(0.0, 1.15 - low, 0.0));
    BuiltAsset {
        mesh,
        uvs: Some(uvs),
    }
}

// Material choices for the optional design-study catalogue only.
fn stage_wall_material(region: ArchitectureRegion) -> u8 {
    use BuildingGrammar::*;
    match vearth_gen::architecture::architecture_profile(region).grammar {
        NordicTimber | Chalet | CapeCod | Machiya | Hanok | TropicalStilt | PacificFale
        | IndonesianRoof | NewariHouse => vearth_gen::material::TIMBER_WALL,
        BritishTerrace | BalticTown => vearth_gen::material::BRICK_WALL,
        Hillside | HarariHouse => vearth_gen::material::STONE_WALL,
        _ => vearth_gen::material::PLASTER_WALL,
    }
}

fn stage_roof_material(region: ArchitectureRegion, shape: Option<RoofShape>) -> u8 {
    if matches!(shape, None | Some(RoofShape::Flat)) {
        return vearth_gen::material::ROOF;
    }
    use BuildingGrammar::*;
    match vearth_gen::architecture::architecture_profile(region).grammar {
        NordicTimber | Chalet | AtlanticRow | BritishTerrace | BalticTown | CapeCod => {
            vearth_gen::material::SLATE_ROOF
        }
        ColonialRow | Continental | Cycladic | Courtyard | Riad | Swahili | IndianPol | Machiya
        | Hanok | IberianHouse | ItalianPortico | Colonia | AzulejoRow => {
            vearth_gen::material::TILE_ROOF
        }
        _ => vearth_gen::material::ROOF,
    }
}

const BUILDING_DESIGN_UNIT_M: f64 = 0.5;
const BUILDING_GLASS: [u8; 3] = [55, 72, 82];

fn building_appearance(material: u8, tint: [u8; 3]) -> SurfaceAppearance {
    SurfaceAppearance {
        material,
        tint: vearth_gen::art_direction::architecture_surface(material, tint),
    }
}

fn building_box_material(
    builder: &mut StageSurfaceBuilder,
    min: (i32, i32, i32),
    max: (i32, i32, i32),
    material: u8,
    tint: [u8; 3],
) {
    builder.box_surface(
        DVec3::new(min.0 as f64, min.1 as f64, min.2 as f64),
        DVec3::new(max.0 as f64 + 1.0, max.1 as f64 + 1.0, max.2 as f64 + 1.0),
        building_appearance(material, tint),
    );
}

fn building_box(
    builder: &mut StageSurfaceBuilder,
    min: (i32, i32, i32),
    max: (i32, i32, i32),
    tint: [u8; 3],
) {
    building_box_material(builder, min, max, vearth_gen::material::WALL, tint);
}

fn building_roof_box(
    builder: &mut StageSurfaceBuilder,
    min: (i32, i32, i32),
    max: (i32, i32, i32),
    tint: [u8; 3],
) {
    building_box_material(builder, min, max, vearth_gen::material::ROOF, tint);
}

fn building_opening(
    builder: &mut StageSurfaceBuilder,
    x: i32,
    y: i32,
    z: i32,
    half_width: i32,
    height: i32,
    trim: [u8; 3],
) {
    building_box(
        builder,
        (x - half_width - 1, y - 1, z),
        (x + half_width + 1, y + height, z),
        trim,
    );
    building_box_material(
        builder,
        (x - half_width, y, z),
        (x + half_width, y + height - 1, z),
        vearth_gen::material::WINDOW,
        BUILDING_GLASS,
    );
}

fn building_door(
    builder: &mut StageSurfaceBuilder,
    x: i32,
    base_y: i32,
    z: i32,
    width: i32,
    height: i32,
    tint: [u8; 3],
) {
    building_box(
        builder,
        (x - width / 2, base_y, z),
        (x + width / 2, base_y + height - 1, z),
        tint,
    );
}

fn building_roof(
    builder: &mut StageSurfaceBuilder,
    shape: RoofShape,
    half_width: i32,
    half_depth: i32,
    eave_y: i32,
    rise: i32,
    overhang: i32,
    tint: [u8; 3],
) {
    let w = (half_width + overhang) as f64 + 0.5;
    let d = (half_depth + overhang) as f64 + 0.5;
    let base = eave_y as f64;
    let h = (rise + 1) as f64;
    let appearance = building_appearance(vearth_gen::material::ROOF, tint);
    let rectangle = |wx: f64, dz: f64, y: f64| {
        vec![
            DVec3::new(-wx, y, -dz),
            DVec3::new(-wx, y, dz),
            DVec3::new(wx, y, dz),
            DVec3::new(wx, y, -dz),
        ]
    };
    match shape {
        RoofShape::Flat => {
            building_roof_box(
                builder,
                (-(w as i32), eave_y, -(d as i32)),
                (w as i32, eave_y, d as i32),
                tint,
            );
            for z in [-(d as i32), d as i32] {
                building_roof_box(
                    builder,
                    (-(w as i32), eave_y + 1, z),
                    (w as i32, eave_y + 1, z),
                    tint,
                );
            }
            for x in [-(w as i32), w as i32] {
                building_roof_box(
                    builder,
                    (x, eave_y + 1, -(d as i32)),
                    (x, eave_y + 1, d as i32),
                    tint,
                );
            }
        }
        RoofShape::Skillion => {
            let top = vec![
                DVec3::new(-w, base + 1., -d),
                DVec3::new(-w, base + h, d),
                DVec3::new(w, base + h, d),
                DVec3::new(w, base + 1., -d),
            ];
            builder.loft(&[rectangle(w, d, base), top], appearance);
        }
        RoofShape::Dome | RoofShape::Onion => {
            let rings = (0..=12)
                .map(|i| {
                    let t = i as f64 / 12.;
                    let angle = t * std::f64::consts::FRAC_PI_2;
                    let radius = angle.cos().max(0.001)
                        * if shape == RoofShape::Onion {
                            1. + 0.22 * (t * std::f64::consts::PI).sin()
                        } else {
                            1.
                        };
                    ellipse_ring(w * radius, d * radius, base + h * angle.sin(), 0.)
                })
                .collect::<Vec<_>>();
            builder.loft(&rings, appearance);
        }
        RoofShape::Hipped | RoofShape::Pyramidal | RoofShape::Mansard => {
            let inset = w.min(d) - 0.001;
            let mut rings = vec![rectangle(w, d, base)];
            if shape == RoofShape::Mansard {
                rings.push(rectangle(
                    w - inset * 0.55,
                    d - inset * 0.55,
                    base + h * 0.65,
                ));
            }
            rings.push(rectangle(w - inset, d - inset, base + h));
            builder.loft(&rings, appearance);
        }
        RoofShape::Gabled | RoofShape::Gambrel | RoofShape::Sawtooth => {
            let mut rings = vec![rectangle(w, d, base)];
            if shape == RoofShape::Gambrel {
                rings.push(rectangle(w * 0.48, d, base + h * 0.55));
            }
            rings.push(rectangle(0.001, d, base + h));
            builder.loft(&rings, appearance);
        }
    }
}

fn building_balcony(
    builder: &mut StageSurfaceBuilder,
    half_width: i32,
    y: i32,
    front_z: i32,
    tint: [u8; 3],
) {
    building_box(
        builder,
        (-half_width, y, front_z),
        (half_width, y, front_z + 2),
        tint,
    );
    for x in [-half_width, 0, half_width] {
        building_box(
            builder,
            (x, y + 1, front_z + 2),
            (x, y + 3, front_z + 2),
            tint,
        );
    }
    building_box(
        builder,
        (-half_width, y + 3, front_z + 2),
        (half_width, y + 3, front_z + 2),
        tint,
    );
}

fn building_veranda(
    builder: &mut StageSurfaceBuilder,
    half_width: i32,
    base_y: i32,
    front_z: i32,
    canopy_y: i32,
    posts: [u8; 3],
    roof: [u8; 3],
) {
    building_box(
        builder,
        (-half_width, base_y, front_z),
        (half_width, base_y, front_z + 3),
        posts,
    );
    building_roof_box(
        builder,
        (-half_width - 1, canopy_y, front_z - 1),
        (half_width + 1, canopy_y, front_z + 4),
        roof,
    );
    for x in [-half_width, 0, half_width] {
        building_box(
            builder,
            (x, base_y + 1, front_z + 3),
            (x, canopy_y - 1, front_z + 3),
            posts,
        );
    }
}

fn regional_dimensions(profile: &ArchitectureProfile, scale: BuildingScale) -> (i32, i32, i32) {
    let medium = scale == BuildingScale::MediumBuilding;
    let storeys = if medium {
        profile.medium_storeys
    } else {
        profile.short_storeys
    } as i32;
    let (width, depth) = match profile.grammar {
        BuildingGrammar::VerandaBungalow | BuildingGrammar::Chalet | BuildingGrammar::Hanok => {
            if medium {
                (14, 9)
            } else {
                (12, 7)
            }
        }
        BuildingGrammar::ColonialRow
        | BuildingGrammar::AtlanticRow
        | BuildingGrammar::ArabianTower
        | BuildingGrammar::IndianPol
        | BuildingGrammar::Machiya
        | BuildingGrammar::BritishTerrace
        | BuildingGrammar::IberianHouse
        | BuildingGrammar::BalticTown
        | BuildingGrammar::HarariHouse
        | BuildingGrammar::NewariHouse
        | BuildingGrammar::AzulejoRow => {
            if medium {
                (9, 10)
            } else {
                (7, 9)
            }
        }
        BuildingGrammar::PacificFale
        | BuildingGrammar::ItalianPortico
        | BuildingGrammar::AsanteCourtyard
        | BuildingGrammar::CapeCod
        | BuildingGrammar::IndonesianRoof => {
            if medium {
                (13, 9)
            } else {
                (11, 8)
            }
        }
        _ => {
            if medium {
                (12, 9)
            } else {
                (9, 7)
            }
        }
    };
    (width, depth, storeys)
}

fn build_regional_building(
    profile: &ArchitectureProfile,
    scale: BuildingScale,
    variant: usize,
) -> BuiltAsset {
    let medium = scale == BuildingScale::MediumBuilding;
    let (half_width, half_depth, storeys) = regional_dimensions(profile, scale);
    let base_y = match profile.grammar {
        BuildingGrammar::TropicalStilt
        | BuildingGrammar::PacificFale
        | BuildingGrammar::IndonesianRoof => 4,
        BuildingGrammar::TropicalShade if !medium => 2,
        _ => 0,
    };
    let floor_h = 6;
    let wall_top = base_y + storeys * floor_h - 1;
    let colorway = match profile.id {
        ArchitectureRegion::Nordic
        | ArchitectureRegion::Sydney
        | ArchitectureRegion::Santorini
        | ArchitectureRegion::AtlanticEurope
        | ArchitectureRegion::ContinentalEurope
        | ArchitectureRegion::BritishIsles
        | ArchitectureRegion::Iberia
        | ArchitectureRegion::Italy
        | ArchitectureRegion::WestAfrica
        | ArchitectureRegion::EthiopianHighlands
        | ArchitectureRegion::USSouthwest
        | ArchitectureRegion::NewEngland
        | ArchitectureRegion::SouthernCone
        | ArchitectureRegion::Indonesia
        | ArchitectureRegion::Himalaya
        | ArchitectureRegion::AtlanticBrazil => 0,
        ArchitectureRegion::Salvador | ArchitectureRegion::Baltic => 1,
        _ => variant % profile.walls.len(),
    };
    let wall = profile.walls[colorway];
    let wall_alt = profile.walls[(colorway + 1) % profile.walls.len()];
    let trim = profile.accents[0];
    let detail = profile.accents[1];
    let dark = profile.accents[2];
    let roof = profile.roofs[colorway % profile.roofs.len()];
    let roof_shape = if medium {
        profile.medium_roofs[variant & 1]
    } else {
        profile.short_roofs[variant & 1]
    };
    let mut builder = StageSurfaceBuilder::default();

    building_box(
        &mut builder,
        (-half_width, base_y, -half_depth),
        (half_width, wall_top, half_depth),
        wall,
    );

    // Regular openings establish scale; every regional grammar then modifies
    // the body with a larger silhouette or climate-specific device.
    let bays = if half_width >= 12 {
        5
    } else if half_width >= 9 {
        4
    } else {
        3
    };
    let has_regular_openings = !matches!(
        profile.grammar,
        BuildingGrammar::IndianPol
            | BuildingGrammar::Machiya
            | BuildingGrammar::Hanok
            | BuildingGrammar::PacificFale
            | BuildingGrammar::AsanteCourtyard
            | BuildingGrammar::HarariHouse
            | BuildingGrammar::Pueblo
            | BuildingGrammar::IndonesianRoof
            | BuildingGrammar::NewariHouse
    );
    if has_regular_openings {
        for floor in 0..storeys {
            let y = base_y + floor * floor_h + 2;
            for bay in 0..bays {
                let x = if bays == 1 {
                    0
                } else {
                    -half_width + 3 + bay * (2 * half_width - 6) / (bays - 1)
                };
                if floor == 0 && bay == bays / 2 {
                    continue;
                }
                building_opening(&mut builder, x, y, half_depth, 1, 3, trim);
            }
        }
    }
    building_door(&mut builder, 0, base_y, half_depth, 3, 5, dark);

    let rise = match profile.grammar {
        BuildingGrammar::IndonesianRoof => {
            if medium {
                8
            } else {
                10
            }
        }
        BuildingGrammar::AsanteCourtyard => 7,
        _ => match roof_shape {
            RoofShape::Flat => 1,
            RoofShape::Dome | RoofShape::Onion => 5,
            _ => {
                if medium {
                    6
                } else {
                    5
                }
            }
        },
    };
    let overhang = match profile.grammar {
        BuildingGrammar::TropicalShade
        | BuildingGrammar::NordicTimber
        | BuildingGrammar::VerandaBungalow
        | BuildingGrammar::Chalet
        | BuildingGrammar::TropicalStilt
        | BuildingGrammar::Hanok
        | BuildingGrammar::PacificFale
        | BuildingGrammar::ItalianPortico
        | BuildingGrammar::AsanteCourtyard
        | BuildingGrammar::CapeCod
        | BuildingGrammar::Colonia
        | BuildingGrammar::NewariHouse => 3,
        BuildingGrammar::IndonesianRoof => 4,
        _ => 1,
    };
    building_roof(
        &mut builder,
        roof_shape,
        half_width,
        half_depth,
        wall_top + 1,
        rise,
        overhang,
        roof,
    );

    match profile.grammar {
        BuildingGrammar::TropicalShade => {
            if medium {
                // Kinshasa tropical modernism: continuous balconies and
                // projecting horizontal/vertical sun-breaks.
                for floor in 1..storeys {
                    let y = base_y + floor * floor_h;
                    building_balcony(&mut builder, half_width - 1, y, half_depth + 1, detail);
                    building_box(
                        &mut builder,
                        (-half_width - 1, y + 4, half_depth + 1),
                        (half_width + 1, y + 4, half_depth + 3),
                        trim,
                    );
                }
                for x in [-half_width + 2, half_width - 2] {
                    building_box(
                        &mut builder,
                        (x, base_y + 1, half_depth + 3),
                        (x, wall_top, half_depth + 3),
                        trim,
                    );
                }
            } else {
                building_veranda(
                    &mut builder,
                    half_width - 1,
                    0,
                    half_depth + 1,
                    base_y + 5,
                    dark,
                    roof,
                );
            }
        }
        BuildingGrammar::NordicTimber => {
            for x in [-half_width, half_width] {
                building_box(
                    &mut builder,
                    (x, base_y, half_depth + 1),
                    (x, wall_top, half_depth + 1),
                    trim,
                );
            }
            building_box(
                &mut builder,
                (half_width - 3, wall_top + 2, -2),
                (half_width - 1, wall_top + rise + 3, 0),
                dark,
            );
        }
        BuildingGrammar::VerandaBungalow => {
            building_veranda(
                &mut builder,
                half_width - 2,
                0,
                half_depth + 1,
                base_y + 5,
                trim,
                roof,
            );
            if !medium {
                // Projecting Federation bay and secondary front gable.
                building_box(
                    &mut builder,
                    (-half_width, base_y, half_depth + 1),
                    (-2, wall_top, half_depth + 3),
                    wall_alt,
                );
                building_roof(
                    &mut builder,
                    RoofShape::Gabled,
                    (half_width - 2) / 2,
                    2,
                    wall_top + 1,
                    4,
                    1,
                    roof,
                );
            } else {
                // Sydney inter-war flats: stepped parapet and corner balconies.
                for floor in 1..storeys {
                    building_balcony(
                        &mut builder,
                        half_width / 2,
                        base_y + floor * floor_h,
                        half_depth + 1,
                        trim,
                    );
                }
                building_box(
                    &mut builder,
                    (-5, wall_top + 2, -half_depth),
                    (5, wall_top + 4, half_depth),
                    wall_alt,
                );
            }
        }
        BuildingGrammar::ColonialRow => {
            for floor in 0..storeys {
                let y = base_y + (floor + 1) * floor_h - 1;
                building_box(
                    &mut builder,
                    (-half_width - 1, y, half_depth + 1),
                    (half_width + 1, y, half_depth + 1),
                    trim,
                );
            }
            for x in [-half_width, half_width] {
                building_box(
                    &mut builder,
                    (x, base_y, half_depth + 1),
                    (x, wall_top, half_depth + 1),
                    trim,
                );
            }
            if medium {
                building_balcony(
                    &mut builder,
                    half_width - 1,
                    base_y + 2 * floor_h,
                    half_depth + 1,
                    dark,
                );
            }
            if profile.id == ArchitectureRegion::Caribbean {
                // Deep galleries turn the colourful street wall into a
                // tropical Caribbean type rather than another Salvador row.
                building_veranda(
                    &mut builder,
                    half_width,
                    base_y,
                    half_depth + 1,
                    base_y + 5,
                    trim,
                    roof,
                );
                if medium {
                    building_balcony(
                        &mut builder,
                        half_width - 1,
                        base_y + 2 * floor_h,
                        half_depth + 1,
                        trim,
                    );
                }
            }
        }
        BuildingGrammar::Cycladic => {
            // Interlocked, stepped volumes and a small vault make Santorini
            // legible without relying on the blue-and-white palette alone.
            building_box(
                &mut builder,
                (1, base_y + 2, -half_depth - 2),
                (
                    half_width + 5,
                    wall_top - if medium { 3 } else { 0 },
                    half_depth - 1,
                ),
                wall_alt,
            );
            building_roof(
                &mut builder,
                if medium {
                    RoofShape::Flat
                } else {
                    RoofShape::Dome
                },
                4,
                4,
                wall_top + if medium { -2 } else { 1 },
                4,
                0,
                profile.roofs[1],
            );
            building_box(
                &mut builder,
                (-half_width - 1, wall_top + 2, -half_depth),
                (-half_width + 1, wall_top + 5, -half_depth + 2),
                trim,
            );
        }
        BuildingGrammar::AtlanticRow => {
            building_box(
                &mut builder,
                (-3, base_y, half_depth + 1),
                (3, base_y + floor_h + 1, half_depth + 3),
                wall_alt,
            );
            for x in [-half_width + 2, half_width - 2] {
                building_box(
                    &mut builder,
                    (x, wall_top + 2, -2),
                    (x + 1, wall_top + rise + 3, 0),
                    dark,
                );
            }
        }
        BuildingGrammar::Continental => {
            building_box(
                &mut builder,
                (-half_width - 1, wall_top - 1, half_depth + 1),
                (half_width + 1, wall_top, half_depth + 1),
                trim,
            );
            for x in [-half_width / 2, half_width / 2] {
                building_box(
                    &mut builder,
                    (x - 1, wall_top + 2, half_depth),
                    (x + 1, wall_top + 4, half_depth + 1),
                    wall_alt,
                );
                building_opening(&mut builder, x, wall_top + 2, half_depth + 1, 0, 2, trim);
            }
        }
        BuildingGrammar::Chalet => {
            for floor in 1..storeys {
                building_balcony(
                    &mut builder,
                    half_width,
                    base_y + floor * floor_h,
                    half_depth + 1,
                    dark,
                );
            }
            for x in [-half_width, 0, half_width] {
                building_box(
                    &mut builder,
                    (x, base_y, half_depth + 1),
                    (x, wall_top, half_depth + 1),
                    dark,
                );
            }
        }
        BuildingGrammar::PorchHouse => {
            building_veranda(
                &mut builder,
                half_width - 2,
                0,
                half_depth + 1,
                base_y + 5,
                trim,
                roof,
            );
            building_box(
                &mut builder,
                (-2, wall_top + 1, half_depth - 1),
                (2, wall_top + 4, half_depth + 1),
                wall_alt,
            );
            building_roof(
                &mut builder,
                RoofShape::Gabled,
                3,
                2,
                wall_top + 4,
                3,
                1,
                roof,
            );
            if profile.id == ArchitectureRegion::SouthernAfrica {
                // A second end gable and long stoep keep the Cape-derived
                // silhouette broad and symmetrical.
                building_box(
                    &mut builder,
                    (-half_width + 1, wall_top + 1, -half_depth + 1),
                    (-half_width + 4, wall_top + 5, half_depth - 1),
                    wall_alt,
                );
                building_roof(
                    &mut builder,
                    RoofShape::Gabled,
                    3,
                    half_depth,
                    wall_top + 4,
                    3,
                    1,
                    roof,
                );
            }
        }
        BuildingGrammar::Courtyard => {
            // Street-closed body, arcade rhythm and roof terrace.
            for x in (-half_width + 2..=half_width - 2).step_by(4) {
                building_box(
                    &mut builder,
                    (x, base_y, half_depth + 1),
                    (x, base_y + floor_h - 1, half_depth + 3),
                    trim,
                );
            }
            building_box(
                &mut builder,
                (-half_width, wall_top + 2, -half_depth),
                (-half_width, wall_top + 4, half_depth),
                detail,
            );
            if profile.id == ArchitectureRegion::CentralAsia {
                // A tall framed entrance and veranda-like upper screen echo
                // the street edge of courtyard houses without copying a
                // ceremonial monument.
                building_box(
                    &mut builder,
                    (-3, base_y, half_depth + 2),
                    (3, wall_top + 3, half_depth + 3),
                    trim,
                );
                building_door(&mut builder, 0, base_y, half_depth + 3, 3, 5, dark);
            }
            building_box(
                &mut builder,
                (half_width, wall_top + 2, -half_depth),
                (half_width, wall_top + 4, half_depth),
                detail,
            );
        }
        BuildingGrammar::Hillside => {
            building_box(
                &mut builder,
                (2, base_y + floor_h, -half_depth - 2),
                (half_width + 5, wall_top + 2, half_depth - 1),
                wall_alt,
            );
            for step in 0..6 {
                building_box(
                    &mut builder,
                    (half_width + 1 + step, base_y + step, half_depth + 1),
                    (half_width + 2 + step, base_y + step, half_depth + 2),
                    dark,
                );
            }
        }
        BuildingGrammar::EarthenSahel => {
            for x in [-half_width, half_width] {
                for layer in 0..=wall_top / 3 {
                    let inset = layer.min(2);
                    building_box(
                        &mut builder,
                        (
                            x - 1 + inset * x.signum(),
                            base_y + layer * 3,
                            half_depth + 1,
                        ),
                        (
                            x + 1,
                            (base_y + layer * 3 + 2).min(wall_top),
                            half_depth + 3 - inset,
                        ),
                        wall_alt,
                    );
                }
            }
            for x in (-half_width..=half_width).step_by(4) {
                building_box(
                    &mut builder,
                    (x, wall_top + 2, -half_depth - 2),
                    (x, wall_top + 2, half_depth + 2),
                    dark,
                );
            }
        }
        BuildingGrammar::Swahili => {
            if medium {
                building_balcony(
                    &mut builder,
                    half_width - 1,
                    base_y + floor_h,
                    half_depth + 1,
                    dark,
                );
            }
            for x in (-half_width..=half_width).step_by(4) {
                building_box(
                    &mut builder,
                    (x, wall_top + 2, half_depth),
                    (x + 1, wall_top + 4, half_depth + 1),
                    wall_alt,
                );
            }
            building_door(&mut builder, 0, base_y, half_depth + 1, 5, 5, dark);
        }
        BuildingGrammar::Riad => {
            for x in [-half_width, half_width] {
                building_box(
                    &mut builder,
                    (x, wall_top + 2, -half_depth),
                    (x, wall_top + 5, -half_depth + 2),
                    wall_alt,
                );
                building_box(
                    &mut builder,
                    (x, wall_top + 2, half_depth - 2),
                    (x, wall_top + 5, half_depth),
                    wall_alt,
                );
            }
            // Sparse street openings keep the inward-looking mass legible.
            building_box(
                &mut builder,
                (-half_width + 2, base_y + 2, half_depth + 1),
                (half_width - 2, wall_top - 1, half_depth + 1),
                wall,
            );
            building_door(&mut builder, 0, base_y, half_depth + 1, 3, 5, dark);
        }
        BuildingGrammar::ArabianTower => {
            for floor in 0..storeys {
                let y = base_y + (floor + 1) * floor_h - 1;
                building_box(
                    &mut builder,
                    (-half_width - 1, y, half_depth + 1),
                    (half_width + 1, y, half_depth + 1),
                    trim,
                );
            }
            for x in [-half_width, half_width] {
                building_box(
                    &mut builder,
                    (x, base_y, half_depth + 1),
                    (x, wall_top, half_depth + 1),
                    wall_alt,
                );
            }
        }
        BuildingGrammar::IndianPol => {
            for floor in 1..storeys {
                building_balcony(
                    &mut builder,
                    half_width - 1,
                    base_y + floor * floor_h,
                    half_depth + 1,
                    dark,
                );
            }
            for floor in 0..storeys {
                let y = base_y + floor * floor_h + 2;
                for x in [-half_width / 2, half_width / 2] {
                    building_box(
                        &mut builder,
                        (x - 2, y - 1, half_depth + 2),
                        (x + 2, y + 2, half_depth + 3),
                        dark,
                    );
                    building_opening(&mut builder, x, y, half_depth + 3, 1, 2, trim);
                }
            }
        }
        BuildingGrammar::TropicalStilt => {
            for x in [-half_width + 2, 0, half_width - 2] {
                for z in [-half_depth + 2, half_depth - 2] {
                    building_box(&mut builder, (x, 0, z), (x + 1, base_y - 1, z + 1), dark);
                }
            }
            building_balcony(&mut builder, half_width, base_y, half_depth + 1, detail);
            for x in [-half_width, half_width] {
                building_box(
                    &mut builder,
                    (x, base_y, half_depth + 1),
                    (x, wall_top, half_depth + 1),
                    dark,
                );
            }
        }
        BuildingGrammar::Machiya => {
            for x in [-half_width, -half_width / 3, half_width / 3, half_width] {
                building_box(
                    &mut builder,
                    (x, base_y, half_depth + 1),
                    (x, (base_y + floor_h - 1).min(wall_top), half_depth + 2),
                    dark,
                );
            }
            for floor in 1..storeys {
                building_box(
                    &mut builder,
                    (-half_width, base_y + floor * floor_h, half_depth + 1),
                    (half_width, base_y + floor * floor_h, half_depth + 2),
                    trim,
                );
            }
            for x in [-half_width / 2, half_width / 2] {
                building_opening(&mut builder, x, base_y + 2, half_depth + 2, 1, 3, detail);
            }
        }
        BuildingGrammar::Hanok => {
            building_box(
                &mut builder,
                (-half_width - 2, base_y, -half_depth - 2),
                (half_width + 2, base_y, half_depth + 2),
                dark,
            );
            for x in [-half_width, 0, half_width] {
                building_box(
                    &mut builder,
                    (x, base_y, half_depth + 1),
                    (x, wall_top, half_depth + 1),
                    dark,
                );
            }
            building_box(
                &mut builder,
                (-half_width - 3, wall_top + 1, -half_depth - 3),
                (half_width + 3, wall_top + 1, half_depth + 3),
                roof,
            );
            for floor in 0..storeys {
                let y = base_y + floor * floor_h + 2;
                for x in [-half_width / 2, half_width / 2] {
                    building_opening(&mut builder, x, y, half_depth + 1, 1, 2, detail);
                }
            }
            if profile.id == ArchitectureRegion::China {
                // Red structural bays and short side wings establish a
                // courtyard-house reading distinct from the Korean profile.
                for x in [-half_width, half_width] {
                    building_box(
                        &mut builder,
                        (x, base_y, half_depth + 2),
                        (x, wall_top, half_depth + 2),
                        trim,
                    );
                }
                building_box(
                    &mut builder,
                    (-half_width, wall_top - 1, half_depth + 2),
                    (half_width, wall_top, half_depth + 2),
                    trim,
                );
                building_box(
                    &mut builder,
                    (-half_width - 4, base_y, -half_depth + 1),
                    (-half_width - 1, wall_top - 2, half_depth + 1),
                    wall_alt,
                );
                building_box(
                    &mut builder,
                    (half_width + 1, base_y, -half_depth + 1),
                    (half_width + 4, wall_top - 2, half_depth + 1),
                    wall_alt,
                );
            }
        }
        BuildingGrammar::PacificFale => {
            for x in [-half_width + 2, 0, half_width - 2] {
                for z in [-half_depth + 2, half_depth - 2] {
                    building_box(&mut builder, (x, 0, z), (x + 1, wall_top, z + 1), dark);
                }
            }
            // Repaint the long elevations as a shaded open perimeter.
            building_box(
                &mut builder,
                (-half_width + 2, base_y + 1, half_depth),
                (half_width - 2, wall_top - 1, half_depth),
                BUILDING_GLASS,
            );
            building_balcony(&mut builder, half_width + 1, base_y, half_depth + 1, detail);
        }
        BuildingGrammar::BritishTerrace => {
            building_box(
                &mut builder,
                (-4, base_y, half_depth + 1),
                (1, (base_y + floor_h + 1).min(wall_top), half_depth + 3),
                wall_alt,
            );
            building_opening(&mut builder, -1, base_y + 2, half_depth + 3, 1, 3, trim);
            for floor in 1..storeys {
                let y = base_y + floor * floor_h;
                building_box(
                    &mut builder,
                    (-half_width, y, half_depth + 1),
                    (half_width, y, half_depth + 1),
                    trim,
                );
            }
            building_box(
                &mut builder,
                (half_width - 3, wall_top + 2, -2),
                (half_width - 1, wall_top + rise + 3, 0),
                dark,
            );
        }
        BuildingGrammar::IberianHouse => {
            building_box(
                &mut builder,
                (-half_width - 1, wall_top - 1, half_depth + 1),
                (half_width + 1, wall_top, half_depth + 1),
                trim,
            );
            for x in [-half_width + 2, half_width - 2] {
                building_box(
                    &mut builder,
                    (x - 1, base_y + 1, half_depth + 2),
                    (x + 1, base_y + floor_h - 1, half_depth + 2),
                    detail,
                );
            }
            if medium {
                building_balcony(
                    &mut builder,
                    half_width - 2,
                    base_y + 2 * floor_h,
                    half_depth + 1,
                    dark,
                );
            }
        }
        BuildingGrammar::ItalianPortico => {
            for x in [-half_width / 2, 0, half_width / 2] {
                building_opening(&mut builder, x, base_y + 2, half_depth + 2, 1, 4, trim);
            }
            building_box(
                &mut builder,
                (-half_width - 1, base_y + floor_h, half_depth + 1),
                (half_width + 1, base_y + floor_h + 1, half_depth + 1),
                trim,
            );
        }
        BuildingGrammar::BalticTown => {
            for x in [-half_width, half_width] {
                building_box(
                    &mut builder,
                    (x, base_y, half_depth + 1),
                    (x, wall_top, half_depth + 1),
                    trim,
                );
            }
            for floor in 1..storeys {
                let y = base_y + floor * floor_h;
                building_box(
                    &mut builder,
                    (-half_width, y, half_depth + 1),
                    (half_width, y, half_depth + 1),
                    detail,
                );
            }
            building_box(
                &mut builder,
                (-2, wall_top + 2, half_depth),
                (2, wall_top + 5, half_depth + 1),
                wall_alt,
            );
        }
        BuildingGrammar::AsanteCourtyard => {
            for side in [-1, 1] {
                let x0 = side * (half_width + 4);
                building_box(
                    &mut builder,
                    (x0 - 3, base_y, -half_depth + 1),
                    (x0 + 3, (base_y + floor_h - 1).min(wall_top), half_depth + 5),
                    wall_alt,
                );
            }
            for y in [base_y + 2, base_y + 4] {
                building_box(
                    &mut builder,
                    (-half_width, y, half_depth + 1),
                    (half_width, y, half_depth + 1),
                    if y == base_y + 2 { detail } else { trim },
                );
            }
        }
        BuildingGrammar::HarariHouse => {
            building_box(
                &mut builder,
                (-half_width, base_y + 1, half_depth + 1),
                (half_width, wall_top, half_depth + 1),
                wall,
            );
            for floor in 0..storeys {
                let y = base_y + floor * floor_h + 2;
                for x in [-half_width / 2, half_width / 2] {
                    building_opening(&mut builder, x, y, half_depth + 1, 0, 2, trim);
                }
            }
            building_door(&mut builder, 0, base_y, half_depth + 2, 3, 5, dark);
            if medium {
                building_balcony(
                    &mut builder,
                    half_width - 2,
                    base_y + floor_h,
                    half_depth + 1,
                    dark,
                );
            }
        }
        BuildingGrammar::Pueblo => {
            building_box(
                &mut builder,
                (1, base_y + floor_h, -half_depth - 2),
                (half_width + 5, wall_top + 2, half_depth - 1),
                wall_alt,
            );
            building_box(
                &mut builder,
                (2, wall_top + 3, -half_depth - 2),
                (half_width + 5, wall_top + 3, half_depth - 1),
                roof,
            );
            for x in (-half_width..=half_width).step_by(5) {
                building_box(
                    &mut builder,
                    (x, wall_top, half_depth),
                    (x, wall_top, half_depth + 4),
                    dark,
                );
            }
            for x in [-half_width / 2, half_width / 2] {
                building_opening(&mut builder, x, base_y + 2, half_depth + 1, 0, 2, detail);
            }
        }
        BuildingGrammar::CapeCod => {
            building_box(
                &mut builder,
                (-1, wall_top + 1, -2),
                (1, wall_top + rise + 4, 0),
                dark,
            );
            for x in [-half_width / 2, half_width / 2] {
                building_box(
                    &mut builder,
                    (x - 2, wall_top + 2, half_depth - 1),
                    (x + 2, wall_top + 5, half_depth + 2),
                    wall_alt,
                );
                building_opening(&mut builder, x, wall_top + 3, half_depth + 2, 0, 2, trim);
                building_box(
                    &mut builder,
                    (x - 2, wall_top + 6, half_depth - 1),
                    (x + 2, wall_top + 6, half_depth + 2),
                    roof,
                );
            }
        }
        BuildingGrammar::Colonia => {
            building_box(
                &mut builder,
                (-half_width - 1, base_y, half_depth + 1),
                (half_width + 1, base_y + 1, half_depth + 2),
                detail,
            );
            for x in [-half_width / 2, half_width / 2] {
                for shutter_x in [x - 2, x + 2] {
                    building_box(
                        &mut builder,
                        (shutter_x, base_y + 2, half_depth + 2),
                        (shutter_x, base_y + 5, half_depth + 2),
                        dark,
                    );
                }
            }
        }
        BuildingGrammar::IndonesianRoof => {
            for x in [-half_width + 2, half_width - 2] {
                for z in [-half_depth + 2, half_depth - 2] {
                    building_box(&mut builder, (x, 0, z), (x + 1, base_y - 1, z + 1), dark);
                }
            }
            building_box(
                &mut builder,
                (-half_width - 2, base_y, half_depth + 1),
                (half_width + 2, base_y, half_depth + 4),
                detail,
            );
            building_box(
                &mut builder,
                (-half_width - 4, wall_top + 1, -half_depth - 3),
                (half_width + 4, wall_top + 1, half_depth + 3),
                roof,
            );
            for x in [-half_width / 2, half_width / 2] {
                building_opening(&mut builder, x, base_y + 2, half_depth + 1, 1, 3, detail);
            }
        }
        BuildingGrammar::NewariHouse => {
            for floor in 0..storeys {
                let y = base_y + floor * floor_h + 1;
                for x in [-half_width / 2, half_width / 2] {
                    building_box(
                        &mut builder,
                        (x - 2, y, half_depth + 1),
                        (x + 2, y + 3, half_depth + 2),
                        dark,
                    );
                    building_opening(&mut builder, x, y + 1, half_depth + 2, 1, 2, trim);
                }
            }
            building_box(
                &mut builder,
                (-half_width - 2, wall_top + 1, -half_depth - 2),
                (half_width + 2, wall_top + 1, half_depth + 2),
                roof,
            );
        }
        BuildingGrammar::AzulejoRow => {
            for floor in 0..storeys {
                let y = base_y + (floor + 1) * floor_h - 1;
                building_box(
                    &mut builder,
                    (-half_width - 1, y, half_depth + 1),
                    (half_width + 1, y, half_depth + 1),
                    trim,
                );
            }
            for x in [-half_width, half_width] {
                building_box(
                    &mut builder,
                    (x, base_y + 1, half_depth + 2),
                    (x, wall_top, half_depth + 2),
                    detail,
                );
            }
            if medium {
                building_balcony(
                    &mut builder,
                    half_width - 1,
                    base_y + 2 * floor_h,
                    half_depth + 1,
                    dark,
                );
            }
        }
    }

    let mesh = builder.architectural_mesh(
        BUILDING_DESIGN_UNIT_M as f32,
        DVec3::ZERO,
        Some((profile.id, roof_shape)),
    );
    BuiltAsset { mesh, uvs: None }
}

fn build_building_catalogue(scale: BuildingScale) -> BuiltAsset {
    let medium = scale == BuildingScale::MediumBuilding;
    let columns = 6usize;
    let spacing_x = if medium { 24.0 } else { 20.0 };
    let spacing_z = if medium { 28.0 } else { 23.0 };
    let rows = ARCHITECTURE_PROFILES.len().div_ceil(columns);
    let x0 = -((columns - 1) as f64) * spacing_x * 0.5;
    // The five art-direction anchors lead the first data row, so place row
    // zero nearest the default camera rather than hiding it behind the global
    // catalogue.
    let z0 = (rows - 1) as f64 * spacing_z * 0.5;
    let mut mesh = MeshBuffers::default();
    for (index, profile) in ARCHITECTURE_PROFILES.iter().enumerate() {
        let asset = build_regional_building(profile, scale, index);
        let column = index % columns;
        let row = index / columns;
        append_built_asset(
            &mut mesh,
            &asset,
            DVec3::new(
                x0 + column as f64 * spacing_x,
                0.0,
                z0 - row as f64 * spacing_z,
            ),
        );
    }
    BuiltAsset { mesh, uvs: None }
}

const TALL_BUILDING_DESIGN_UNIT_M: f64 = 1.0;

fn tall_shade(color: [u8; 3], shade: f32) -> [u8; 3] {
    color.map(|channel| (f32::from(channel) * shade).round().clamp(0.0, 255.0) as u8)
}

fn tall_box(
    builder: &mut StageSurfaceBuilder,
    profile: &TallBuildingProfile,
    min: (i32, i32, i32),
    max: (i32, i32, i32),
) {
    building_box(builder, min, max, profile.frame);
}

fn tall_posts(
    builder: &mut StageSurfaceBuilder,
    profile: &TallBuildingProfile,
    xs: &[i32],
    zs: &[i32],
    top: i32,
) {
    for &x in xs {
        for &z in zs {
            tall_box(builder, profile, (x - 1, 0, z - 1), (x + 1, top, z + 1));
        }
    }
}

/// Author one canonical massing example for each structural/facade family.
/// These are comparative staging models, not landmark copies. Production
/// still gets its footprint, parts and height from source data.
fn build_tall_massing(
    builder: &mut StageSurfaceBuilder,
    profile: &TallBuildingProfile,
) -> ((i32, i32), i32) {
    match profile.massing {
        TallMassing::Tripartite => {
            tall_box(builder, profile, (-15, 0, -10), (15, 11, 10));
            tall_box(builder, profile, (-11, 12, -8), (11, 74, 8));
            tall_box(builder, profile, (-13, 75, -9), (13, 83, 9));
            ((16, 11), 84)
        }
        TallMassing::Setback => {
            tall_box(builder, profile, (-17, 0, -12), (17, 17, 12));
            tall_box(builder, profile, (-14, 18, -10), (14, 49, 10));
            tall_box(builder, profile, (-11, 50, -8), (11, 75, 8));
            tall_box(builder, profile, (-8, 76, -6), (8, 95, 6));
            tall_box(builder, profile, (-4, 96, -4), (4, 108, 4));
            ((18, 13), 109)
        }
        TallMassing::Slab => {
            tall_posts(builder, profile, &[-18, -6, 6, 18], &[-4, 4], 6);
            tall_box(builder, profile, (-22, 7, -7), (22, 84, 7));
            tall_box(builder, profile, (-18, 85, -6), (18, 88, 6));
            ((23, 8), 89)
        }
        TallMassing::Box => {
            tall_box(builder, profile, (-17, 0, -13), (17, 8, 13));
            tall_box(builder, profile, (-12, 9, -11), (12, 98, 11));
            tall_box(builder, profile, (-8, 99, -7), (8, 104, 7));
            ((18, 14), 105)
        }
        TallMassing::MonumentalConcrete => {
            tall_box(builder, profile, (-18, 0, -14), (18, 18, 14));
            tall_box(builder, profile, (-14, 19, -10), (14, 82, 10));
            tall_box(builder, profile, (-18, 27, -5), (-15, 72, 5));
            tall_box(builder, profile, (15, 27, -5), (18, 72, 5));
            tall_box(builder, profile, (-10, 83, -8), (10, 88, 8));
            ((19, 15), 89)
        }
        TallMassing::LoggiaSlab => {
            tall_posts(builder, profile, &[-12, -4, 4, 12], &[-4, 4], 5);
            tall_box(builder, profile, (-16, 6, -7), (16, 84, 7));
            tall_box(builder, profile, (-7, 85, -5), (7, 90, 5));
            ((18, 11), 91)
        }
        TallMassing::Pediment => {
            tall_box(builder, profile, (-15, 0, -11), (15, 13, 11));
            tall_box(builder, profile, (-11, 14, -8), (11, 94, 8));
            for side in [-1.0_f64, 1.0] {
                let rings = [(95., 2., 12.), (110., 6., 7.)].map(|(y, inner, outer)| {
                    let lo = (side * inner).min(side * outer);
                    let hi = (side * inner).max(side * outer);
                    vec![
                        DVec3::new(lo, y, -8.),
                        DVec3::new(lo, y, 9.),
                        DVec3::new(hi, y, 9.),
                        DVec3::new(hi, y, -8.),
                    ]
                });
                builder.loft(
                    &rings,
                    building_appearance(vearth_gen::material::WALL, profile.frame),
                );
            }
            ((16, 12), 110)
        }
        TallMassing::Tube => {
            tall_box(builder, profile, (-14, 0, -12), (14, 116, 12));
            tall_box(builder, profile, (-10, 117, -8), (10, 122, 8));
            ((15, 13), 123)
        }
        TallMassing::BundledTube => {
            let heights = [[78, 104, 78], [96, 139, 113], [78, 113, 78]];
            for (zi, &z) in [-9, 0, 9].iter().enumerate() {
                for (xi, &x) in [-9, 0, 9].iter().enumerate() {
                    tall_box(
                        builder,
                        profile,
                        (x - 4, 0, z - 4),
                        (x + 4, heights[zi][xi], z + 4),
                    );
                }
            }
            ((14, 14), 140)
        }
        TallMassing::TaperedDiagrid => {
            let rings =
                [(0., 8.), (41.48, 14.), (123., 7.)].map(|(y, r)| ellipse_ring(r, r, y, 0.));
            builder.loft(
                &rings,
                building_appearance(vearth_gen::material::WALL, profile.frame),
            );
            ((15, 15), 123)
        }
        TallMassing::Twist => {
            let rings = (0..=28)
                .map(|i| {
                    let t = i as f64 / 28.;
                    let (sn, cs) = (t * 70f64.to_radians()).sin_cos();
                    [(-12., -9.), (-12., 9.), (12., 9.), (12., -9.)]
                        .map(|(x, z)| DVec3::new(x * cs - z * sn, t * 113., x * sn + z * cs))
                        .to_vec()
                })
                .collect::<Vec<_>>();
            builder.loft(
                &rings,
                building_appearance(vearth_gen::material::WALL, profile.frame),
            );
            ((17, 17), 113)
        }
        TallMassing::Terraces => {
            tall_box(builder, profile, (-20, 0, -15), (20, 14, 15));
            tall_box(builder, profile, (-14, 15, -11), (14, 49, 11));
            tall_box(builder, profile, (-11, 50, -9), (14, 82, 9));
            tall_box(builder, profile, (-8, 83, -7), (12, 109, 7));
            tall_box(builder, profile, (-6, 110, -5), (9, 122, 5));
            ((21, 16), 123)
        }
        TallMassing::Buttressed => {
            tall_box(builder, profile, (-5, 0, -5), (5, 145, 5));
            for stage in 0..=6 {
                let bottom = (stage * 24) as f64;
                let top = ((stage + 1) * 24).min(146) as f64;
                let length = (19 - stage * 2).max(5) as f64;
                let width = if stage < 4 { 4. } else { 3. };
                for degrees in [90f64, 210., 330.] {
                    let (sn, cs) = degrees.to_radians().sin_cos();
                    let ring = |y| {
                        [(0., -width), (0., width), (length, width), (length, -width)]
                            .map(|(x, z)| DVec3::new(x * cs - z * sn, y, x * sn + z * cs))
                            .to_vec()
                    };
                    builder.loft(
                        &[ring(bottom), ring(top)],
                        building_appearance(vearth_gen::material::WALL, profile.frame),
                    );
                }
            }
            for (low, high, r) in [(146, 154, 3), (154, 160, 2), (160, 168, 1)] {
                tall_box(builder, profile, (-r, low, -r), (r, high, r));
            }
            ((23, 23), 169)
        }
        TallMassing::Cylinder => {
            for (bottom, top, radius) in [(0., 16., 11.5), (16., 97., 10.)] {
                builder.loft(
                    &[
                        ellipse_ring(radius, radius, bottom, 0.),
                        ellipse_ring(radius, radius, top, 0.),
                    ],
                    building_appearance(vearth_gen::material::WALL, profile.frame),
                );
            }
            ((14, 14), 97)
        }
        TallMassing::SCurve => {
            tall_box(builder, profile, (-14, 0, -17), (14, 7, 17));
            let ring = |y| {
                let mut p = (0..=32)
                    .map(|i| {
                        let z = -16. + i as f64;
                        DVec3::new((z / 16. * std::f64::consts::PI).sin() * 7. - 4.5, y, z)
                    })
                    .collect::<Vec<_>>();
                p.extend((0..=32).rev().map(|i| {
                    let z = -16. + i as f64;
                    DVec3::new((z / 16. * std::f64::consts::PI).sin() * 7. + 4.5, y, z)
                }));
                p
            };
            builder.loft(
                &[ring(8.), ring(89.)],
                building_appearance(vearth_gen::material::WALL, profile.frame),
            );
            ((15, 18), 89)
        }
        TallMassing::Rippled => {
            tall_box(builder, profile, (-15, 0, -12), (15, 10, 12));
            builder.loft(
                &[
                    ellipse_ring(10., 8., 11., 0.),
                    ellipse_ring(10., 8., 133., 0.),
                ],
                building_appearance(vearth_gen::material::WALL, profile.frame),
            );
            tall_box(builder, profile, (-6, 133, -5), (6, 138, 5));
            ((16, 14), 139)
        }
        TallMassing::Crescent => {
            let ring = |y: f64, r: f64| {
                let inner = r * 0.70;
                let offset = r * 0.43;
                let x = (r * r - inner * inner + offset * offset) / (2. * offset);
                let outer_angle = (x / r).acos();
                let inner_angle = ((x - offset) / inner).acos();
                let mut p = (0..=48)
                    .map(|i| {
                        let a = -outer_angle
                            - i as f64 / 48. * (std::f64::consts::TAU - 2. * outer_angle);
                        DVec3::new(r * a.cos(), y, r * a.sin())
                    })
                    .collect::<Vec<_>>();
                p.extend((1..48).map(|i| {
                    let a =
                        inner_angle + i as f64 / 48. * (std::f64::consts::TAU - 2. * inner_angle);
                    DVec3::new(offset + inner * a.cos(), y, inner * a.sin())
                }));
                p
            };
            builder.loft(
                &[ring(0., 16.), ring(159., 10.)],
                building_appearance(vearth_gen::material::WALL, profile.frame),
            );
            ((17, 17), 159)
        }
    }
}

fn apply_tall_facade(
    builder: &mut StageSurfaceBuilder,
    profile: &TallBuildingProfile,
    _half_bounds: (i32, i32),
    height: i32,
) {
    let floor_h = f64::from(profile.floor_m.max(3.));
    let bay = f64::from(profile.bay_m.max(2.));
    let mut windows = Vec::new();
    for face in &builder.faces {
        let [a, b, c, d] = face.points;
        let normal = (b - a).cross(d - a).normalize_or_zero();
        if normal.y.abs() > 0.75 {
            continue;
        }
        let low = a.y.min(b.y);
        let high = c.y.max(d.y);
        if high - low < 1. {
            continue;
        }
        let count = ((b - a).length() / bay).round().max(1.) as usize;
        let sample = |u: f64, y: f64| {
            let v = ((y - low) / (high - low)).clamp(0., 1.);
            a.lerp(b, u).lerp(d.lerp(c, u), v) + normal * 0.015
        };
        for floor in (low / floor_h).floor() as i32..(high / floor_h).ceil() as i32 {
            let base = floor as f64 * floor_h;
            let bands = if floor == 0 {
                (profile.base_x, profile.base_y)
            } else if base >= height as f64 - floor_h {
                (profile.middle_x, profile.crown_y)
            } else {
                (profile.middle_x, profile.middle_y)
            };
            let y0 = (base + floor_h * bands.1.0).max(low);
            let y1 = (base + floor_h * bands.1.1).min(high);
            if y1 <= y0 {
                continue;
            }
            if profile.mechanical_every > 0
                && floor > 1
                && (floor + 1) % profile.mechanical_every as i32 == 0
            {
                continue;
            }
            for column in 0..count {
                let u0 = (column as f64 + f64::from(bands.0.0)) / count as f64;
                let u1 = (column as f64 + f64::from(bands.0.1)) / count as f64;
                windows.push(StageFace {
                    points: [
                        sample(u0, y0),
                        sample(u1, y0),
                        sample(u1, y1),
                        sample(u0, y1),
                    ],
                    appearance: building_appearance(
                        vearth_gen::material::WINDOW,
                        tall_shade(
                            profile.glass,
                            0.94 + ((floor + column as i32).rem_euclid(3)) as f32 * 0.035,
                        ),
                    ),
                });
            }
        }
    }
    for (points, appearance) in &mut builder.triangles {
        if (points[1] - points[0]).cross(points[2] - points[0]).y > 0. {
            *appearance = building_appearance(vearth_gen::material::ROOF, profile.roof);
        }
    }
    builder.faces.extend(windows);
}

fn add_tall_details(builder: &mut StageSurfaceBuilder, profile: &TallBuildingProfile) {
    match profile.id {
        TallBuildingStyle::ResidentialLoggia => {
            // Concrete balcony slabs read as horizontal shade devices without
            // adding a dense railing grille in front of the facade.
            for y in (10..84).step_by(4) {
                building_box(builder, (-16, y, 8), (16, y, 10), profile.spandrel);
                building_box(builder, (-16, y, -10), (16, y, -8), profile.spandrel);
            }
        }
        TallBuildingStyle::TerracedMixedUse => {
            for (y, min_x, max_x, min_z, max_z) in [
                (15, -19, 19, -14, 14),
                (50, -13, 13, -10, 10),
                (83, -10, 13, -8, 8),
                (110, -7, 11, -6, 6),
            ] {
                building_roof_box(builder, (min_x, y, min_z), (max_x, y, max_z), profile.roof);
            }
        }
        TallBuildingStyle::ButtressedSupertall => {
            building_box_material(
                builder,
                (0, 169, 0),
                (0, 186, 0),
                vearth_gen::material::STEEL,
                profile.accent,
            );
        }
        TallBuildingStyle::CircularResidential | TallBuildingStyle::RippledBalcony => {
            let circular = profile.id == TallBuildingStyle::CircularResidential;
            let (start, end, step) = if circular { (18, 96, 3) } else { (14, 132, 4) };
            for y in (start..end).step_by(step) {
                let ring = |height: f64| {
                    (0..64)
                        .map(|i| {
                            let angle = -(i as f64) * std::f64::consts::TAU / 64.;
                            let radius = if circular {
                                12.2 + (angle * 8.).cos().abs() * 0.9
                            } else {
                                12. + (angle * 3. + y as f64 * 0.14).sin() * 2.2
                            };
                            DVec3::new(
                                radius * angle.cos(),
                                height,
                                radius * angle.sin() * if circular { 1. } else { 0.78 },
                            )
                        })
                        .collect::<Vec<_>>()
                };
                builder.loft(
                    &[ring(y as f64), ring(y as f64 + 0.65)],
                    building_appearance(vearth_gen::material::WALL, profile.accent),
                );
            }
        }
        _ => {}
    }
}

fn build_tall_building(style: TallBuildingStyle) -> BuiltAsset {
    let profile = tall_building_profile(style);
    let mut builder = StageSurfaceBuilder::default();
    let (half_bounds, height) = build_tall_massing(&mut builder, profile);
    apply_tall_facade(&mut builder, profile, half_bounds, height);
    add_tall_details(&mut builder, profile);
    let mesh = builder.architectural_mesh(TALL_BUILDING_DESIGN_UNIT_M as f32, DVec3::ZERO, None);
    BuiltAsset { mesh, uvs: None }
}

fn build_observation_tower() -> BuiltAsset {
    let profile = vearth_gen::observation_towers::observation_tower_profile(217.);
    let mut builder = StageSurfaceBuilder::default();
    let h = f64::from(profile.height_m);
    let mut section = |fractions: Vec<(f64, f64)>, material, tint| {
        let rings = fractions
            .into_iter()
            .map(|(t, r)| ellipse_ring(r, r, t * h, 0.))
            .collect::<Vec<_>>();
        builder.loft(&rings, building_appearance(material, tint));
    };
    let base = f64::from(profile.shaft_base_radius_m);
    let top = f64::from(profile.shaft_top_radius_m);
    let pod = f64::from(profile.pod_radius_m);
    section(
        vec![(0., base * 1.22), (0.045, base * 1.22)],
        vearth_gen::material::WALL,
        [151, 157, 158],
    );
    section(
        (0..=16)
            .map(|i| {
                let t = i as f64 / 16.;
                (
                    0.045 + t * 0.630,
                    base + (top - base) * (1. - (1. - t).powf(1.55)),
                )
            })
            .collect(),
        vearth_gen::material::WALL,
        [176, 179, 176],
    );
    for (a, b, mat, tint) in [
        (0., 0.25, vearth_gen::material::WALL, [176, 179, 176]),
        (0.25, 0.76, vearth_gen::material::WINDOW, [78, 122, 149]),
        (0.76, 1., vearth_gen::material::WALL, [176, 179, 176]),
    ] {
        section(
            (0..=8)
                .map(|i| {
                    let t = a + (b - a) * i as f64 / 8.;
                    (
                        0.675 + 0.09 * t,
                        pod * (0.88 + (t * std::f64::consts::PI).sin() * 0.12),
                    )
                })
                .collect(),
            mat,
            tint,
        );
    }
    section(
        vec![(0.765, pod * 1.10), (0.785, pod * 1.10)],
        vearth_gen::material::STEEL,
        [128, 139, 143],
    );
    section(
        vec![(0.785, top), (0.825, top)],
        vearth_gen::material::WALL,
        [176, 179, 176],
    );
    section(
        vec![(0.825, top), (1. - 1. / h, 0.7)],
        vearth_gen::material::STEEL,
        [128, 139, 143],
    );
    section(
        vec![(1. - 1. / h, 0.9), (1., 0.9)],
        vearth_gen::material::AVIATION_BEACON,
        [224, 24, 18],
    );
    BuiltAsset {
        mesh: builder.mesh(1., DVec3::ZERO),
        uvs: None,
    }
}

fn build_tall_building_catalogue() -> BuiltAsset {
    let columns = 4usize;
    let spacing_x = 58.0;
    let spacing_z = 58.0;
    let rows = (TALL_BUILDING_PROFILES.len() + 1).div_ceil(columns);
    let x0 = -((columns - 1) as f64) * spacing_x * 0.5;
    let z0 = (rows - 1) as f64 * spacing_z * 0.5;
    let mut mesh = MeshBuffers::default();
    for (index, profile) in TALL_BUILDING_PROFILES.iter().enumerate() {
        let asset = build_tall_building(profile.id);
        append_built_asset(
            &mut mesh,
            &asset,
            DVec3::new(
                x0 + (index % columns) as f64 * spacing_x,
                0.0,
                z0 - (index / columns) as f64 * spacing_z,
            ),
        );
    }
    let index = TALL_BUILDING_PROFILES.len();
    let observation_tower = build_observation_tower();
    append_built_asset(
        &mut mesh,
        &observation_tower,
        DVec3::new(
            x0 + (index % columns) as f64 * spacing_x,
            0.0,
            z0 - (index / columns) as f64 * spacing_z,
        ),
    );
    BuiltAsset { mesh, uvs: None }
}

fn append_built_asset(target: &mut MeshBuffers, asset: &BuiltAsset, centre: DVec3) {
    assert!(asset.uvs.is_none(), "catalogue assets must be untextured");
    let bounds = Bounds::from_mesh(&asset.mesh);
    append_mesh(
        target,
        &asset.mesh,
        DVec3::new(
            centre.x - bounds.centre().x,
            centre.y,
            centre.z - bounds.centre().z,
        ),
    );
}

fn build_tree(profile: TreeProfileId) -> BuiltAsset {
    let asset = vearth_gen::tier0::build_near_tree_mesh(profile);
    BuiltAsset {
        mesh: asset.mesh,
        uvs: None,
    }
}

#[derive(Clone, Copy)]
struct TreeGridColumn {
    label: &'static str,
    lod: u8,
}

const TREE_GRID_COLUMNS: [TreeGridColumn; 3] = [
    TreeGridColumn {
        label: "Coarse",
        lod: 2,
    },
    TreeGridColumn {
        label: "Middle",
        lod: 1,
    },
    TreeGridColumn {
        label: "Fine",
        lod: 0,
    },
];

#[derive(Clone, Copy)]
enum TreeColorTreatment {
    Base,
    Mix {
        target_linear: [f32; 3],
        amount: f32,
    },
    Multiply([f32; 3]),
}

#[derive(Clone, Copy)]
struct TreeSeasonVariant {
    label: &'static str,
    treatment: TreeColorTreatment,
}

#[derive(Clone)]
struct TreeGridInfo {
    columns: Vec<String>,
    rows: Vec<&'static str>,
}

fn tree_season_variants(profile: TreeProfileId) -> [TreeSeasonVariant; 2] {
    use FoliageSeasonClass as Season;
    use TreeColorTreatment::{Base, Mix, Multiply};

    let base = |label| TreeSeasonVariant {
        label,
        treatment: Base,
    };
    let mix = |label, target_linear, amount| TreeSeasonVariant {
        label,
        treatment: Mix {
            target_linear,
            amount,
        },
    };
    let multiply = |label, value| TreeSeasonVariant {
        label,
        treatment: Multiply(value),
    };

    match profile.foliage_traits().season {
        Season::EvergreenNeedle => [base("Summer"), multiply("Winter", [0.968, 0.986, 0.964])],
        Season::EvergreenBroadleaf => [
            multiply("Wet season", [1.02, 1.04, 0.98]),
            multiply("Dry season", [0.82, 0.90, 0.76]),
        ],
        Season::DeciduousYellow => [base("Summer"), mix("Autumn", [0.50, 0.29, 0.065], 0.82)],
        Season::DeciduousOrange => [base("Summer"), mix("Autumn", [0.62, 0.20, 0.055], 0.82)],
        Season::DeciduousRed => [base("Summer"), mix("Autumn", [0.42, 0.055, 0.028], 0.88)],
        Season::DeciduousConifer => [base("Summer"), mix("Autumn", [0.55, 0.32, 0.06], 0.90)],
        Season::DroughtDeciduous => [
            base("Wet season"),
            mix("Dry season", [0.34, 0.26, 0.105], 0.72),
        ],
        Season::SpringFlowering => [mix("Spring bloom", [1.0, 0.48, 0.66], 0.70), base("Summer")],
        Season::OrchardFlowering => [mix("Spring bloom", [1.0, 0.72, 0.78], 0.78), base("Summer")],
        Season::TropicalBroadleaf => [
            multiply("Wet season", [1.04, 1.08, 0.98]),
            multiply("Dry season", [0.78, 0.84, 0.65]),
        ],
        Season::Palm => [
            base("Wet season"),
            multiply("Dry season", [0.90, 0.88, 0.72]),
        ],
    }
}

fn srgb_u8_to_linear(value: u8) -> f32 {
    let value = f32::from(value) / 255.0;
    if value <= 0.040_45 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb_u8(value: f32) -> u8 {
    let value = value.clamp(0.0, 1.0);
    let srgb = if value <= 0.003_130_8 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    };
    (srgb * 255.0).round() as u8
}

fn apply_tree_color_treatment(mesh: &mut MeshBuffers, treatment: TreeColorTreatment) {
    for vertex in &mut mesh.vertices {
        let material = ((vertex.face_ao >> 5) & 0xff) as u8;
        if !vearth_gen::material::is_foliage(material) {
            continue;
        }

        let mut linear = [
            srgb_u8_to_linear(vertex.rgba[0]),
            srgb_u8_to_linear(vertex.rgba[1]),
            srgb_u8_to_linear(vertex.rgba[2]),
        ];
        match treatment {
            TreeColorTreatment::Base => {}
            TreeColorTreatment::Mix {
                target_linear,
                amount,
            } => {
                for channel in 0..3 {
                    linear[channel] += (target_linear[channel] - linear[channel]) * amount;
                }
            }
            TreeColorTreatment::Multiply(multiplier) => {
                for channel in 0..3 {
                    linear[channel] *= multiplier[channel];
                }
            }
        }
        for channel in 0..3 {
            vertex.rgba[channel] = linear_to_srgb_u8(linear[channel]);
        }

        // The seasonal result is baked for this comparison cell. Mapping the
        // semantic foliage material back to the shared leaf layer prevents
        // the single global weather phase from recolouring every row alike.
        vertex.face_ao =
            (vertex.face_ao & !(0xff << 5)) | (u32::from(vearth_gen::material::LEAVES) << 5);
    }
}

fn clone_mesh(mesh: &MeshBuffers) -> MeshBuffers {
    MeshBuffers {
        vertices: mesh.vertices.clone(),
        indices: mesh.indices.clone(),
    }
}

fn build_tree_grid(profile: TreeProfileId) -> (BuiltAsset, TreeGridInfo) {
    let variants = tree_season_variants(profile);
    let lod_assets =
        TREE_GRID_COLUMNS.map(|column| vearth_gen::tree_mesh::build_tree_mesh(profile, column.lod));
    let max_size = lod_assets
        .iter()
        .map(|asset| Bounds::from_mesh(&asset.mesh).size())
        .fold(DVec3::ZERO, DVec3::max);
    let x_spacing = max_size.x.max(max_size.z) * 1.35 + 3.0;
    let z_spacing = max_size.z * 1.55 + 4.0;
    let x0 = -((TREE_GRID_COLUMNS.len() - 1) as f64) * x_spacing * 0.5;
    let z0 = -((variants.len() - 1) as f64) * z_spacing * 0.5;
    let mut combined = MeshBuffers::default();

    for (row, variant) in variants.iter().enumerate() {
        for (column, asset) in lod_assets.iter().enumerate() {
            let mut mesh = clone_mesh(&asset.mesh);
            apply_tree_color_treatment(&mut mesh, variant.treatment);
            let bounds = Bounds::from_mesh(&mesh);
            append_mesh(
                &mut combined,
                &mesh,
                DVec3::new(
                    x0 + column as f64 * x_spacing - bounds.centre().x,
                    0.0,
                    z0 + row as f64 * z_spacing - bounds.centre().z,
                ),
            );
        }
    }

    (
        BuiltAsset {
            mesh: combined,
            uvs: None,
        },
        TreeGridInfo {
            columns: TREE_GRID_COLUMNS
                .iter()
                .zip(&lod_assets)
                .map(|(column, asset)| {
                    format!(
                        "{} · {} triangles",
                        column.label,
                        asset.mesh.indices.len() / 3
                    )
                })
                .collect(),
            rows: variants.iter().map(|variant| variant.label).collect(),
        },
    )
}

fn build_landmark(landmark: &'static vearth_gen::landmarks::Landmark) -> BuiltAsset {
    let mut mesh = vearth_gen::landmarks::canonical_mesh(landmark.name, 0)
        .expect("every catalog landmark has an authored surface");
    let centre_x = (landmark.bbox_min[0] + landmark.bbox_max[0]) * 0.5;
    let centre_z = (landmark.bbox_min[2] + landmark.bbox_max[2]) * 0.5;
    transform_mesh(&mut mesh, 1.0, DVec3::new(-centre_x, 0.0, -centre_z));
    BuiltAsset { mesh, uvs: None }
}

#[derive(Clone)]
struct StageFace {
    points: [DVec3; 4],
    appearance: SurfaceAppearance,
}

#[derive(Default)]
struct StageSurfaceBuilder {
    faces: Vec<StageFace>,
    triangles: Vec<([DVec3; 3], SurfaceAppearance)>,
}

fn ellipse_ring(rx: f64, rz: f64, y: f64, rotation: f64) -> Vec<DVec3> {
    (0..48)
        .map(|i| {
            let a = -(i as f64) * std::f64::consts::TAU / 48.;
            let (sn, cs) = rotation.sin_cos();
            let x = rx * a.cos();
            let z = rz * a.sin();
            DVec3::new(x * cs - z * sn, y, x * sn + z * cs)
        })
        .collect()
}

impl StageSurfaceBuilder {
    fn box_surface(&mut self, lo: DVec3, hi: DVec3, appearance: SurfaceAppearance) {
        if !lo.cmplt(hi).all() {
            return;
        }
        let ring = |y| {
            vec![
                DVec3::new(lo.x, y, lo.z),
                DVec3::new(lo.x, y, hi.z),
                DVec3::new(hi.x, y, hi.z),
                DVec3::new(hi.x, y, lo.z),
            ]
        };
        self.loft(&[ring(lo.y), ring(hi.y)], appearance);
    }
    fn loft(&mut self, rings: &[Vec<DVec3>], appearance: SurfaceAppearance) {
        if rings.len() < 2 || rings[0].len() < 3 {
            return;
        }
        let signed_area = |ring: &Vec<DVec3>| {
            ring.iter()
                .zip(ring.iter().cycle().skip(1))
                .take(ring.len())
                .map(|(a, b)| a.x * b.z - b.x * a.z)
                .sum::<f64>()
        };
        let reverse = rings
            .iter()
            .map(signed_area)
            .find(|area| area.abs() > 1e-10)
            .is_some_and(|area| area > 0.0);
        // Reverse every section together so twisted profiles keep their
        // corresponding edges while caps and walls share outward winding.
        let normalized;
        let rings = if reverse {
            normalized = rings
                .iter()
                .map(|ring| ring.iter().rev().copied().collect())
                .collect::<Vec<Vec<DVec3>>>();
            normalized.as_slice()
        } else {
            rings
        };
        for pair in rings.windows(2) {
            assert_eq!(pair[0].len(), pair[1].len());
            for i in 0..pair[0].len() {
                let j = (i + 1) % pair[0].len();
                self.faces.push(StageFace {
                    points: [pair[0][i], pair[0][j], pair[1][j], pair[1][i]],
                    appearance,
                });
            }
        }
        for (ring, reverse) in [
            (rings.first().unwrap(), true),
            (rings.last().unwrap(), false),
        ] {
            // Ear clipping keeps caps correct for the concave crescent and S profiles.
            let mut ids = (0..ring.len()).collect::<Vec<_>>();
            let mut guard = 0;
            while ids.len() > 2 && guard < ring.len() * ring.len() {
                guard += 1;
                let mut found = false;
                for at in 0..ids.len() {
                    let ia = ids[(at + ids.len() - 1) % ids.len()];
                    let ib = ids[at];
                    let ic = ids[(at + 1) % ids.len()];
                    let a = ring[ia];
                    let b = ring[ib];
                    let c = ring[ic];
                    let cross = |p: DVec3, q: DVec3, r: DVec3| {
                        (q.x - p.x) * (r.z - p.z) - (q.z - p.z) * (r.x - p.x)
                    };
                    if cross(a, b, c) >= -1e-10 {
                        continue;
                    }
                    if ids.iter().any(|&id| {
                        id != ia
                            && id != ib
                            && id != ic
                            && cross(a, b, ring[id]) < -1e-10
                            && cross(b, c, ring[id]) < -1e-10
                            && cross(c, a, ring[id]) < -1e-10
                    }) {
                        continue;
                    }
                    self.triangles
                        .push((if reverse { [c, b, a] } else { [a, b, c] }, appearance));
                    ids.remove(at);
                    found = true;
                    break;
                }
                if !found {
                    break;
                }
            }
        }
    }
    fn mesh(self, scale: f32, origin: DVec3) -> MeshBuffers {
        self.architectural_mesh(scale, origin, None)
    }
    fn architectural_mesh(
        self,
        scale: f32,
        origin: DVec3,
        style: Option<(ArchitectureRegion, RoofShape)>,
    ) -> MeshBuffers {
        let mut mesh = MeshBuffers::default();
        let mut emit = |points: [DVec3; 3], appearance: SurfaceAppearance| {
            let normal = (points[1] - points[0])
                .cross(points[2] - points[0])
                .normalize_or_zero();
            if normal.length_squared() < 0.5 {
                return;
            }
            // Openings overlay their authored trim plane; a small normal offset
            // prevents coplanar glass and frame surfaces from flickering.
            let points = if matches!(
                appearance.material,
                vearth_gen::material::WINDOW | vearth_gen::material::LIT_WINDOW
            ) {
                points.map(|point| point + normal * 0.006)
            } else {
                points
            };
            let material = match (style, appearance.material) {
                (Some((region, _)), vearth_gen::material::WALL) => stage_wall_material(region),
                (Some((region, roof)), vearth_gen::material::ROOF) => {
                    stage_roof_material(region, Some(roof))
                }
                (_, other) => other,
            };
            let payload = encode_surface_normal(
                2 | (3 << 3) | (u32::from(material) << 5),
                normal.as_vec3().to_array(),
            );
            let rgba = [
                appearance.tint[0],
                appearance.tint[1],
                appearance.tint[2],
                255,
            ];
            let first = mesh.vertices.len() as u32;
            mesh.vertices.extend(points.map(|p| SurfaceVertex {
                pos: (p * scale as f64 + origin).as_vec3().to_array(),
                face_ao: payload,
                rgba,
            }));
            mesh.indices.extend([first, first + 1, first + 2]);
        };
        for face in self.faces {
            let [a, b, c, d] = face.points;
            emit([a, b, c], face.appearance);
            emit([a, c, d], face.appearance);
        }
        for (triangle, appearance) in self.triangles {
            emit(triangle, appearance);
        }
        mesh
    }
}

fn repeat_asset(asset: &BuiltAsset, count: usize) -> (MeshBuffers, Vec<[f32; 2]>) {
    let count = count.max(1);
    let uvs = if let Some(source) = &asset.uvs {
        assert_eq!(source.len(), asset.mesh.vertices.len());
        source.repeat(count)
    } else {
        vec![[0.0; 2]; asset.mesh.vertices.len() * count]
    };
    let bounds = Bounds::from_mesh(&asset.mesh);
    let authored_centre = bounds.centre();
    if count <= 1 {
        let mut mesh = MeshBuffers::default();
        append_mesh(
            &mut mesh,
            &asset.mesh,
            DVec3::new(-authored_centre.x, 0.0, -authored_centre.z),
        );
        return (mesh, uvs);
    }
    let size = bounds.size();
    let columns = (count as f64).sqrt().ceil() as usize;
    let rows = count.div_ceil(columns);
    let spacing = size.x.max(size.z) * 1.18 + 2.0;
    let x0 = -(columns.saturating_sub(1) as f64) * spacing * 0.5;
    let z0 = -(rows.saturating_sub(1) as f64) * spacing * 0.5;
    let mut mesh = MeshBuffers::default();
    for index in 0..count {
        let column = index % columns;
        let row = index / columns;
        append_mesh(
            &mut mesh,
            &asset.mesh,
            DVec3::new(
                x0 + column as f64 * spacing - authored_centre.x,
                0.0,
                z0 + row as f64 * spacing - authored_centre.z,
            ),
        );
    }
    (mesh, uvs)
}

fn nice_grid_step(target: f64) -> f64 {
    let power = 10.0_f64.powf(target.max(0.1).log10().floor());
    let scaled = target / power;
    let unit = if scaled <= 1.0 {
        1.0
    } else if scaled <= 2.0 {
        2.0
    } else if scaled <= 5.0 {
        5.0
    } else {
        10.0
    };
    unit * power
}

fn ground_grid(bounds: Bounds) -> MeshBuffers {
    let size = bounds.size();
    let content_span = size.x.max(size.z).max(18.0);
    let span = (content_span * 4.0).max(80.0);
    let step = nice_grid_step(content_span / 24.0);
    let half_cells = ((span * 0.5 / step).ceil() as i32).clamp(20, 96);
    let mut mesh = MeshBuffers::default();
    for z in -half_cells..half_cells {
        for x in -half_cells..half_cells {
            let x0 = f64::from(x) * step;
            let x1 = f64::from(x + 1) * step;
            let z0 = f64::from(z) * step;
            let z1 = f64::from(z + 1) * step;
            let tint = if x == -1 || x == 0 {
                [88, 106, 122]
            } else if z == -1 || z == 0 {
                [105, 88, 75]
            } else if (x + z) & 1 == 0 {
                [91, 96, 99]
            } else {
                [79, 84, 87]
            };
            push_ground_quad(&mut mesh, x0, x1, z0, z1, tint);
        }
    }
    mesh
}

fn push_ground_quad(mesh: &mut MeshBuffers, x0: f64, x1: f64, z0: f64, z1: f64, tint: [u8; 3]) {
    let first = mesh.vertices.len() as u32;
    let face_ao = 2 | (3 << 3) | (u32::from(vearth_gen::material::PAVED) << 5);
    let vertex = |x: f64, z: f64| SurfaceVertex {
        pos: [x as f32, -0.025, z as f32],
        face_ao,
        rgba: [tint[0], tint[1], tint[2], 255],
    };
    mesh.vertices.extend([
        vertex(x0, z0),
        vertex(x1, z0),
        vertex(x1, z1),
        vertex(x0, z1),
    ]);
    mesh.indices
        .extend([first, first + 2, first + 1, first, first + 3, first + 2]);
}

fn stage_frame() -> (DVec3, [DVec3; 3], DMat4) {
    // Equatorial right-handed tangent frame: local x=east, y=up, z=south.
    // (east/up/north would be left-handed and invert triangle winding.)
    // Keeping the stage on the ellipsoid lets the production atmosphere and
    // lighting shaders work unchanged while no world cells are present.
    let origin = vearth_core::ellipsoid::geodetic_to_ecef(0.0, 0.0, 0.0);
    let east = DVec3::Y;
    let up = DVec3::X;
    let south = -DVec3::Z;
    let model = DMat4::from_cols(
        east.extend(0.0),
        up.extend(0.0),
        south.extend(0.0),
        origin.extend(1.0),
    );
    (origin, [east, up, south], model)
}

#[derive(Clone, Copy)]
struct StageCamera {
    azimuth: f64,
    elevation: f64,
    distance: f64,
    target: DVec3,
}

impl StageCamera {
    fn new(bounds: Bounds, options: &StageOptions) -> Self {
        let mut camera = Self {
            azimuth: options.azimuth_deg.to_radians(),
            elevation: options.elevation_deg.to_radians(),
            distance: 1.0,
            target: bounds.centre(),
        };
        camera.frame(bounds);
        if let Some(distance) = options.distance_m {
            camera.distance = distance;
        }
        camera.clamp();
        camera
    }

    fn frame(&mut self, bounds: Bounds) {
        let size = bounds.size();
        let radius = size.length().max(2.0) * 0.5;
        self.target = DVec3::new(
            bounds.centre().x,
            bounds.min.y + size.y * 0.46,
            bounds.centre().z,
        );
        self.distance = (radius / (DEFAULT_FOV_Y * 0.5).sin() * 1.18).max(4.0);
        self.clamp();
    }

    fn clamp(&mut self) {
        self.elevation = self
            .elevation
            .clamp(-5.0_f64.to_radians(), 89.0_f64.to_radians());
        self.distance = self.distance.max(0.5);
    }

    fn orbit(&mut self, dx: f64, dy: f64) {
        self.azimuth -= dx * 0.008;
        self.elevation += dy * 0.008;
        self.clamp();
    }

    fn zoom(&mut self, amount: f64) {
        self.distance *= (-amount * 0.12).exp();
        self.clamp();
    }

    fn pan(&mut self, forward: f64, right: f64, elapsed_s: f64) {
        let (sin_az, cos_az) = self.azimuth.sin_cos();
        let forward_axis = DVec3::new(-sin_az, 0.0, -cos_az);
        let right_axis = DVec3::new(cos_az, 0.0, -sin_az);
        let direction = (forward_axis * forward + right_axis * right).normalize_or_zero();
        let speed_mps = (self.distance * 0.55).clamp(0.75, 250.0);
        self.target += direction * speed_mps * elapsed_s;
    }

    fn preset(&mut self, azimuth_deg: f64, elevation_deg: f64) {
        self.azimuth = azimuth_deg.to_radians();
        self.elevation = elevation_deg.to_radians();
        self.clamp();
    }

    fn pose(self) -> (DVec3, DMat4) {
        let (_, axes, _) = stage_frame();
        let (sin_az, cos_az) = self.azimuth.sin_cos();
        let (sin_el, cos_el) = self.elevation.sin_cos();
        let eye_local =
            self.target + DVec3::new(sin_az * cos_el, sin_el, cos_az * cos_el) * self.distance;
        let origin = stage_frame().0;
        let eye = origin + axes[0] * eye_local.x + axes[1] * eye_local.y + axes[2] * eye_local.z;
        let target =
            origin + axes[0] * self.target.x + axes[1] * self.target.y + axes[2] * self.target.z;
        let forward = (target - eye).normalize();
        let camera_up = if self.elevation > 86.0_f64.to_radians() {
            axes[2]
        } else {
            axes[1]
        };
        (
            eye,
            glam::dcamera::rh::view::look_to_mat4(DVec3::ZERO, forward, camera_up),
        )
    }
}

struct StageScene {
    bounds: Bounds,
    draw: PlayerDraw,
    triangles: usize,
    tree_grid: Option<TreeGridInfo>,
}

impl StageScene {
    fn build(renderer: &mut Renderer, descriptor: &StageDescriptor, instances: usize) -> Self {
        let (staged, staged_uvs, tree_grid) = match descriptor.source {
            StageSource::Tree(profile) => {
                let (asset, info) = build_tree_grid(profile);
                let uvs = vec![[0.0; 2]; asset.mesh.vertices.len()];
                (asset.mesh, uvs, Some(info))
            }
            _ => {
                let asset = build_asset(descriptor);
                let (staged, uvs) = repeat_asset(&asset, instances);
                (staged, uvs, None)
            }
        };
        let bounds = Bounds::from_mesh(&staged);
        let grid = ground_grid(bounds);
        let grid_uvs = vec![[0.0; 2]; grid.vertices.len()];
        let triangles = staged.indices.len() / 3;
        renderer.upload_player_parts_with_uv(&[grid, staged], &[grid_uvs, staged_uvs]);
        renderer.write_vehicle_textures();
        let (_, axes, model) = stage_frame();
        Self {
            bounds,
            draw: PlayerDraw {
                model,
                axes,
                parts: vec![0, 1],
                effects: None,
            },
            triangles,
            tree_grid,
        }
    }
}

struct StageUi {
    ctx: egui::Context,
    winit_state: egui_winit::State,
    renderer: egui_wgpu::Renderer,
    pending_textures: crate::ui::PendingTextureDeltas,
}

#[derive(Default)]
struct StageUiAction {
    selected: Option<usize>,
    instances: Option<usize>,
    frame: bool,
    preset: Option<(f64, f64)>,
    grid_visible: Option<bool>,
}

struct StageUiPaint {
    primitives: Vec<egui::ClippedPrimitive>,
    pixels_per_point: f32,
}

struct StageUiInfo<'a> {
    descriptor: &'a StageDescriptor,
    instances: usize,
    grid_visible: bool,
    triangles: usize,
    tree_grid: Option<&'a TreeGridInfo>,
    camera: StageCamera,
}

impl StageUi {
    fn new(window: &Window, device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let ctx = egui::Context::default();
        crate::ui::configure_fonts(&ctx);
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = egui::Color32::from_rgb(18, 21, 22);
        visuals.window_fill = egui::Color32::from_rgba_unmultiplied(18, 21, 22, 248);
        visuals.selection.bg_fill = egui::Color32::from_rgb(194, 220, 92);
        visuals.selection.stroke.color = egui::Color32::from_rgb(12, 15, 12);
        visuals.widgets.active.bg_fill = egui::Color32::from_rgb(194, 220, 92);
        ctx.set_visuals(visuals);
        let winit_state = egui_winit::State::new(
            ctx.clone(),
            egui::ViewportId::ROOT,
            window,
            Some(window.scale_factor() as f32),
            None,
            None,
        );
        let renderer =
            egui_wgpu::Renderer::new(device, format, egui_wgpu::RendererOptions::default());
        Self {
            ctx,
            winit_state,
            renderer,
            pending_textures: crate::ui::PendingTextureDeltas::default(),
        }
    }

    fn on_event(&mut self, window: &Window, event: &WindowEvent) -> bool {
        self.winit_state.on_window_event(window, event).consumed
    }

    fn prepare(
        &mut self,
        window: &Window,
        catalog: &[StageDescriptor],
        selected: usize,
        info: StageUiInfo<'_>,
    ) -> (StageUiAction, StageUiPaint) {
        let input = self.winit_state.take_egui_input(window);
        let mut action = StageUiAction::default();
        let output = self.ctx.run_ui(input, |ctx| {
            egui::Panel::right("stage controls")
                .exact_size(292.0)
                .show(ctx, |ui| {
                    ui.add_space(10.0);
                    ui.label(
                        egui::RichText::new("OBJECT STAGE")
                            .size(18.0)
                            .family(crate::ui::plex_medium_family())
                            .strong()
                            .color(egui::Color32::from_rgb(222, 235, 187)),
                    );
                    ui.label(
                        egui::RichText::new("NO WORLD STREAMING")
                            .monospace()
                            .size(9.0)
                            .color(egui::Color32::from_rgb(154, 184, 83)),
                    );
                    ui.add_space(12.0);
                    ui.label(egui::RichText::new("ASSET").monospace().size(10.0).weak());
                    let mut candidate = selected;
                    egui::ComboBox::from_id_salt("stage asset")
                        .selected_text(format!(
                            "{} · {}",
                            info.descriptor.category, info.descriptor.label
                        ))
                        .width(260.0)
                        .show_ui(ui, |ui| {
                            egui::ScrollArea::vertical()
                                .max_height(430.0)
                                .show(ui, |ui| {
                                    for (index, descriptor) in catalog.iter().enumerate() {
                                        ui.selectable_value(
                                            &mut candidate,
                                            index,
                                            format!(
                                                "{} · {}",
                                                descriptor.category, descriptor.label
                                            ),
                                        );
                                    }
                                });
                        });
                    if candidate != selected {
                        action.selected = Some(candidate);
                    }

                    ui.add_space(12.0);
                    ui.label(egui::RichText::new("LAYOUT").monospace().size(10.0).weak());
                    if let Some(tree_grid) = info.tree_grid {
                        ui.label(
                            egui::RichText::new("Columns · LOD · orange axis")
                                .size(10.0)
                                .weak(),
                        );
                        ui.horizontal_wrapped(|ui| {
                            for column in &tree_grid.columns {
                                ui.monospace(column);
                            }
                        });
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new("Rows · season · blue axis")
                                .size(10.0)
                                .weak(),
                        );
                        ui.horizontal_wrapped(|ui| {
                            for row in &tree_grid.rows {
                                ui.monospace(*row);
                            }
                        });
                    } else {
                        ui.horizontal(|ui| {
                            ui.label("Copies");
                            for count in [1, 4, 9, 16] {
                                if ui
                                    .selectable_label(info.instances == count, count.to_string())
                                    .clicked()
                                {
                                    action.instances = Some(count);
                                }
                            }
                        });
                    }
                    let mut grid_visible = info.grid_visible;
                    if ui.checkbox(&mut grid_visible, "Ground grid").changed() {
                        action.grid_visible = Some(grid_visible);
                    }

                    ui.add_space(12.0);
                    ui.label(egui::RichText::new("CAMERA").monospace().size(10.0).weak());
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("Front").clicked() {
                            action.preset = Some((0.0, 12.0));
                        }
                        if ui.button("Right").clicked() {
                            action.preset = Some((90.0, 12.0));
                        }
                        if ui.button("Back").clicked() {
                            action.preset = Some((180.0, 12.0));
                        }
                        if ui.button("Left").clicked() {
                            action.preset = Some((270.0, 12.0));
                        }
                        if ui.button("Top").clicked() {
                            action.preset = Some((0.0, 89.0));
                        }
                    });
                    if ui.button("Frame object  (F)").clicked() {
                        action.frame = true;
                    }

                    ui.add_space(14.0);
                    ui.separator();
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new("INSPECTION")
                            .monospace()
                            .size(10.0)
                            .weak(),
                    );
                    egui::Grid::new("stage stats")
                        .num_columns(2)
                        .spacing([14.0, 4.0])
                        .show(ui, |ui| {
                            if let Some(tree_grid) = info.tree_grid {
                                ui.label("Grid cells");
                                ui.monospace(
                                    (tree_grid.columns.len() * tree_grid.rows.len()).to_string(),
                                );
                                ui.end_row();
                            }
                            ui.label("Triangles");
                            ui.monospace(format_count(info.triangles));
                            ui.end_row();
                            ui.label("Azimuth");
                            ui.monospace(format!(
                                "{:.0}°",
                                info.camera.azimuth.to_degrees().rem_euclid(360.0)
                            ));
                            ui.end_row();
                            ui.label("Elevation");
                            ui.monospace(format!("{:.0}°", info.camera.elevation.to_degrees()));
                            ui.end_row();
                            ui.label("Distance");
                            ui.monospace(format!("{:.1} m", info.camera.distance));
                            ui.end_row();
                        });

                    ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                        ui.add_space(10.0);
                        ui.label(
                            egui::RichText::new(
                                "WASD to pan · drag to orbit · scroll to zoom · 1–5 views",
                            )
                            .size(10.0)
                            .weak(),
                        );
                        ui.label(
                            egui::RichText::new(&info.descriptor.id)
                                .monospace()
                                .size(10.0)
                                .weak(),
                        );
                    });
                });
        });
        self.winit_state
            .handle_platform_output(window, output.platform_output);
        let primitives = self.ctx.tessellate(output.shapes, output.pixels_per_point);
        self.pending_textures.append(output.textures_delta);
        (
            action,
            StageUiPaint {
                primitives,
                pixels_per_point: output.pixels_per_point,
            },
        )
    }

    fn paint(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::TextureView,
        width: u32,
        height: u32,
        frame: StageUiPaint,
    ) {
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [width, height],
            pixels_per_point: frame.pixels_per_point,
        };
        let textures_delta = self.pending_textures.take();
        for (id, delta) in &textures_delta.set {
            self.renderer.update_texture(device, queue, *id, delta);
        }
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("stage egui"),
        });
        let command_buffers =
            self.renderer
                .update_buffers(device, queue, &mut encoder, &frame.primitives, &screen);
        {
            let render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("stage egui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });
            let mut render_pass = render_pass.forget_lifetime();
            self.renderer
                .render(&mut render_pass, &frame.primitives, &screen);
        }
        queue.submit(command_buffers.into_iter().chain([encoder.finish()]));
        for id in &textures_delta.free {
            self.renderer.free_texture(id);
        }
    }
}

fn format_count(value: usize) -> String {
    if value >= 1_000_000 {
        format!("{:.1}m", value as f64 / 1_000_000.0)
    } else if value >= 1_000 {
        format!("{:.1}k", value as f64 / 1_000.0)
    } else {
        value.to_string()
    }
}

struct State {
    window: Arc<Window>,
    gfx: Gfx,
    ui: StageUi,
    catalog: Vec<StageDescriptor>,
    selected: usize,
    instances: usize,
    grid_visible: bool,
    scene: StageScene,
    camera: StageCamera,
    mouse_down: bool,
    cursor: Option<(f64, f64)>,
    pan_keys: HashSet<KeyCode>,
    last_frame: Instant,
    solar_utc_seconds: f64,
}

impl State {
    fn new(window: Arc<Window>, mut gfx: Gfx, options: StageOptions) -> Self {
        gfx.renderer.set_clouds_visible(false);
        gfx.renderer.set_precipitation_visible(false);
        gfx.renderer.set_building_night_lights_enabled(false);
        let catalog = stage_catalog();
        let selected = catalog
            .iter()
            .position(|descriptor| descriptor.id == options.asset)
            .unwrap_or(0);
        let scene = StageScene::build(&mut gfx.renderer, &catalog[selected], options.instances);
        let camera = StageCamera::new(scene.bounds, &options);
        let ui = StageUi::new(&window, &gfx.renderer.device, gfx.view_format());
        Self {
            window,
            gfx,
            ui,
            catalog,
            selected,
            instances: options.instances,
            grid_visible: true,
            scene,
            camera,
            mouse_down: false,
            cursor: None,
            pan_keys: HashSet::new(),
            last_frame: Instant::now(),
            solar_utc_seconds: options.solar_utc_seconds(),
        }
    }

    fn rebuild(&mut self, frame: bool) {
        self.scene = StageScene::build(
            &mut self.gfx.renderer,
            &self.catalog[self.selected],
            self.instances,
        );
        if frame {
            self.camera.frame(self.scene.bounds);
        }
    }

    fn apply_ui(&mut self, action: StageUiAction) -> bool {
        let mut redraw = false;
        if let Some(selected) = action.selected {
            self.selected = selected;
            self.rebuild(true);
            redraw = true;
        }
        if let Some(instances) = action.instances
            && instances != self.instances
        {
            self.instances = instances;
            self.rebuild(true);
            redraw = true;
        }
        if let Some(visible) = action.grid_visible {
            self.grid_visible = visible;
            redraw = true;
        }
        if action.frame {
            self.camera.frame(self.scene.bounds);
            redraw = true;
        }
        if let Some((azimuth, elevation)) = action.preset {
            self.camera.preset(azimuth, elevation);
            redraw = true;
        }
        redraw
    }

    fn frame(&mut self) {
        let now = Instant::now();
        let elapsed_s = now.duration_since(self.last_frame).as_secs_f64().min(0.05);
        self.last_frame = now;
        let forward = f64::from(self.pan_keys.contains(&KeyCode::KeyW))
            + f64::from(self.pan_keys.contains(&KeyCode::ArrowUp))
            - f64::from(self.pan_keys.contains(&KeyCode::KeyS))
            - f64::from(self.pan_keys.contains(&KeyCode::ArrowDown));
        let right = f64::from(self.pan_keys.contains(&KeyCode::KeyD))
            + f64::from(self.pan_keys.contains(&KeyCode::ArrowRight))
            - f64::from(self.pan_keys.contains(&KeyCode::KeyA))
            - f64::from(self.pan_keys.contains(&KeyCode::ArrowLeft));
        if forward != 0.0 || right != 0.0 {
            self.camera.pan(forward, right, elapsed_s);
            self.window.request_redraw();
        }

        let info = StageUiInfo {
            descriptor: &self.catalog[self.selected],
            instances: self.instances,
            grid_visible: self.grid_visible,
            triangles: self.scene.triangles,
            tree_grid: self.scene.tree_grid.as_ref(),
            camera: self.camera,
        };
        let (action, ui_paint) = self
            .ui
            .prepare(&self.window, &self.catalog, self.selected, info);
        let redraw = self.apply_ui(action);
        self.scene.draw.parts = if self.grid_visible {
            vec![0, 1]
        } else {
            vec![1]
        };
        let (cam_pos, view) = self.camera.pose();
        let solar = SolarClock::new(SolarStart::At(self.solar_utc_seconds), None);
        let lighting = solar.lighting();
        let weather = WeatherModel::new(WeatherConfig {
            mode: WeatherMode::Clear,
            ..WeatherConfig::default()
        })
        .render_state(cam_pos, solar.utc_seconds());
        let result = (|| -> Result<(), crate::gfx::SurfaceFrameError> {
            let target = self.gfx.begin_frame()?;
            self.gfx.render_world(
                &target,
                cam_pos,
                view,
                DEFAULT_FOV_Y,
                &[],
                lighting,
                weather,
                Some(&self.scene.draw),
            );
            self.ui.paint(
                &self.gfx.renderer.device,
                &self.gfx.renderer.queue,
                &target.view,
                self.gfx.config.width,
                self.gfx.config.height,
                ui_paint,
            );
            target.present(&self.gfx.renderer.queue);
            Ok(())
        })();
        match result {
            Ok(()) => {}
            Err(crate::gfx::SurfaceFrameError::Lost | crate::gfx::SurfaceFrameError::Outdated) => {
                self.gfx
                    .resize(self.gfx.config.width, self.gfx.config.height);
                self.window.request_redraw();
            }
            Err(crate::gfx::SurfaceFrameError::Timeout) => {
                log::warn!("stage surface timeout");
            }
            Err(crate::gfx::SurfaceFrameError::Occluded) => {}
            Err(crate::gfx::SurfaceFrameError::Validation) => {
                log::error!("stage surface validation error");
            }
        }
        if redraw {
            self.window.request_redraw();
        }
    }
}

pub struct StageApp {
    options: StageOptions,
    state: Option<State>,
}

impl ApplicationHandler for StageApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title(WINDOW_TITLE)
                        .with_inner_size(winit::dpi::LogicalSize::new(1280, 800)),
                )
                .expect("stage window"),
        );
        let gfx = pollster::block_on(Gfx::new(&crate::host::Window::new(window.clone())))
            .expect("stage GPU init");
        self.state = Some(State::new(window.clone(), gfx, self.options.clone()));
        window.request_redraw();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        let ui_consumed = state.ui.on_event(&state.window, &event);
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                state.gfx.resize(size.width, size.height);
                state.window.request_redraw();
            }
            WindowEvent::Focused(false) => {
                state.pan_keys.clear();
            }
            WindowEvent::MouseInput {
                state: button_state,
                button: MouseButton::Left,
                ..
            } => {
                // A release over the inspector must still terminate an orbit
                // that began over the viewport.
                if button_state == ElementState::Released {
                    state.mouse_down = false;
                } else if !ui_consumed {
                    state.mouse_down = true;
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let current = (position.x, position.y);
                if !ui_consumed
                    && state.mouse_down
                    && let Some(previous) = state.cursor
                {
                    state
                        .camera
                        .orbit(current.0 - previous.0, current.1 - previous.1);
                }
                state.cursor = Some(current);
                state.window.request_redraw();
            }
            WindowEvent::MouseWheel { delta, .. } if !ui_consumed => {
                let amount = match delta {
                    MouseScrollDelta::LineDelta(_, y) => f64::from(y),
                    MouseScrollDelta::PixelDelta(point) => point.y / 60.0,
                };
                state.camera.zoom(amount);
                state.window.request_redraw();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let PhysicalKey::Code(code) = event.physical_key else {
                    return;
                };
                if matches!(
                    code,
                    KeyCode::KeyW
                        | KeyCode::KeyA
                        | KeyCode::KeyS
                        | KeyCode::KeyD
                        | KeyCode::ArrowUp
                        | KeyCode::ArrowLeft
                        | KeyCode::ArrowDown
                        | KeyCode::ArrowRight
                ) {
                    if event.state == ElementState::Released {
                        state.pan_keys.remove(&code);
                    } else if !ui_consumed {
                        state.pan_keys.insert(code);
                    }
                    state.window.request_redraw();
                    return;
                }
                if ui_consumed || event.state != ElementState::Pressed {
                    return;
                }
                match PhysicalKey::Code(code) {
                    PhysicalKey::Code(KeyCode::Escape) => event_loop.exit(),
                    PhysicalKey::Code(KeyCode::KeyF) => state.camera.frame(state.scene.bounds),
                    PhysicalKey::Code(KeyCode::Digit1) => state.camera.preset(0.0, 12.0),
                    PhysicalKey::Code(KeyCode::Digit2) => state.camera.preset(90.0, 12.0),
                    PhysicalKey::Code(KeyCode::Digit3) => state.camera.preset(180.0, 12.0),
                    PhysicalKey::Code(KeyCode::Digit4) => state.camera.preset(270.0, 12.0),
                    PhysicalKey::Code(KeyCode::Digit5) => state.camera.preset(0.0, 89.0),
                    _ => return,
                }
                state.window.request_redraw();
            }
            WindowEvent::RedrawRequested => state.frame(),
            _ => {
                if ui_consumed {
                    state.window.request_redraw();
                }
            }
        }
    }
}

pub fn run_native(options: StageOptions) {
    let event_loop = EventLoop::new().expect("stage event loop");
    event_loop
        .run_app(&mut StageApp {
            options,
            state: None,
        })
        .expect("run stage");
}

/// Render the exact staging scene offscreen. This path shares asset builders,
/// layout, lighting and camera math with the interactive workspace.
pub fn run_screenshot(
    out: &str,
    width: u32,
    height: u32,
    options: StageOptions,
) -> Result<(), Box<dyn std::error::Error>> {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))?;
    let required_features = crate::gfx::renderer_features(
        adapter.features(),
        adapter.get_downlevel_capabilities().flags,
    );
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("vearth stage headless"),
        required_features,
        ..Default::default()
    }))?;
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let mut renderer = Renderer::new(device, queue, format);
    renderer.set_clouds_visible(false);
    renderer.set_precipitation_visible(false);
    renderer.set_building_night_lights_enabled(false);

    let catalog = stage_catalog();
    let descriptor = resolve_descriptor(&catalog, &options.asset)
        .ok_or_else(|| format!("unknown stage asset {:?}", options.asset))?;
    let scene = StageScene::build(&mut renderer, descriptor, options.instances);
    let camera = StageCamera::new(scene.bounds, &options);
    let (cam_pos, view) = camera.pose();
    let solar = SolarClock::new(SolarStart::At(options.solar_utc_seconds()), None);
    let lighting = solar.lighting();
    let weather = WeatherModel::new(WeatherConfig {
        mode: WeatherMode::Clear,
        ..WeatherConfig::default()
    })
    .render_state(cam_pos, solar.utc_seconds());

    let color = renderer.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("stage offscreen"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let color_view = color.create_view(&Default::default());
    let depth = create_depth(&renderer.device, width, height);
    let stats = renderer.render(
        &color_view,
        &depth,
        width,
        height,
        cam_pos,
        view,
        DEFAULT_FOV_Y,
        &[],
        lighting,
        weather,
        Some(&scene.draw),
    );
    log::info!(
        "stage screenshot: {} · {} instance(s) · {} triangles",
        descriptor.id,
        options.instances,
        stats.triangles,
    );

    let bytes_per_row = (width * 4).div_ceil(256) * 256;
    let readback = renderer.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("stage readback"),
        size: u64::from(bytes_per_row) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = renderer.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &color,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    renderer.queue.submit([encoder.finish()]);
    let slice = readback.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = tx.send(result);
    });
    renderer.device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(Duration::from_secs(30)),
    })?;
    rx.recv()??;
    let mapped = slice.get_mapped_range()?;
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for row in 0..height {
        let start = (row * bytes_per_row) as usize;
        rgba.extend_from_slice(&mapped[start..start + (width * 4) as usize]);
    }
    drop(mapped);
    let file = std::fs::File::create(out)?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&rgba)?;
    log::info!("stage screenshot: wrote {out}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn construction_studies_cover_physical_layers_at_identical_scale() {
        let catalog = stage_catalog();
        let mut layers = HashSet::new();
        let mut reference = None;
        for (slug, _, material, tint) in construction_finish_studies() {
            let asset =
                build_asset(resolve_descriptor(&catalog, &format!("materials:{slug}")).unwrap());
            layers.insert(crate::atlas::material_traits(material).atlas_layer);
            let positions = asset
                .mesh
                .vertices
                .iter()
                .map(|v| v.pos)
                .collect::<Vec<_>>();
            if let Some((positions_before, indices_before)) = &reference {
                assert_eq!(&positions, positions_before);
                assert_eq!(&asset.mesh.indices, indices_before);
            } else {
                reference = Some((positions, asset.mesh.indices.clone()));
            }
            for vertex in asset.mesh.vertices {
                assert_eq!((vertex.face_ao >> 5) & 255, u32::from(material));
                assert_eq!(&vertex.rgba[..3], &tint);
            }
        }
        assert_eq!(layers, (23..=38).collect());
    }

    #[test]
    fn construction_variants_keep_the_default_geometry_and_reach_packed_vertices() {
        let catalog = stage_catalog();
        let baseline = build_construction_finish(SurfaceAppearance {
            material: vearth_gen::material::TILE_ROOF,
            tint: [100; 3],
        });
        for (slug, _, material, tint) in construction_variant_studies() {
            let asset =
                build_asset(resolve_descriptor(&catalog, &format!("materials:{slug}")).unwrap());
            assert_eq!(asset.mesh.indices, baseline.mesh.indices);
            for (actual, expected) in asset.mesh.vertices.iter().zip(&baseline.mesh.vertices) {
                assert_eq!(actual.pos, expected.pos);
                assert_eq!((actual.face_ao >> 5) & 255, u32::from(material));
                assert_eq!((actual.face_ao >> 13) & 3, 1);
                assert_eq!(&actual.rgba[..3], &tint);
                assert_eq!(
                    vearth_core::mesh::PackedSurfaceVertex::try_from_full(*actual)
                        .unwrap()
                        .to_full(),
                    *actual
                );
            }
        }
    }

    #[test]
    fn road_closeups_hold_geometry_fixed_across_material_comparisons() {
        let catalog = stage_catalog();
        for (slug, _, material, _) in road_finish_studies() {
            let after =
                build_asset(resolve_descriptor(&catalog, &format!("roads:{slug}")).unwrap());
            let before =
                build_asset(resolve_descriptor(&catalog, &format!("roads:{slug}:before")).unwrap());
            assert_eq!(after.mesh.indices, before.mesh.indices);
            assert_eq!(after.mesh.indices.len(), 6);
            for (a, b) in after.mesh.vertices.iter().zip(&before.mesh.vertices) {
                assert_eq!(a.pos, b.pos);
                assert_eq!((a.face_ao >> 5) & 255, u32::from(material));
                assert_eq!(
                    (b.face_ao >> 5) & 255,
                    u32::from(vearth_gen::material::PATH)
                );
            }
        }
    }

    #[test]
    fn local_paving_previews_use_the_production_resolved_appearance() {
        let catalog = stage_catalog();
        let samples = vearth_gen::road_materials::local_samples();
        assert!(!samples.is_empty());
        let overview = build_asset(resolve_descriptor(&catalog, "roads:places").unwrap());
        assert_eq!(overview.mesh.indices.len(), samples.len() * 6);
        for (index, sample) in samples.iter().enumerate() {
            let descriptor = resolve_descriptor(&catalog, &format!("roads:place:{index}")).unwrap();
            assert!(descriptor.label.contains(sample.label));
            let asset = build_asset(descriptor);
            for vertex in asset.mesh.vertices {
                assert_eq!(
                    (vertex.face_ao >> 5) & 255,
                    u32::from(sample.appearance.material)
                );
                assert_eq!(&vertex.rgba[..3], &sample.appearance.tint);
            }
        }
    }

    #[test]
    fn production_highrise_preview_uses_live_panel_materials() {
        for usage in [
            vearth_data::vectors::BuildingUse::Commercial,
            vearth_data::vectors::BuildingUse::Residential,
        ] {
            let asset = build_generated_buildings(
                40.75_f64.to_radians(),
                -73.98_f64.to_radians(),
                Some(usage),
            );
            assert!(
                asset
                    .mesh
                    .vertices
                    .iter()
                    .any(|v| vearth_gen::highrise::is_facade(((v.face_ao >> 5) & 255) as u8))
            );
            let bounds = Bounds::from_mesh(&asset.mesh);
            assert!(
                (128.3..128.5).contains(&bounds.size().y),
                "production bounds {:?}",
                bounds.size()
            );
        }
    }

    #[test]
    fn local_studies_use_valid_production_geometry_and_special_materials() {
        for (key, required_material) in [
            ("alberobello-trulli", vearth_gen::material::SLATE_ROOF),
            ("sibiu-eyebrow", vearth_gen::material::TILE_ROOF),
            ("shirakawa-ogimachi", vearth_gen::material::THATCH_ROOF),
            ("kurashiki-bikan", vearth_gen::material::NAMAKO_WALL),
        ] {
            let place = vearth_gen::architecture_places::places()
                .iter()
                .find(|p| p.style_key == key)
                .unwrap();
            let asset = build_place_buildings(place);
            assert!(!asset.mesh.indices.is_empty(), "{key}");
            assert!(
                asset
                    .mesh
                    .vertices
                    .iter()
                    .any(|v| ((v.face_ao >> 5) & 255) == u32::from(required_material)),
                "{key}"
            );
            assert!(
                asset
                    .mesh
                    .vertices
                    .iter()
                    .all(|v| v.pos.iter().all(|x| x.is_finite()))
            );
            let bounds = Bounds::from_mesh(&asset.mesh);
            assert!(bounds.size().y <= 8.6, "{key}: height {:?}", bounds.size());
            if key == "alberobello-trulli" {
                let normals: HashSet<_> = asset
                    .mesh
                    .vertices
                    .iter()
                    .filter(|v| ((v.face_ao >> 5) & 255) == u32::from(required_material))
                    .map(|v| v.face_ao >> 16)
                    .collect();
                assert!(
                    normals.len() >= 12,
                    "production cone lost its faces: {}",
                    normals.len()
                );
            }
        }
    }

    #[test]
    fn catalog_resolves_prefixed_and_unique_short_names() {
        let catalog = stage_catalog();
        assert_eq!(
            resolve_descriptor(&catalog, "all-houses").unwrap().id,
            "buildings:all-houses"
        );
        assert_eq!(
            resolve_descriptor(&catalog, "buildings:all-medium")
                .unwrap()
                .category,
            "Regional architecture"
        );
        assert_eq!(
            resolve_descriptor(&catalog, "buildings:house:sydney")
                .unwrap()
                .category,
            "Regional house"
        );
        assert_eq!(
            resolve_descriptor(&catalog, "all-tall").unwrap().id,
            "buildings:all-tall"
        );
        assert_eq!(
            resolve_descriptor(&catalog, "observation-tower")
                .unwrap()
                .id,
            "buildings:observation-tower"
        );
        assert_eq!(
            resolve_descriptor(&catalog, "split-arrow").unwrap().id,
            "drone:split-arrow"
        );
        assert_eq!(
            resolve_descriptor(&catalog, "drone:phase-manta")
                .unwrap()
                .category,
            "Vehicle"
        );
        assert_eq!(
            resolve_descriptor(&catalog, "sparrow-hoverbike")
                .unwrap()
                .id,
            "vehicle:sparrow-hoverbike"
        );
        assert_eq!(
            resolve_descriptor(&catalog, "tall:bundled-tube")
                .unwrap()
                .category,
            "Legacy tall building"
        );
        assert_eq!(
            resolve_descriptor(&catalog, "medium:salvador-da-bahia")
                .unwrap()
                .id,
            "buildings:medium:salvador-da-bahia"
        );
        assert_eq!(
            resolve_descriptor(&catalog, "tree:banyan")
                .unwrap()
                .category,
            "Tree"
        );
        assert_eq!(
            resolve_descriptor(&catalog, "banyan").unwrap().id,
            "tree:banyan"
        );
        assert_eq!(
            resolve_descriptor(&catalog, "eiffel-tower").unwrap().id,
            "landmark:eiffel-tower"
        );
    }

    #[test]
    fn production_tree_preview_builds_a_grounded_mesh() {
        let asset = build_tree(TreeProfileId::Banyan);
        let bounds = Bounds::from_mesh(&asset.mesh);
        assert!((100..=3000).contains(&(asset.mesh.indices.len() / 3)));
        assert!(asset.mesh.vertices.iter().all(|vertex| {
            vertex.pos.into_iter().all(f32::is_finite)
                && vearth_core::mesh::decode_surface_normal(vertex.face_ao).is_some()
        }));
        let materials: HashSet<_> = asset
            .mesh
            .vertices
            .iter()
            .map(|vertex| ((vertex.face_ao >> 5) & 255) as u8)
            .collect();
        assert!(materials.contains(&vearth_gen::material::TRUNK));
        assert!(materials.contains(&vearth_gen::material::foliage(
            TreeProfileId::Banyan.foliage_traits()
        )));
        assert!(bounds.min.y.abs() < 1e-6);
        assert!(bounds.max.y > 10.0);
    }

    #[test]
    fn vehicle_previews_use_the_production_surface_meshes() {
        for kind in [
            crate::vehicles::VehicleKind::Kestrel,
            crate::vehicles::VehicleKind::Sparrow,
            crate::vehicles::VehicleKind::PhaseManta,
            crate::vehicles::VehicleKind::SplitArrow,
        ] {
            let (production, uvs) = crate::vehicles::build_stage_surface(kind);
            let preview = build_vehicle_stage(kind);
            assert_eq!(preview.mesh.indices, production.indices);
            assert_eq!(preview.mesh.vertices.len(), production.vertices.len());
            assert_eq!(preview.uvs.as_ref(), Some(&uvs));
            assert!((Bounds::from_mesh(&preview.mesh).min.y - 1.15).abs() < 1e-5);
            assert!(preview.mesh.vertices.iter().all(|vertex| {
                vearth_core::mesh::decode_surface_normal(vertex.face_ao).is_some()
            }));
        }
        assert!(resolve_descriptor(&stage_catalog(), "vehicle:riptide-watercraft").is_none());
    }

    #[test]
    fn repeated_vehicle_previews_preserve_production_texture_coordinates() {
        for kind in [
            crate::vehicles::VehicleKind::Kestrel,
            crate::vehicles::VehicleKind::PhaseManta,
        ] {
            let asset = build_vehicle_stage(kind);
            let production_uvs = asset.uvs.as_ref().unwrap();
            assert!(production_uvs.iter().any(|uv| *uv != [0.0; 2]));
            assert!(
                asset
                    .mesh
                    .vertices
                    .iter()
                    .any(|vertex| (vertex.face_ao >> 13) & 7 != 0)
            );
            for count in [1, 3] {
                let (mesh, uvs) = repeat_asset(&asset, count);
                assert_eq!(uvs.len(), mesh.vertices.len());
                assert_eq!(uvs, production_uvs.repeat(count));
                for copy in 0..count {
                    let vertex_offset = copy * asset.mesh.vertices.len();
                    let index_offset = copy * asset.mesh.indices.len();
                    for (index, source) in asset.mesh.vertices.iter().enumerate() {
                        let repeated = &mesh.vertices[vertex_offset + index];
                        assert_eq!(repeated.face_ao, source.face_ao);
                        assert_eq!(repeated.rgba, source.rgba);
                    }
                    for (index, source) in asset.mesh.indices.iter().enumerate() {
                        assert_eq!(
                            mesh.indices[index_offset + index],
                            *source + vertex_offset as u32
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn tree_grid_contains_every_rendered_lod_and_profile_season() {
        let (asset, info) = build_tree_grid(TreeProfileId::SugarMaple);
        let bounds = Bounds::from_mesh(&asset.mesh);
        assert_eq!(info.columns.len(), 3);
        assert_eq!(info.rows, ["Summer", "Autumn"]);
        assert!(info.columns[0].starts_with("Coarse"));
        assert!(info.columns[2].starts_with("Fine"));
        assert!(
            info.columns
                .iter()
                .all(|label| label.ends_with("triangles"))
        );
        let production_triangles: usize = (0..=2)
            .map(|lod| {
                vearth_gen::tree_mesh::build_tree_mesh(TreeProfileId::SugarMaple, lod)
                    .mesh
                    .indices
                    .len()
                    / 3
            })
            .sum();
        assert_eq!(
            asset.mesh.indices.len() / 3,
            production_triangles * info.rows.len()
        );
        assert!(bounds.min.y.abs() < 1e-6);
        assert!(bounds.size().x > bounds.size().z);

        let materials: HashSet<_> = asset
            .mesh
            .vertices
            .iter()
            .map(|vertex| ((vertex.face_ao >> 5) & 0xff) as u8)
            .collect();
        assert!(materials.contains(&vearth_gen::material::TRUNK));
        assert!(materials.contains(&vearth_gen::material::LEAVES));
        assert!(
            materials
                .iter()
                .all(|material| !vearth_gen::material::is_foliage(*material)),
            "seasonal colours must be baked per grid row"
        );
    }

    #[test]
    fn tree_grid_uses_the_seasonal_response_declared_by_each_profile() {
        assert_eq!(
            tree_season_variants(TreeProfileId::FloweringOrnamental).map(|row| row.label),
            ["Spring bloom", "Summer"]
        );
        assert_eq!(
            tree_season_variants(TreeProfileId::Banyan).map(|row| row.label),
            ["Wet season", "Dry season"]
        );
        assert_eq!(
            tree_season_variants(TreeProfileId::NorwaySpruce).map(|row| row.label),
            ["Summer", "Winter"]
        );
    }

    #[test]
    fn repeated_layout_expands_around_the_origin() {
        let asset = build_vehicle_stage(crate::vehicles::VehicleKind::Kestrel);
        let single = Bounds::from_mesh(&repeat_asset(&asset, 1).0);
        let repeated = Bounds::from_mesh(&repeat_asset(&asset, 9).0);
        assert!(repeated.size().x > single.size().x * 2.0);
        assert!(repeated.size().z > single.size().z * 2.0);
        assert!(repeated.centre().x.abs() < 1e-6);
        assert!(repeated.centre().z.abs() < 1e-6);
    }

    #[test]
    fn options_validate_assets_and_camera_values() {
        let args = vec![
            "vearth-app".into(),
            "--stage".into(),
            "banyan".into(),
            "--stage-instances".into(),
            "4".into(),
            "--stage-azimuth".into(),
            "90".into(),
        ];
        let parsed = parse_options(&args).unwrap().unwrap();
        assert_eq!(parsed.asset, "tree:banyan");
        assert_eq!(parsed.instances, 4);
        assert_eq!(parsed.azimuth_deg, 90.0);
    }

    #[test]
    fn stage_sun_hour_preserves_noon_and_accepts_repeatable_raking_light() {
        assert_eq!(
            StageOptions::default().solar_utc_seconds(),
            STAGE_UTC_SECONDS
        );
        let args = |hour: &str| {
            vec![
                "vearth-app".into(),
                "--stage".into(),
                "roads:cobblestone".into(),
                "--stage-sun-hour".into(),
                hour.into(),
            ]
        };
        let afternoon = parse_options(&args("15")).unwrap().unwrap();
        assert_eq!(
            afternoon.solar_utc_seconds(),
            STAGE_UTC_SECONDS + 3.0 * 3600.0
        );
        for hour in ["NaN", "inf", "-1", "24", "noon"] {
            assert!(parse_options(&args(hour)).is_err());
        }
        let mut missing = args("15");
        missing.pop();
        assert!(parse_options(&missing).is_err());
    }

    #[test]
    fn both_architecture_catalogues_include_every_region_at_one_scale() {
        for scale in [BuildingScale::ShortHouse, BuildingScale::MediumBuilding] {
            let asset = build_building_catalogue(scale);
            let bounds = Bounds::from_mesh(&asset.mesh);
            assert!(asset.mesh.indices.len() > ARCHITECTURE_PROFILES.len() * 36);
            assert!(!asset.mesh.indices.is_empty());
            assert!(bounds.min.y.abs() < 1e-6);
            assert!(bounds.size().x > 70.0);
            assert!(bounds.size().z > 55.0);
            if scale == BuildingScale::ShortHouse {
                assert!(
                    bounds.max.y < 14.0,
                    "short catalogue is too tall: {bounds:?}"
                );
            } else {
                assert!(
                    bounds.max.y > 14.0,
                    "medium catalogue is too short: {bounds:?}"
                );
            }
        }
    }

    #[test]
    fn loft_normalizes_reversed_sections_without_changing_geometry() {
        let rings = [0., 5.].map(|y| {
            vec![
                DVec3::new(-2., y, -3.),
                DVec3::new(-2., y, 3.),
                DVec3::new(2., y, 3.),
                DVec3::new(2., y, -3.),
            ]
        });
        let reversed = rings.clone().map(|mut ring| {
            ring.reverse();
            ring
        });
        let appearance = building_appearance(vearth_gen::material::WALL, [99, 100, 101]);
        let mut a = StageSurfaceBuilder::default();
        a.loft(&rings, appearance);
        let a = a.mesh(1., DVec3::ZERO);
        let mut b = StageSurfaceBuilder::default();
        b.loft(&reversed, appearance);
        let b = b.mesh(1., DVec3::ZERO);
        assert_eq!(a.indices, b.indices);
        assert_eq!(a.vertices, b.vertices);
        assert_eq!(a.indices.len(), 36);
    }

    #[test]
    fn direct_stage_primitives_preserve_materials_without_volume_work() {
        use vearth_gen::material;
        let mut builder = StageSurfaceBuilder::default();
        building_box(&mut builder, (0, 0, 0), (20, 12, 16), [153, 121, 98]);
        building_roof(
            &mut builder,
            RoofShape::Hipped,
            10,
            8,
            13,
            6,
            1,
            [122, 77, 55],
        );
        building_box_material(
            &mut builder,
            (4, 3, 17),
            (7, 6, 17),
            material::WINDOW,
            BUILDING_GLASS,
        );
        building_box_material(
            &mut builder,
            (10, 3, 17),
            (13, 6, 17),
            material::LIT_WINDOW,
            BUILDING_GLASS,
        );
        let mesh = builder.architectural_mesh(
            0.6,
            DVec3::ZERO,
            Some((ArchitectureRegion::Italy, RoofShape::Hipped)),
        );
        let materials: HashSet<_> = mesh
            .vertices
            .iter()
            .map(|v| ((v.face_ao >> 5) & 255) as u8)
            .collect();
        for material in [
            material::PLASTER_WALL,
            material::TILE_ROOF,
            material::WINDOW,
            material::LIT_WINDOW,
        ] {
            assert!(materials.contains(&material));
        }
        assert!(
            mesh.vertices
                .iter()
                .all(|v| v.pos.iter().all(|c| c.is_finite())
                    && vearth_core::mesh::decode_surface_normal(v.face_ao).is_some())
        );
        assert!(mesh.vertices.iter().any(|v| {
            let n = vearth_core::mesh::decode_surface_normal(v.face_ao).unwrap();
            n[1] > 0.1 && n[1] < 0.99
        }));
        let mut huge = StageSurfaceBuilder::default();
        huge.box_surface(
            DVec3::ZERO,
            DVec3::splat(1_000_000.),
            building_appearance(material::WALL, [1, 2, 3]),
        );
        let huge = huge.mesh(1., DVec3::ZERO);
        assert_eq!(huge.indices.len(), 36);
        assert_eq!(huge.vertices.len(), 36);
    }

    #[test]
    fn tall_catalogue_includes_every_family_and_full_height_range() {
        let asset = build_tall_building_catalogue();
        let bounds = Bounds::from_mesh(&asset.mesh);
        assert!(asset.mesh.indices.len() > TALL_BUILDING_PROFILES.len() * 36);
        assert!(!asset.mesh.indices.is_empty());
        assert!(bounds.min.y.abs() < 1e-6);
        assert!(
            bounds.max.y > 180.0,
            "supertall spire is missing: {bounds:?}"
        );
        assert!(bounds.size().x > 180.0);
        assert!(bounds.size().z > 150.0);
        for profile in TALL_BUILDING_PROFILES {
            let focused = build_tall_building(profile.id);
            assert!(
                focused.mesh.indices.len() > 100,
                "{} is underbuilt",
                profile.slug
            );
            assert!(
                !focused.mesh.indices.is_empty(),
                "{} has no mesh",
                profile.slug
            );
        }
    }

    #[test]
    fn observation_tower_stage_has_concrete_glass_and_antenna_height() {
        let asset = build_observation_tower();
        let bounds = Bounds::from_mesh(&asset.mesh);
        assert!(asset.mesh.indices.len() > 100);
        assert!(!asset.mesh.indices.is_empty());
        assert!(bounds.min.y.abs() < 1e-6);
        assert!(bounds.max.y > 210.0);
        assert!(bounds.size().x > 15.0);
        assert!(bounds.size().x < 30.0);
    }

    #[test]
    fn anchor_regions_have_distinct_short_house_silhouettes() {
        use vearth_gen::architecture::ArchitectureRegion;
        let bounds = |region| {
            let profile = vearth_gen::architecture::architecture_profile(region);
            Bounds::from_mesh(&build_regional_building(profile, BuildingScale::ShortHouse, 0).mesh)
        };
        let congo = bounds(ArchitectureRegion::CongoBasin);
        let nordic = bounds(ArchitectureRegion::Nordic);
        let sydney = bounds(ArchitectureRegion::Sydney);
        let salvador = bounds(ArchitectureRegion::Salvador);
        let santorini = bounds(ArchitectureRegion::Santorini);
        assert!(congo.min.y <= 0.0 && congo.max.y > 6.0);
        assert!(nordic.max.y > santorini.max.y);
        assert!(sydney.size().x > salvador.size().x);
        assert!(salvador.size().y > santorini.size().y);
        assert!(santorini.size().x > 10.0);
    }
}
