//! Global authored paving priors and bounded, independently sourced local treatments.
//! Geography selects a palette, never a claim about a particular road's surveyed surface.

use crate::architecture_catalog::{RegionalFamily, family_for_context};
use std::{collections::BTreeMap, sync::OnceLock};
use vearth_data::{
    admin_regions::AdminRegionContext,
    climate::ClimateAtlas,
    road_profile::{RoadDetail, name_seed},
    vectors::RoadClass,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub(crate) enum Finish {
    Asphalt,
    Concrete,
    Brick,
    Stone,
    Gravel,
    Earth,
    Cobblestone,
    StoneSlab,
}
pub(crate) const FINISHES: [Finish; 8] = [
    Finish::Asphalt,
    Finish::Concrete,
    Finish::Brick,
    Finish::Stone,
    Finish::Gravel,
    Finish::Earth,
    Finish::Cobblestone,
    Finish::StoneSlab,
];
impl Finish {
    pub(crate) const fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Recipe {
    pub label: &'static str,
    /// Additive preferences, applied only after the resolver checks route function.
    pub paving: [f32; 8],
    pub colors: [[f32; 3]; 8],
}

#[derive(Clone, Copy)]
struct RegionalRecipe {
    label: &'static str,
    // Concrete, brick, cut setts, rounded cobbles, stone slabs. These are authored
    // choices, not percentages of the region's installed paving stock.
    units: [f32; 5],
    stone: [f32; 3],
    brick: [f32; 3],
}
macro_rules! regions {
    ($($name:ident => ($label:literal, $units:expr, $stone:expr, $brick:expr)),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        enum Region { $($name),+ }
        impl Region { fn recipe(self) -> RegionalRecipe { match self { $(Self::$name => RegionalRecipe {label:$label, units:$units, stone:$stone, brick:$brick}),+ } } }
    }
}
regions! {
    Neutral => ("Contemporary neutral", [25.,4.,3.,0.,5.], [145.,145.,137.], [142.,116.,100.]),
    LowCountries => ("Low Countries clay and concrete", [15.,90.,9.,1.,8.], [145.,143.,132.], [139.,105.,87.]),
    Atlantic => ("Atlantic European concrete and stone", [24.,8.,14.,2.,16.], [148.,147.,139.], [142.,110.,96.]),
    British => ("British and Irish flags and concrete", [27.,8.,12.,3.,85.], [158.,150.,132.], [143.,111.,96.]),
    Nordic => ("Nordic concrete and cool granite", [28.,5.,15.,1.,19.], [143.,145.,144.], [139.,113.,104.]),
    Continental => ("Continental European unit paving", [22.,17.,19.,2.,17.], [148.,146.,138.], [142.,114.,98.]),
    Alpine => ("Alpine aggregate and stone", [22.,6.,35.,3.,65.], [146.,148.,143.], [142.,119.,101.]),
    Mediterranean => ("Mediterranean warm stone", [21.,9.,24.,3.,75.], [162.,153.,134.], [151.,118.,96.]),
    Iberian => ("Iberian granite and pale stone", [19.,12.,40.,3.,65.], [163.,159.,143.], [147.,114.,95.]),
    Eastern => ("Eastern European concrete and granite", [30.,8.,16.,2.,13.], [149.,148.,141.], [139.,113.,103.]),
    NorthAmerican => ("North American concrete", [38.,6.,4.,0.,9.], [148.,147.,137.], [143.,110.,94.]),
    NortheastAmerican => ("Northeastern concrete and granite", [34.,12.,8.,1.,14.], [146.,145.,138.], [137.,103.,89.]),
    DryAmerican => ("Dry American concrete and warm aggregate", [34.,10.,5.,1.,10.], [161.,150.,132.], [151.,118.,96.]),
    TropicalAmerican => ("Tropical American concrete and unit paving", [34.,13.,9.,1.,13.], [157.,154.,143.], [148.,115.,96.]),
    Andean => ("Andean concrete and muted stone", [29.,8.,21.,5.,14.], [141.,139.,130.], [147.,112.,91.]),
    SouthernCone => ("Southern Cone concrete and tiles", [30.,19.,10.,1.,18.], [156.,151.,139.], [147.,113.,94.]),
    Brazilian => ("Brazilian concrete and pale stone", [29.,14.,20.,2.,18.], [167.,162.,147.], [148.,115.,96.]),
    NorthAfrican => ("North African concrete and pale aggregate", [32.,9.,12.,2.,19.], [167.,156.,133.], [150.,116.,94.]),
    DryAfrican => ("Dry African concrete and ochre aggregate", [36.,9.,5.,1.,8.], [159.,147.,124.], [145.,110.,89.]),
    HumidAfrican => ("Humid African concrete and earth", [38.,10.,4.,0.,8.], [151.,145.,131.], [146.,109.,91.]),
    HighlandAfrican => ("African highland concrete and stone", [29.,9.,21.,4.,12.], [139.,136.,126.], [142.,108.,88.]),
    SouthernAfrican => ("Southern African concrete and block paving", [35.,24.,8.,1.,10.], [151.,146.,132.], [142.,111.,94.]),
    Arabian => ("Arabian concrete and pale paving", [38.,12.,9.,1.,22.], [168.,157.,137.], [153.,125.,103.]),
    WestAsian => ("West Asian concrete and stone", [30.,13.,17.,2.,21.], [159.,148.,130.], [148.,113.,91.]),
    ContinentalAsian => ("Continental Asian concrete and aggregate", [37.,9.,10.,1.,12.], [150.,148.,139.], [145.,115.,99.]),
    SouthAsian => ("South Asian concrete and paving blocks", [40.,17.,9.,1.,14.], [155.,149.,135.], [149.,114.,92.]),
    Himalayan => ("Himalayan stone and concrete", [29.,6.,48.,5.,60.], [148.,145.,134.], [144.,114.,95.]),
    EastAsian => ("East Asian concrete and cut stone", [32.,15.,13.,1.,62.], [150.,151.,145.], [143.,115.,102.]),
    Japanese => ("Japanese asphalt concrete and fine stone", [27.,12.,10.,0.,50.], [147.,148.,143.], [140.,115.,102.]),
    SoutheastAsian => ("Southeast Asian concrete and unit paving", [40.,16.,6.,0.,14.], [158.,154.,142.], [145.,113.,94.]),
    Australian => ("Australian concrete and dark stone", [35.,15.,7.,1.,17.], [128.,132.,129.], [148.,114.,94.]),
    Pacific => ("Pacific concrete and local aggregate", [40.,5.,4.,0.,9.], [145.,145.,134.], [145.,117.,98.]),
    Polar => ("Cold region asphalt concrete and gravel", [29.,2.,4.,0.,6.], [144.,145.,139.], [137.,114.,104.])
}

fn region(context: AdminRegionContext) -> Option<Region> {
    if context.country_name == "Slovenia" {
        return Some(Region::Eastern);
    }
    if matches!(context.country_name, "Netherlands" | "Belgium") {
        return Some(Region::LowCountries);
    }
    use RegionalFamily as F;
    Some(match family_for_context(context)? {
        // Construction refinements retain their previous paving priors; roof
        // and facade evidence alone does not establish a different road surface.
        F::AtlanticSlate if context.country_name == "United Kingdom" => Region::British,
        F::AtlanticSlate | F::PortugueseAtlantic => Region::Atlantic,
        F::NorthEuropeanBrick => Region::Nordic,
        F::JulianAlpine | F::Adriatic => Region::Eastern,
        F::Aegean | F::Maltese => Region::Mediterranean,
        F::Pueblo => Region::DryAmerican,
        F::PeruvianCoast => Region::Andean,
        F::ChileanLakes => Region::Polar,
        F::Tamil | F::Gujarati => Region::SouthAsian,
        F::Bhutanese => Region::Himalayan,
        F::Hui => Region::EastAsian,
        F::Cape => Region::SouthernAfrican,
        F::Tasmanian => Region::Australian,
        F::AtlanticEurope => Region::Atlantic,
        F::BritishIsles => Region::British,
        F::Nordic | F::Baltic => Region::Nordic,
        F::CentralEurope => Region::Continental,
        F::Alpine => Region::Alpine,
        F::Mediterranean | F::Italian => Region::Mediterranean,
        F::Iberian => Region::Iberian,
        F::Balkan | F::EastEuropean => Region::Eastern,
        F::Caucasus | F::Anatolian | F::Levantine | F::Iranian | F::Caspian => Region::WestAsian,
        F::NorthAmerican | F::NorthAmericanSouth | F::PacificNorthwest => Region::NorthAmerican,
        F::NewEngland | F::Quebec => Region::NortheastAmerican,
        F::NorthAmericanDry | F::MexicanDry => Region::DryAmerican,
        F::Arctic => Region::Polar,
        F::Caribbean | F::MexicanTropical | F::MesoamericanTropical | F::SouthAmericanTropical => {
            Region::TropicalAmerican
        }
        F::MexicanHighland | F::Andean => Region::Andean,
        F::SouthernCone => Region::SouthernCone,
        F::Patagonian => Region::Polar,
        F::BrazilianAtlantic | F::BrazilianNortheast => Region::Brazilian,
        F::Maghreb | F::Nile => Region::NorthAfrican,
        F::Saharan | F::Sahel | F::Horn => Region::DryAfrican,
        F::WestAfrican | F::CentralAfrican | F::Swahili | F::Madagascan => Region::HumidAfrican,
        F::EastAfrican | F::EthiopianHighland => Region::HighlandAfrican,
        F::SouthAfrican => Region::SouthernAfrican,
        F::IndianOcean | F::PacificIsland => Region::Pacific,
        F::GulfArabian | F::YemeniHighland => Region::Arabian,
        F::CentralAsian => Region::ContinentalAsian,
        F::IndianPlain | F::Deccan | F::MonsoonCoast | F::BengalDelta | F::NortheastIndian => {
            Region::SouthAsian
        }
        F::Himalayan | F::HimalayanDry => Region::Himalayan,
        F::ChineseNorth | F::ChineseSouth | F::Korean => Region::EastAsian,
        F::Japanese | F::JapaneseSnow | F::Ryukyuan => Region::Japanese,
        F::SoutheastAsian | F::Malay | F::Indonesian | F::Philippine | F::Vietnamese => {
            Region::SoutheastAsian
        }
        F::Australian | F::AustralianTropical | F::NewZealand => Region::Australian,
    })
}

pub(crate) fn recipe(lat: f64, lon: f64, context: Option<AdminRegionContext>) -> Recipe {
    let regional = context.and_then(region).unwrap_or(Region::Neutral).recipe();
    let climate = ClimateAtlas::bundled().annual_surface(lat, lon);
    let dry = (1.0 - climate.precipitation_mm_day / 3.0).clamp(0.0, 1.0);
    let warm = ((climate.mean_temperature_c - 3.0) / 24.0).clamp(0.0, 1.0);
    let sandy = dry * warm;
    let mix = |a: [f32; 3], b: [f32; 3], t: f32| std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t);
    let stone = regional.stone;
    let mut colors = [
        [77., 80., 78.],
        mix([162., 163., 155.], [169., 162., 145.], sandy * 0.65),
        regional.brick,
        stone,
        mix([145., 145., 132.], [168., 153., 121.], sandy),
        mix([145., 132., 109.], [172., 148., 108.], sandy),
        mix(stone, [139., 139., 129.], 0.25),
        mix(stone, [164., 161., 148.], 0.12),
    ];
    let [concrete, mut brick, stone, cobble, slab] = regional.units;
    if context.is_some_and(|context| context.country_name == "Hong Kong S.A.R.") {
        // Hong Kong's standard block-paving detail prefers concrete blocks;
        // the shared rectangular unit texture does not imply fired clay.
        colors[Finish::Brick.index()] = [151., 152., 145.];
        brick = 70.;
    }
    Recipe {
        label: regional.label,
        paving: [0., concrete, brick, stone, 0., 0., cobble, slab],
        colors,
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct LocalPaving {
    pub label: &'static str,
    pub finish: Finish,
    pub color: [f32; 3],
}
#[derive(Clone, Copy)]
enum Role {
    Walking,
    Shared,
    WalkingAndShared,
}

struct Place {
    label: &'static str,
    // South, west, north, east in degrees; intentionally small source-scoped envelopes.
    bounds: [f64; 4],
    names: &'static [&'static str],
    role: Role,
    finish: Finish,
    color: [f32; 3],
    source: &'static str,
}
const YORK: &str = "https://www.york.gov.uk/downloads/file/1747/sd109-city-of-york-streetscape-strategy-and-guidance-2014-";
const DUMBO: &str = "https://www.nyc.gov/site/dep/news/25-029/city-completes-108-million-historic-street-restoration-upgrade-dumbo";
const KYOTO: &str =
    "https://www.city.kyoto.lg.jp/gyozai/cmsfiles/contents/0000172/172480/zenntai.pdf";
const PLACES: &[Place] = &[
    Place {
        label: "Amsterdam canal walking brick",
        bounds: [52.3670, 4.8800, 52.3760, 4.8970],
        names: &["Herengracht", "Keizersgracht", "Prinsengracht"],
        role: Role::Walking,
        finish: Finish::Brick,
        color: [139., 105., 87.],
        source: "https://www.amsterdam.nl/publish/pages/869895/visie_openbare_ruimte_2025.pdf",
    },
    Place {
        label: "Soweto memorial walking brick",
        bounds: [-26.2365, 27.9078, -26.2345, 27.9093],
        names: &["Khumalo Street"],
        role: Role::Walking,
        finish: Finish::Brick,
        color: [145., 109., 91.],
        source: "https://joburg.org.za/play_/Pages/Play%20in%20Joburg/Culture%20and%20Heritage/Links/Soweto%27s-red-brick-route-to-16-June-1976.aspx",
    },
    Place {
        label: "Paraty irregular stone streets",
        bounds: [-23.2210, -44.7168, -23.2160, -44.7122],
        names: &[
            "Rua do Comércio",
            "Rua da Lapa",
            "Rua Samuel Costa",
            "Rua Doutor Samuel Costa",
        ],
        role: Role::Shared,
        finish: Finish::Cobblestone,
        color: [146., 144., 133.],
        source: "https://agenciadenoticias.ibge.gov.br/agencia-noticias/2012-agencia-de-noticias/noticias/23992-nomes-geograficos-identificam-lugares-e-ajudam-a-contar-suas-historias",
    },
    Place {
        label: "York College Street mixed stone setts",
        bounds: [53.9624, -1.0824, 53.9636, -1.0800],
        names: &["College Street"],
        role: Role::Shared,
        finish: Finish::Stone,
        color: [145., 142., 128.],
        source: YORK,
    },
    Place {
        label: "York Stonegate sandstone flags",
        bounds: [53.9604, -1.0847, 53.9620, -1.0815],
        names: &["Stonegate"],
        role: Role::WalkingAndShared,
        finish: Finish::StoneSlab,
        color: [164., 152., 129.],
        source: YORK,
    },
    Place {
        label: "York Fossgate stone setts",
        bounds: [53.9575, -1.0797, 53.9603, -1.0762],
        names: &["Fossgate"],
        role: Role::Shared,
        finish: Finish::Stone,
        color: [146., 140., 126.],
        source: "https://democracy.york.gov.uk/mgAi.aspx?ID=34368",
    },
    Place {
        label: "York Fossgate sandstone footways",
        bounds: [53.9575, -1.0797, 53.9603, -1.0762],
        names: &["Fossgate"],
        role: Role::Walking,
        finish: Finish::StoneSlab,
        color: [164., 152., 129.],
        source: "https://democracy.york.gov.uk/mgAi.aspx?ID=34368",
    },
    Place {
        label: "DUMBO restored Belgian block",
        bounds: [40.7025, -73.9923, 40.7045, -73.9812],
        names: &[
            "Water Street",
            "Washington Street",
            "Pearl Street",
            "Gold Street",
        ],
        role: Role::Shared,
        finish: Finish::Stone,
        color: [145., 142., 134.],
        source: DUMBO,
    },
    Place {
        label: "DUMBO Pearl Street Plaza granite",
        bounds: [40.7030, -73.9873, 40.7042, -73.9860],
        names: &["Pearl Street", "Pearl Street Plaza"],
        role: Role::Walking,
        finish: Finish::StoneSlab,
        color: [156., 153., 143.],
        source: DUMBO,
    },
    Place {
        label: "Melbourne Hardware Lane bluestone",
        bounds: [-37.8150, 144.9598, -37.8127, 144.9618],
        names: &["Hardware Lane"],
        role: Role::WalkingAndShared,
        finish: Finish::StoneSlab,
        color: [121., 127., 125.],
        source: "https://participate.melbourne.vic.gov.au/hardware-lane/streetscape-renewal",
    },
    Place {
        label: "Kyoto Hanamikoji stone",
        bounds: [34.9990, 135.7745, 35.0041, 135.7759],
        names: &[
            "花見小路通",
            "花見小路",
            "Hanamikoji-dori",
            "Hanamikoji Street",
        ],
        role: Role::WalkingAndShared,
        finish: Finish::StoneSlab,
        color: [151., 152., 145.],
        source: KYOTO,
    },
    Place {
        label: "Kyoto Miyagawacho stone",
        bounds: [34.9963, 135.7684, 35.0012, 135.7698],
        names: &["宮川町通", "宮川町", "Miyagawacho-dori"],
        role: Role::WalkingAndShared,
        finish: Finish::Stone,
        color: [145., 147., 140.],
        source: KYOTO,
    },
    Place {
        label: "Kyoto Tatsumi stone",
        bounds: [35.0050, 135.7737, 35.0064, 135.7759],
        names: &["辰巳通", "Tatsumi-dori"],
        role: Role::WalkingAndShared,
        finish: Finish::StoneSlab,
        color: [145., 148., 143.],
        source: KYOTO,
    },
];

struct LocalIndex {
    buckets: BTreeMap<(i16, i16), Vec<usize>>,
    names: Vec<Vec<u64>>,
}
fn bucket(lat: f64, lon: f64) -> (i16, i16) {
    ((lat / 2.).floor() as i16, (lon / 2.).floor() as i16)
}
fn index() -> &'static LocalIndex {
    static INDEX: OnceLock<LocalIndex> = OnceLock::new();
    INDEX.get_or_init(|| {
        let mut buckets = BTreeMap::<_, Vec<_>>::new();
        for (i, place) in PLACES.iter().enumerate() {
            debug_assert!(place.source.starts_with("https://"));
            let a = bucket(place.bounds[0], place.bounds[1]);
            let b = bucket(place.bounds[2], place.bounds[3]);
            for y in a.0..=b.0 {
                for x in a.1..=b.1 {
                    buckets.entry((y, x)).or_default().push(i);
                }
            }
        }
        LocalIndex {
            buckets,
            names: PLACES
                .iter()
                .map(|place| {
                    place
                        .names
                        .iter()
                        .map(|name| name_seed(Some(name)))
                        .collect()
                })
                .collect(),
        }
    })
}
fn role_matches(role: Role, detail: RoadDetail, class: RoadClass) -> bool {
    if matches!(
        class,
        RoadClass::Highway | RoadClass::Major | RoadClass::Rail
    ) || matches!(
        detail,
        RoadDetail::Ramp
            | RoadDetail::Cycleway
            | RoadDetail::Crossing
            | RoadDetail::Track
            | RoadDetail::Bridleway
            | RoadDetail::Driveway
            | RoadDetail::ParkingAisle
            | RoadDetail::Pier
            | RoadDetail::Steps
            | RoadDetail::Corridor
    ) {
        return false;
    }
    let walking = matches!(detail, RoadDetail::Footway | RoadDetail::Sidewalk)
        || detail == RoadDetail::Generic && class == RoadClass::Path;
    let shared = matches!(
        detail,
        RoadDetail::Pedestrian | RoadDetail::LivingStreet | RoadDetail::Alley
    ) || detail == RoadDetail::Generic
        && matches!(class, RoadClass::Minor | RoadClass::Service);
    match role {
        Role::Walking => walking,
        Role::Shared => shared,
        Role::WalkingAndShared => walking || shared,
    }
}

pub(crate) fn local_finish(
    lat: f64,
    lon: f64,
    detail: RoadDetail,
    class: RoadClass,
    seed: u64,
) -> Option<LocalPaving> {
    if seed == 0 || !lat.is_finite() || !lon.is_finite() || lat.abs() > std::f64::consts::FRAC_PI_2
    {
        return None;
    }
    let lat = lat.to_degrees();
    let lon = (lon.to_degrees() + 180.).rem_euclid(360.) - 180.;
    let index = index();
    for &i in index.buckets.get(&bucket(lat, lon))? {
        let place = &PLACES[i];
        let [south, west, north, east] = place.bounds;
        if lat >= south
            && lat <= north
            && lon >= west
            && lon <= east
            && role_matches(place.role, detail, class)
            && index.names[i].contains(&seed)
        {
            return Some(LocalPaving {
                label: place.label,
                finish: place.finish,
                color: place.color,
            });
        }
    }
    None
}

/// Catalog-derived inputs for renderer specimens and material audits.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LocalExample {
    pub label: &'static str,
    pub source: &'static str,
    pub lat_rad: f64,
    pub lon_rad: f64,
    pub detail: RoadDetail,
    pub class: RoadClass,
    pub name_seed: u64,
    pub finish: Finish,
    pub color: [f32; 3],
}

pub(crate) fn local_examples() -> impl ExactSizeIterator<Item = LocalExample> {
    PLACES.iter().map(|place| LocalExample {
        label: place.label,
        source: place.source,
        lat_rad: ((place.bounds[0] + place.bounds[2]) * 0.5).to_radians(),
        lon_rad: ((place.bounds[1] + place.bounds[3]) * 0.5).to_radians(),
        detail: if matches!(place.role, Role::Walking) {
            RoadDetail::Sidewalk
        } else {
            RoadDetail::Pedestrian
        },
        class: RoadClass::Path,
        name_seed: name_seed(Some(place.names[0])),
        finish: place.finish,
        color: place.color,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use vearth_data::admin_regions::AdminRegionAtlas;
    #[test]
    fn every_bundled_administrative_region_has_paving() {
        let mut recipes = std::collections::BTreeSet::new();
        for context in AdminRegionAtlas::bundled().contexts() {
            let region =
                region(context).unwrap_or_else(|| panic!("Unmapped paving geography: {context:?}"));
            let recipe = region.recipe();
            recipes.insert(recipe.label);
            assert!(recipe.units.iter().all(|v| v.is_finite() && *v >= 0.));
            assert!(recipe.units.iter().sum::<f32>() > 0.);
        }
        assert!(
            recipes.len() >= 30,
            "Global palettes collapsed: {recipes:?}"
        );
    }
    #[test]
    fn unknown_geography_has_finite_climate_conditioned_materials() {
        let dry = recipe(23.4_f64.to_radians(), 25.7_f64.to_radians(), None);
        let wet = recipe(-3.5_f64.to_radians(), -62.2_f64.to_radians(), None);
        assert_ne!(
            dry.colors[Finish::Earth.index()],
            wet.colors[Finish::Earth.index()]
        );
        for lat in [-89., -60., 0., 30., 89.] {
            for lon in [-180., -90., 0., 90., 180.] {
                let recipe = recipe(f64::to_radians(lat), f64::to_radians(lon), None);
                assert!(
                    recipe
                        .colors
                        .into_iter()
                        .flatten()
                        .all(|v| v.is_finite() && (65.0..=180.0).contains(&v))
                );
            }
        }
    }
    #[test]
    fn local_catalog_requires_both_location_and_route_identity() {
        for place in PLACES {
            assert!(place.source.starts_with("https://"));
            assert!(
                place.bounds[2] - place.bounds[0] < 0.02
                    && place.bounds[3] - place.bounds[1] < 0.02
            );
            let lat = ((place.bounds[0] + place.bounds[2]) * 0.5).to_radians();
            let lon = ((place.bounds[1] + place.bounds[3]) * 0.5).to_radians();
            let detail = if matches!(place.role, Role::Walking) {
                RoadDetail::Footway
            } else {
                RoadDetail::Pedestrian
            };
            for name in place.names {
                let matched =
                    local_finish(lat, lon, detail, RoadClass::Path, name_seed(Some(name))).unwrap();
                assert_eq!(matched.finish, place.finish, "{}", place.label);
                assert_eq!(matched.label, place.label);
                assert!(
                    local_finish(
                        lat + 0.01,
                        lon,
                        detail,
                        RoadClass::Path,
                        name_seed(Some(name))
                    )
                    .is_none()
                );
            }
            assert!(local_finish(lat, lon, detail, RoadClass::Path, 0).is_none());
            assert!(
                local_finish(
                    lat,
                    lon,
                    detail,
                    RoadClass::Path,
                    name_seed(Some("unrelated route"))
                )
                .is_none()
            );
        }
    }
    #[test]
    fn local_catalog_preserves_transport_and_accessibility_roles() {
        for place in PLACES {
            let lat = ((place.bounds[0] + place.bounds[2]) * 0.5).to_radians();
            let lon = ((place.bounds[1] + place.bounds[3]) * 0.5).to_radians();
            let seed = name_seed(Some(place.names[0]));
            for detail in [
                RoadDetail::Cycleway,
                RoadDetail::Crossing,
                RoadDetail::Track,
                RoadDetail::Bridleway,
                RoadDetail::Ramp,
            ] {
                assert!(local_finish(lat, lon, detail, RoadClass::Path, seed).is_none());
            }
            for class in [RoadClass::Highway, RoadClass::Major, RoadClass::Rail] {
                assert!(local_finish(lat, lon, RoadDetail::Generic, class, seed).is_none());
            }
        }
    }
    #[test]
    fn diagnostic_examples_exercise_the_production_lookup() {
        for example in local_examples() {
            let actual = local_finish(
                example.lat_rad,
                example.lon_rad,
                example.detail,
                example.class,
                example.name_seed,
            )
            .unwrap();
            assert_eq!(actual.label, example.label);
            assert_eq!(actual.finish, example.finish);
            assert_eq!(actual.color, example.color);
            assert!(example.source.starts_with("https://"));
        }
    }

    #[test]
    fn local_lookup_handles_longitude_wrap_and_bad_coordinates() {
        let lat = 53.963_f64.to_radians();
        let lon = (-1.081_f64).to_radians();
        let seed = name_seed(Some("College Street"));
        assert_eq!(
            local_finish(lat, lon, RoadDetail::Pedestrian, RoadClass::Path, seed)
                .unwrap()
                .finish,
            Finish::Stone
        );
        assert_eq!(
            local_finish(
                lat,
                lon + std::f64::consts::TAU,
                RoadDetail::Pedestrian,
                RoadClass::Path,
                seed
            )
            .unwrap()
            .finish,
            Finish::Stone
        );
        assert!(
            local_finish(f64::NAN, lon, RoadDetail::Pedestrian, RoadClass::Path, seed).is_none()
        );
        assert!(
            local_finish(
                lat,
                f64::INFINITY,
                RoadDetail::Pedestrian,
                RoadClass::Path,
                seed
            )
            .is_none()
        );
    }
}
