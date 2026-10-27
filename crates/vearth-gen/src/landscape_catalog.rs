//! Authored landscape priors; scope and evidence: docs/LANDSCAPES.md.
use super::{Gate, Profile, Rule, Season};

// Forest, grass, scrub and dormant herbaceous anchors are sRGB bytes.
// Substrate is a small stable offset to the evidence-selected material,
// never a geological classifier. Strength scales every cover independently.
macro_rules! profiles {
    ($($id:ident: $season:ident, $forest:expr, $grass:expr, $scrub:expr, $dormant:expr, $substrate:expr, $strength:expr;)+) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        #[repr(u8)]
        pub(super) enum Id { $($id,)+ }
        pub(super) const PROFILES: &[Profile] = &[$(Profile {
            name: stringify!($id), season: Season::$season,
            vegetation: [$forest, $grass, $scrub], dormant: $dormant,
            substrate: $substrate, strength: $strength,
        },)+];
    }
}
profiles! {
    Atlantic: Temperate, [66,99,76], [111,149,98], [113,121,96], [138,130,100], [0,1,2], 0.55;
    Heath: Cool, [72,96,79], [113,137,98], [111,113,94], [133,118,100], [-2,0,2], 0.55;
    Continental: Temperate, [76,99,66], [133,151,91], [124,132,91], [151,129,91], [2,1,-1], 0.48;
    Mediterranean: WinterRain, [88,106,76], [137,148,94], [130,138,103], [171,150,106], [4,2,-2], 0.55;
    Alpine: Short, [77,105,86], [120,150,105], [120,130,104], [146,135,113], [-2,1,3], 0.58;
    Boreal: Cool, [62,94,82], [112,135,101], [104,119,99], [135,124,106], [-3,0,3], 0.52;
    Larch: Cool, [77,106,77], [128,140,98], [119,128,96], [146,127,94], [1,1,-1], 0.50;
    Tundra: Short, [81,100,86], [112,124,96], [117,119,100], [140,120,101], [-1,1,3], 0.55;
    VolcanicHeath: Short, [74,99,80], [115,139,91], [110,124,88], [139,127,100], [-4,-3,-1], 0.58;
    Prairie: Temperate, [86,110,76], [141,156,95], [140,146,103], [170,148,105], [2,1,-2], 0.52;
    Sagebrush: Dry, [99,115,94], [147,151,112], [141,147,116], [169,152,120], [2,1,0], 0.55;
    Sandstone: Dry, [110,119,84], [147,146,97], [145,142,103], [167,150,112], [7,1,-3], 0.52;
    Desert: Dry, [116,124,90], [163,157,115], [160,154,119], [179,161,124], [4,2,-2], 0.42;
    MexicanUpland: Rain, [78,103,76], [125,141,88], [125,132,95], [163,140,98], [3,1,-1], 0.52;
    HumidTropical: Evergreen, [51,91,70], [91,133,85], [73,113,81], [122,133,91], [2,0,-1], 0.52;
    CloudForest: Evergreen, [65,107,91], [108,143,104], [91,124,97], [135,137,106], [-1,1,2], 0.55;
    Cerrado: Savanna, [87,108,70], [135,150,86], [127,135,86], [171,141,90], [6,0,-3], 0.57;
    Caatinga: Savanna, [108,121,83], [145,147,95], [146,142,103], [173,153,115], [3,1,-1], 0.53;
    Pampas: Temperate, [83,112,80], [138,154,101], [130,141,104], [167,146,108], [0,0,-2], 0.50;
    Patagonian: Cool, [78,101,86], [147,147,112], [140,142,115], [164,147,119], [1,1,3], 0.55;
    Puna: Short, [92,109,81], [143,146,98], [139,136,98], [170,148,108], [3,1,-1], 0.53;
    Sahel: Savanna, [111,125,79], [145,155,86], [149,145,98], [181,155,108], [5,2,-2], 0.55;
    Savanna: Savanna, [92,113,73], [141,155,86], [132,140,91], [177,146,91], [4,1,-3], 0.56;
    Miombo: Savanna, [95,112,76], [142,150,94], [128,135,94], [165,138,98], [5,1,-2], 0.53;
    Highland: Rain, [73,103,80], [116,145,87], [115,131,93], [155,137,96], [0,0,1], 0.55;
    Fynbos: WinterRain, [91,113,87], [138,149,104], [127,136,108], [161,145,117], [1,2,3], 0.58;
    Monsoon: Rain, [72,105,72], [118,146,79], [115,133,84], [169,139,93], [4,0,-2], 0.55;
    Ghats: Evergreen, [57,101,77], [105,142,87], [89,122,83], [146,135,94], [3,0,-2], 0.57;
    Deccan: Rain, [91,111,75], [129,147,82], [129,136,89], [169,141,96], [-2,-2,-1], 0.55;
    Steppe: Temperate, [96,118,85], [147,156,106], [140,147,109], [173,151,111], [3,2,-1], 0.53;
    EastAsian: Temperate, [68,104,78], [116,146,86], [108,132,90], [150,132,99], [1,1,0], 0.51;
    Eucalypt: Evergreen, [81,111,96], [141,150,106], [127,142,118], [170,151,116], [3,1,0], 0.55;
    Mallee: WinterRain, [103,120,94], [152,153,110], [141,147,117], [176,153,117], [5,2,-2], 0.55;
    Spinifex: Dry, [115,125,88], [163,154,99], [155,147,102], [184,156,110], [8,0,-3], 0.57;
    Tussock: Cool, [78,109,88], [151,155,108], [135,141,108], [172,148,111], [1,1,2], 0.57;
}

// name: profile, centre latitude/longitude, support radii in degrees, gate.
// Ellipses are soft physiographic footprints, not ecosystem polygons.
macro_rules! rules {
    ($($name:literal: $id:ident, $lat:expr, $lon:expr, $dy:expr, $dx:expr, $gate:ident;)+) => {
        pub(super) const RULES: &[Rule] = &[$(Rule {
            name: $name, profile: Id::$id, centre: [$lat, $lon], radius: [$dy, $dx], gate: Gate::$gate,
        },)+];
    }
}
rules! {
    "Atlantic seaboard meadows": Atlantic, 51.5,-6.5,7.0,6.0, Moist;
    "Atlantic upland heath": Heath, 57.0,-5.0,5.5,6.0, Cool;
    "Western European woodland": Continental, 49.5,6.5,6.0,9.0, Moist;
    "Central European woodland": Continental, 50.5,19.0,6.0,11.0, Moist;
    "Western Mediterranean": Mediterranean, 39.0,-2.5,5.0,8.0, Mild;
    "Central Mediterranean": Mediterranean, 39.0,13.0,5.0,8.0, Mild;
    "Eastern Mediterranean": Mediterranean, 37.5,26.0,5.5,10.0, Mild;
    "Atlas uplands": Mediterranean, 32.0,-5.0,3.5,6.0, Upland;
    "Alps": Alpine, 46.5,10.0,2.7,7.0, Alpine;
    "Pyrenees": Alpine, 42.7,0.5,1.8,4.0, Alpine;
    "Carpathians": Alpine, 47.5,24.0,3.0,5.0, Alpine;
    "Caucasus": Alpine, 42.5,44.0,2.5,6.0, Alpine;
    "Nordic boreal": Boreal, 63.0,21.0,7.0,14.0, Cool;
    "Western Siberian taiga": Boreal, 61.0,65.0,8.0,24.0, Cool;
    "Central Siberian larch": Larch, 62.0,103.0,9.0,24.0, Cool;
    "Eastern Siberian larch": Larch, 63.0,139.0,10.0,25.0, Cool;
    "Fennoscandian tundra": Tundra, 70.0,25.0,4.0,16.0, Cold;
    "Taymyr tundra": Tundra, 73.0,100.0,6.0,28.0, Cold;
    "Chukotka tundra": Tundra, 69.0,177.0,6.0,24.0, Cold;
    "Iceland heath and lava": VolcanicHeath, 64.9,-18.8,2.1,7.0, Cool;
    "Pacific temperate rainforest": Atlantic, 48.0,-124.0,8.0,5.0, Moist;
    "Canadian western boreal": Boreal, 58.0,-119.0,8.0,17.0, Cool;
    "Canadian central boreal": Boreal, 56.0,-94.0,7.0,18.0, Cool;
    "Canadian eastern boreal": Boreal, 54.0,-68.0,8.0,17.0, Cool;
    "Alaska tundra": Tundra, 69.0,-153.0,5.0,17.0, Cold;
    "Canadian Arctic tundra": Tundra, 70.0,-102.0,7.0,30.0, Cold;
    "Greenland ice-free margins": Tundra, 69.0,-45.0,10.0,16.0, Cold;
    "Eastern North American woodland": Continental, 40.0,-80.0,9.0,12.0, Moist;
    "Northern prairie": Prairie, 48.0,-103.0,7.0,10.0, Dry;
    "Central prairie": Prairie, 39.0,-99.0,6.0,8.0, Dry;
    "Great Basin sagebrush": Sagebrush, 40.0,-116.0,5.0,7.0, Dry;
    "Colorado Plateau": Sandstone, 37.5,-110.5,4.0,5.5, Dry;
    "Sonoran drylands": Desert, 30.0,-112.0,5.0,6.0, HotDry;
    "Chihuahuan drylands": Desert, 29.0,-104.0,5.0,5.0, HotDry;
    "California Mediterranean": Mediterranean, 36.0,-120.0,5.0,4.0, Mild;
    "Rocky Mountain meadows": Alpine, 43.0,-110.0,9.0,5.0, Alpine;
    "Sierra Nevada meadows": Alpine, 37.5,-119.0,3.5,2.5, Alpine;
    "Mexican highlands": MexicanUpland, 21.0,-101.0,5.5,6.0, Upland;
    "Mesoamerican humid forests": HumidTropical, 13.0,-85.0,7.0,8.0, TropicalMoist;
    "Amazon west": HumidTropical, -5.0,-71.0,10.0,10.0, TropicalMoist;
    "Amazon central": HumidTropical, -3.0,-59.0,10.0,13.0, TropicalMoist;
    "Guiana shield": HumidTropical, 5.0,-61.0,5.0,9.0, TropicalMoist;
    "Andean cloud forests": CloudForest, -8.0,-75.0,12.0,4.0, HumidUpland;
    "Northern Andean paramo": Alpine, 3.0,-75.0,6.0,4.0, Alpine;
    "Altiplano puna": Puna, -18.0,-68.0,8.0,6.0, Alpine;
    "Cerrado plateau": Cerrado, -15.0,-48.0,9.0,9.0, Warm;
    "Caatinga dry woodland": Caatinga, -8.0,-39.0,6.0,6.0, Dry;
    "Atlantic tropical forest": HumidTropical, -22.0,-43.0,9.0,5.0, TropicalMoist;
    "Pampas grassland": Pampas, -35.0,-61.0,7.0,9.0, Mild;
    "Patagonian steppe": Patagonian, -47.0,-69.0,9.0,7.0, Dry;
    "Valdivian temperate forest": Atlantic, -41.0,-73.0,6.0,3.5, Moist;
    "Central Chile Mediterranean": Mediterranean, -33.0,-71.0,5.0,3.0, Mild;
    "Western Sahara": Desert, 24.0,-6.0,9.0,13.0, HotDry;
    "Central Sahara": Desert, 24.0,15.0,9.0,15.0, HotDry;
    "Eastern Sahara": Desert, 24.0,31.0,9.0,10.0, HotDry;
    "Western Sahel": Sahel, 14.0,-6.0,5.0,12.0, Warm;
    "Central Sahel": Sahel, 13.5,15.0,5.0,14.0, Warm;
    "Congo humid forests": HumidTropical, -1.0,23.0,9.0,13.0, TropicalMoist;
    "Guinean humid forests": HumidTropical, 6.0,-7.0,5.0,10.0, TropicalMoist;
    "East African savanna": Savanna, -3.0,35.0,9.0,9.0, Warm;
    "Ethiopian highlands": Highland, 9.0,38.0,6.0,6.0, Upland;
    "East African highlands": Highland, -1.0,37.0,6.0,5.0, Upland;
    "Miombo woodland": Miombo, -13.0,26.0,9.0,13.0, Warm;
    "Kalahari drylands": Savanna, -24.0,23.0,7.0,8.0, Dry;
    "Namib coastal desert": Desert, -24.0,15.0,7.0,4.0, HotDry;
    "Cape fynbos": Fynbos, -33.5,20.0,3.0,5.5, Mild;
    "Drakensberg grasslands": Highland, -29.0,29.0,4.5,4.0, Upland;
    "Madagascar moist east": HumidTropical, -19.0,48.0,7.0,3.0, TropicalMoist;
    "Arabian interior": Desert, 23.0,47.0,9.0,11.0, HotDry;
    "Zagros dry uplands": Sagebrush, 33.0,48.0,6.0,7.0, Dry;
    "Kazakh steppe": Steppe, 49.0,67.0,7.0,18.0, Dry;
    "Mongolian steppe": Steppe, 47.0,105.0,7.0,17.0, Dry;
    "Tian Shan meadows": Alpine, 42.0,78.0,4.0,10.0, Alpine;
    "Tibetan plateau": Puna, 33.0,88.0,6.0,13.0, Alpine;
    "Himalayan foothills": Monsoon, 28.0,83.0,4.0,13.0, Warm;
    "Himalayan alpine": Alpine, 29.0,85.0,4.0,13.0, Alpine;
    "Indian monsoon lowlands": Monsoon, 23.0,80.0,8.0,10.0, Warm;
    "Deccan plateau": Deccan, 17.0,77.0,6.0,5.0, Warm;
    "Northern Western Ghats": Ghats, 17.0,73.8,4.5,2.0, TropicalMoist;
    "Southern Western Ghats": Ghats, 11.5,76.0,4.0,2.0, TropicalMoist;
    "Northeast India humid uplands": CloudForest, 26.0,93.0,4.0,6.0, HumidUpland;
    "Indochina monsoon": Monsoon, 18.0,102.0,8.0,8.0, Warm;
    "Sundaland humid forest": HumidTropical, 0.0,112.0,9.0,16.0, TropicalMoist;
    "New Guinea humid forest": HumidTropical, -5.0,143.0,6.0,11.0, TropicalMoist;
    "East Asian temperate woodland": EastAsian, 35.0,117.0,9.0,12.0, Moist;
    "Korea and Japan woodland": EastAsian, 38.0,135.0,9.0,11.0, Moist;
    "Amur mixed woodland": Boreal, 49.0,132.0,7.0,13.0, Cool;
    "Australian tropical north": Savanna, -15.0,133.0,7.0,13.0, Warm;
    "Queensland wet tropics": HumidTropical, -17.0,146.0,4.0,3.0, TropicalMoist;
    "Eastern eucalypt woodland": Eucalypt, -30.0,151.0,9.0,6.0, Mild;
    "Southeastern eucalypt woodland": Eucalypt, -36.0,145.0,5.0,8.0, Mild;
    "Southwestern scrub": Mallee, -32.0,118.0,5.0,6.0, Mild;
    "Australian red interior": Spinifex, -24.0,130.0,9.0,14.0, Dry;
    "Tasmanian cool forest": Atlantic, -42.0,146.0,3.0,4.0, Moist;
    "New Zealand eastern tussock": Tussock, -44.0,170.5,4.0,3.5, Mild;
    "New Zealand alpine": Alpine, -43.5,169.8,4.0,3.0, Alpine;
    "New Zealand humid west": Atlantic, -42.0,171.0,5.0,3.0, Moist;
}
