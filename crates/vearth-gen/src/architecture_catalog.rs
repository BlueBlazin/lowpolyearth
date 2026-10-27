//! Administrative classifications shared by road and agricultural generation.
//!
//! Building appearance is resolved by `architecture_resolver` from explicit
//! country, subdivision and local recipes. These broad geographic labels do
//! not provide building palettes, construction mixtures or facade styles.

use vearth_data::admin_regions::AdminRegionContext;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ResidentialForm {
    BrickPitch,
    RenderPitch,
    StonePitch,
    StoneSlab,
    TimberPitch,
    RenderSlab,
    BrickSlab,
    MasonryMetal,
    TimberMetal,
    EarthFlat,
    EarthPitch,
    /// A terrace/parapet treatment; this never inserts a courtyard into a footprint.
    CourtyardRender,
    TropicalRenderPitch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PaletteFamily {
    CoolMasonry,
    WarmMasonry,
    RedBrick,
    Ochre,
    LightRender,
    TropicalPastel,
    Timber,
    Earth,
    Neutral,
}

macro_rules! families {
    ($($name:ident => $label:literal),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum RegionalFamily { $($name),+ }
        pub const REGIONAL_FAMILIES: &[RegionalFamily] = &[$(RegionalFamily::$name),+];
        impl RegionalFamily {
            pub const fn label(self) -> &'static str {
                match self { $(Self::$name => $label),+ }
            }
        }
    };
}

families! {
    AtlanticSlate => "Atlantic stone, render and slate",
    NorthEuropeanBrick => "North European brick and tile",
    JulianAlpine => "Julian Alpine pale render and dark roofs",
    Adriatic => "Adriatic limestone and terracotta",
    Aegean => "South Aegean white render and terraces",
    Maltese => "Maltese limestone and flat roofs",
    PortugueseAtlantic => "Northern Portuguese granite and clay tile",
    Pueblo => "New Mexican render and parapets",
    PeruvianCoast => "Peruvian dry coast brick and concrete",
    ChileanLakes => "Chilean lake district timber and sheet roofs",
    Tamil => "Tamil masonry, terraces and clay tile",
    Gujarati => "Gujarati masonry and rendered courtyards",
    Bhutanese => "Bhutanese pale masonry and rain roofs",
    Hui => "Anhui light render and dark tile",
    Cape => "Western Cape light plaster and sheet roofs",
    Tasmanian => "Tasmanian brick, stone and weatherboard",
    AtlanticEurope => "Atlantic European masonry",
    BritishIsles => "British and Irish masonry",
    Nordic => "Nordic timber and masonry",
    Baltic => "Baltic timber and masonry",
    CentralEurope => "Central European masonry",
    Alpine => "Alpine masonry and timber",
    Mediterranean => "Mediterranean masonry",
    Iberian => "Iberian masonry and render",
    Italian => "Italian masonry and tile",
    Balkan => "Balkan masonry",
    EastEuropean => "Eastern European masonry",
    Caucasus => "Caucasian stone and masonry",
    Anatolian => "Anatolian masonry",
    NorthAmerican => "North American mixed construction",
    NewEngland => "Northeastern timber and brick",
    NorthAmericanSouth => "Southern North American porches",
    PacificNorthwest => "North American Pacific timber and masonry",
    NorthAmericanDry => "Dry North American render",
    Arctic => "Cold-region lightweight construction",
    Quebec => "Quebec masonry and timber",
    Caribbean => "Caribbean masonry and shade",
    MexicanHighland => "Mexican highland masonry",
    MexicanDry => "Northern Mexican masonry",
    MexicanTropical => "Tropical Mexican masonry",
    MesoamericanTropical => "Tropical Mesoamerican masonry",
    Andean => "Andean masonry",
    SouthAmericanTropical => "Tropical South American construction",
    SouthernCone => "Southern Cone masonry",
    Patagonian => "Southern temperate metal and timber",
    BrazilianAtlantic => "Atlantic Brazilian masonry",
    BrazilianNortheast => "Northeastern Brazilian masonry",
    Maghreb => "Maghreb contemporary masonry",
    Saharan => "Dryland masonry and render",
    Nile => "Nile brick and concrete",
    Sahel => "Sahel masonry and sheet roofs",
    WestAfrican => "West African block and metal roofs",
    CentralAfrican => "Central African masonry and rain roofs",
    EastAfrican => "East African masonry and metal roofs",
    Swahili => "East African coastal construction",
    Horn => "Horn of Africa dryland masonry",
    EthiopianHighland => "Ethiopian highland construction",
    SouthAfrican => "Southern African brick and sheet roofs",
    Madagascan => "Malagasy highland masonry",
    IndianOcean => "Indian Ocean island construction",
    GulfArabian => "Arabian contemporary concrete",
    Levantine => "Levantine stone and concrete",
    Iranian => "Iranian brick and render",
    Caspian => "Caspian rain roofs",
    YemeniHighland => "Yemeni highland masonry",
    CentralAsian => "Continental Eurasian masonry and concrete",
    IndianPlain => "South Asian plain masonry",
    Deccan => "Deccan masonry and concrete",
    MonsoonCoast => "South Asian monsoon coast",
    BengalDelta => "Bengal delta masonry and sheet roofs",
    Himalayan => "Himalayan masonry and metal roofs",
    HimalayanDry => "Cold arid mountain masonry",
    NortheastIndian => "Northeast Indian rain roofs",
    ChineseNorth => "Northern Chinese masonry",
    ChineseSouth => "Southern Chinese masonry",
    Japanese => "Japanese contemporary housing",
    JapaneseSnow => "Japanese snow-region housing",
    Ryukyuan => "Ryukyuan concrete and rain roofs",
    Korean => "Korean contemporary masonry",
    SoutheastAsian => "Mainland Southeast Asian construction",
    Malay => "Malay contemporary masonry and shade",
    Indonesian => "Indonesian masonry and tile",
    Philippine => "Philippine masonry and metal roofs",
    Vietnamese => "Vietnamese masonry and shade",
    Australian => "Australian brick and lightweight housing",
    AustralianTropical => "Tropical Australian housing",
    NewZealand => "New Zealand timber and masonry",
    PacificIsland => "Tropical island contemporary construction",
}

#[derive(Clone, Copy)]
struct SubdivisionRule {
    country: &'static str,
    family: RegionalFamily,
    names: &'static [&'static str],
}

// Sorted by source country spelling for allocation-free binary range lookup.
// Exact subdivision names are checked against the bundled atlas by the tests.
const SUBDIVISION_RULES: &[SubdivisionRule] = &[
    SubdivisionRule {
        country: "Algeria",
        family: RegionalFamily::Saharan,
        names: &[
            "Adrar",
            "Béchar",
            "El Oued",
            "Ghardaïa",
            "Illizi",
            "Ouargla",
            "Tamanghasset",
            "Tindouf",
        ],
    },
    SubdivisionRule {
        country: "Argentina",
        family: RegionalFamily::Patagonian,
        names: &[
            "Chubut",
            "Neuquén",
            "Río Negro",
            "Santa Cruz",
            "Tierra del Fuego",
        ],
    },
    SubdivisionRule {
        country: "Argentina",
        family: RegionalFamily::Andean,
        names: &["Catamarca", "Jujuy", "La Rioja", "Salta"],
    },
    SubdivisionRule {
        country: "Argentina",
        family: RegionalFamily::SouthAmericanTropical,
        names: &["Chaco", "Corrientes", "Formosa", "Misiones"],
    },
    SubdivisionRule {
        country: "Australia",
        family: RegionalFamily::AustralianTropical,
        names: &["Northern Territory", "Queensland", "Lord Howe Island"],
    },
    SubdivisionRule {
        country: "Australia",
        family: RegionalFamily::Arctic,
        names: &["Macquarie Island"],
    },
    SubdivisionRule {
        country: "Australia",
        family: RegionalFamily::Tasmanian,
        names: &["Tasmania"],
    },
    SubdivisionRule {
        country: "Brazil",
        family: RegionalFamily::SouthAmericanTropical,
        names: &["Acre", "Amapá", "Amazonas", "Pará", "Rondônia", "Roraima"],
    },
    SubdivisionRule {
        country: "Brazil",
        family: RegionalFamily::BrazilianNortheast,
        names: &[
            "Alagoas",
            "Bahia",
            "Ceará",
            "Maranhão",
            "Paraíba",
            "Pernambuco",
            "Piauí",
            "Rio Grande do Norte",
            "Sergipe",
        ],
    },
    SubdivisionRule {
        country: "Brazil",
        family: RegionalFamily::SouthernCone,
        names: &["Paraná", "Rio Grande do Sul", "Santa Catarina"],
    },
    SubdivisionRule {
        country: "Cameroon",
        family: RegionalFamily::Sahel,
        names: &["Far North", "North"],
    },
    SubdivisionRule {
        country: "Canada",
        family: RegionalFamily::Quebec,
        names: &["Quebec"],
    },
    SubdivisionRule {
        country: "Canada",
        family: RegionalFamily::PacificNorthwest,
        names: &["British Columbia"],
    },
    SubdivisionRule {
        country: "Canada",
        family: RegionalFamily::NewEngland,
        names: &[
            "New Brunswick",
            "Newfoundland and Labrador",
            "Nova Scotia",
            "Prince Edward Island",
        ],
    },
    SubdivisionRule {
        country: "Canada",
        family: RegionalFamily::Arctic,
        names: &["Northwest Territories", "Nunavut", "Yukon"],
    },
    SubdivisionRule {
        country: "Chile",
        family: RegionalFamily::Patagonian,
        names: &["Araucanía", "Aysén", "Magellan and the Chilean Antarctic"],
    },
    SubdivisionRule {
        country: "Chile",
        family: RegionalFamily::Andean,
        names: &["Antofagasta", "Arica y Parinacota", "Atacama", "Tarapacá"],
    },
    SubdivisionRule {
        country: "Chile",
        family: RegionalFamily::ChileanLakes,
        names: &["Los Lagos", "Los Ríos"],
    },
    SubdivisionRule {
        country: "China",
        family: RegionalFamily::ChineseSouth,
        names: &[
            "Chongqing",
            "Fujian",
            "Guangdong",
            "Guangxi",
            "Guizhou",
            "Hainan",
            "Hubei",
            "Hunan",
            "Jiangsu",
            "Jiangxi",
            "Shanghai",
            "Sichuan",
            "Yunnan",
            "Zhejiang",
        ],
    },
    SubdivisionRule {
        country: "China",
        family: RegionalFamily::HimalayanDry,
        names: &["Tibet", "Qinghai"],
    },
    SubdivisionRule {
        country: "China",
        family: RegionalFamily::CentralAsian,
        names: &["Xinjiang", "Gansu", "Ningxia", "Inner Mongolia"],
    },
    SubdivisionRule {
        country: "China",
        family: RegionalFamily::PacificIsland,
        names: &["Paracel Islands"],
    },
    SubdivisionRule {
        country: "China",
        family: RegionalFamily::Hui,
        names: &["Anhui"],
    },
    SubdivisionRule {
        country: "Croatia",
        family: RegionalFamily::Adriatic,
        names: &[
            "Dubrovnik-Neretva",
            "Split-Dalmatia",
            "Šibenik-Knin",
            "Zadar",
            "Istria",
        ],
    },
    SubdivisionRule {
        country: "Ethiopia",
        family: RegionalFamily::Horn,
        names: &["Afar", "Dire Dawa", "Somali"],
    },
    SubdivisionRule {
        country: "France",
        family: RegionalFamily::Caribbean,
        names: &["Guadeloupe", "Martinique"],
    },
    SubdivisionRule {
        country: "France",
        family: RegionalFamily::SouthAmericanTropical,
        names: &["French Guiana"],
    },
    SubdivisionRule {
        country: "France",
        family: RegionalFamily::IndianOcean,
        names: &["Mayotte", "Réunion"],
    },
    SubdivisionRule {
        country: "France",
        family: RegionalFamily::Mediterranean,
        names: &[
            "Alpes-Maritimes",
            "Alpes-de-Haute-Provence",
            "Aude",
            "Bouches-du-Rhône",
            "Corse-du-Sud",
            "Gard",
            "Haute-Corse",
            "Hérault",
            "Pyrénées-Orientales",
            "Var",
            "Vaucluse",
        ],
    },
    SubdivisionRule {
        country: "France",
        family: RegionalFamily::Alpine,
        names: &["Haute-Savoie", "Hautes-Alpes", "Isère", "Savoie"],
    },
    SubdivisionRule {
        country: "France",
        family: RegionalFamily::CentralEurope,
        names: &["Bas-Rhin", "Haut-Rhin", "Moselle"],
    },
    SubdivisionRule {
        country: "France",
        family: RegionalFamily::AtlanticSlate,
        names: &["Finistère", "Côtes-d'Armor", "Morbihan", "Ille-et-Vilaine"],
    },
    SubdivisionRule {
        country: "French Southern and Antarctic Lands",
        family: RegionalFamily::IndianOcean,
        names: &["Scattered Islands in the Indian Ocean"],
    },
    SubdivisionRule {
        country: "Germany",
        family: RegionalFamily::Alpine,
        names: &["Bavaria"],
    },
    SubdivisionRule {
        country: "Germany",
        family: RegionalFamily::NorthEuropeanBrick,
        names: &[
            "Free Hanseatic Bremen",
            "Hamburg",
            "Mecklenburg-Western Pomerania",
            "Schleswig-Holstein",
        ],
    },
    SubdivisionRule {
        country: "Greece",
        family: RegionalFamily::Aegean,
        names: &["South Aegean"],
    },
    SubdivisionRule {
        country: "India",
        family: RegionalFamily::Himalayan,
        names: &[
            "Himachal Pradesh",
            "Jammu and Kashmir",
            "Uttarakhand",
            "Sikkim",
        ],
    },
    SubdivisionRule {
        country: "India",
        family: RegionalFamily::HimalayanDry,
        names: &["Ladakh"],
    },
    SubdivisionRule {
        country: "India",
        family: RegionalFamily::NortheastIndian,
        names: &[
            "Arunachal Pradesh",
            "Assam",
            "Manipur",
            "Meghalaya",
            "Mizoram",
            "Nagaland",
            "Tripura",
        ],
    },
    SubdivisionRule {
        country: "India",
        family: RegionalFamily::Deccan,
        names: &[
            "Andhra Pradesh",
            "Chhattisgarh",
            "Karnataka",
            "Madhya Pradesh",
            "Maharashtra",
            "Telangana",
        ],
    },
    SubdivisionRule {
        country: "India",
        family: RegionalFamily::MonsoonCoast,
        names: &[
            "Goa",
            "Kerala",
            "Andaman and Nicobar Islands",
            "Lakshadweep",
        ],
    },
    SubdivisionRule {
        country: "India",
        family: RegionalFamily::BengalDelta,
        names: &["West Bengal"],
    },
    SubdivisionRule {
        country: "India",
        family: RegionalFamily::Saharan,
        names: &["Rajasthan"],
    },
    SubdivisionRule {
        country: "India",
        family: RegionalFamily::Tamil,
        names: &["Tamil Nadu", "Puducherry"],
    },
    SubdivisionRule {
        country: "India",
        family: RegionalFamily::Gujarati,
        names: &["Gujarat"],
    },
    SubdivisionRule {
        country: "Indonesia",
        family: RegionalFamily::PacificIsland,
        names: &[
            "Maluku",
            "North Maluku",
            "Papua",
            "West Papua",
            "East Nusa Tenggara",
        ],
    },
    SubdivisionRule {
        country: "Indonesia",
        family: RegionalFamily::Malay,
        names: &[
            "Aceh",
            "Bangka Belitung Islands",
            "Bengkulu",
            "Central Kalimantan",
            "East Kalimantan",
            "Jambi",
            "North Sumatra",
            "Riau",
            "Riau Islands",
            "South Kalimantan",
            "South Sumatra",
            "West Kalimantan",
            "West Sumatra",
        ],
    },
    SubdivisionRule {
        country: "Iran",
        family: RegionalFamily::Caspian,
        names: &["Gilan", "Golestan", "Mazandaran"],
    },
    SubdivisionRule {
        country: "Italy",
        family: RegionalFamily::Alpine,
        names: &[
            "Aosta",
            "Belluno",
            "Sondrio",
            "Trentino",
            "Trentino-South Tyrol",
            "Verbano-Cusio-Ossola",
        ],
    },
    SubdivisionRule {
        country: "Japan",
        family: RegionalFamily::JapaneseSnow,
        names: &[
            "Akita Prefecture",
            "Aomori Prefecture",
            "Fukui Prefecture",
            "Hokkaidō",
            "Ishikawa Prefecture",
            "Iwate Prefecture",
            "Niigata Prefecture",
            "Toyama Prefecture",
            "Yamagata Prefecture",
        ],
    },
    SubdivisionRule {
        country: "Japan",
        family: RegionalFamily::Ryukyuan,
        names: &["Okinawa Prefecture"],
    },
    SubdivisionRule {
        country: "Kenya",
        family: RegionalFamily::Swahili,
        names: &["Coast"],
    },
    SubdivisionRule {
        country: "Kenya",
        family: RegionalFamily::Horn,
        names: &["North Eastern"],
    },
    SubdivisionRule {
        country: "Madagascar",
        family: RegionalFamily::IndianOcean,
        names: &[
            "Anosy",
            "Atsimo-Atsinanana",
            "Diana",
            "Sava",
            "Toamasina",
            "Vatovavy-Fitovinany",
        ],
    },
    SubdivisionRule {
        country: "Malaysia",
        family: RegionalFamily::PacificIsland,
        names: &["Sabah", "Sarawak", "Labuan"],
    },
    SubdivisionRule {
        country: "Mexico",
        family: RegionalFamily::MexicanDry,
        names: &[
            "Baja California",
            "Baja California Sur",
            "Chihuahua",
            "Coahuila",
            "Durango",
            "Nuevo León",
            "Sonora",
        ],
    },
    SubdivisionRule {
        country: "Mexico",
        family: RegionalFamily::MexicanTropical,
        names: &[
            "Campeche",
            "Chiapas",
            "Colima",
            "Guerrero",
            "Nayarit",
            "Quintana Roo",
            "Tabasco",
            "Veracruz",
            "Yucatán",
        ],
    },
    SubdivisionRule {
        country: "Morocco",
        family: RegionalFamily::Saharan,
        names: &[
            "Guelmim-Es Semara",
            "Laâyoune-Boujdour-Sakia El Hamra",
            "Oued Ed-Dahab",
        ],
    },
    SubdivisionRule {
        country: "New Zealand",
        family: RegionalFamily::PacificIsland,
        names: &["Tokelau", "Kermadec Islands"],
    },
    SubdivisionRule {
        country: "New Zealand",
        family: RegionalFamily::Arctic,
        names: &[
            "Antipodes Islands",
            "Auckland Islands",
            "Campbell Islands",
            "Snares Islands",
        ],
    },
    SubdivisionRule {
        country: "Nigeria",
        family: RegionalFamily::Sahel,
        names: &[
            "Bauchi State",
            "Borno State",
            "Gombe State",
            "Jigawa State",
            "Kano State",
            "Katsina State",
            "Kebbi State",
            "Sokoto State",
            "Yobe State",
            "Zamfara State",
        ],
    },
    SubdivisionRule {
        country: "Norway",
        family: RegionalFamily::Arctic,
        names: &["Svalbard", "Bouvet Island"],
    },
    SubdivisionRule {
        country: "Peru",
        family: RegionalFamily::SouthAmericanTropical,
        names: &[
            "Amazonas",
            "Loreto",
            "Madre de Dios",
            "San Martín",
            "Ucayali",
        ],
    },
    SubdivisionRule {
        country: "Peru",
        family: RegionalFamily::PeruvianCoast,
        names: &["Lima", "Callao", "Ica", "Lambayeque", "La Libertad"],
    },
    SubdivisionRule {
        country: "Portugal",
        family: RegionalFamily::Mediterranean,
        names: &["Madeira"],
    },
    SubdivisionRule {
        country: "Portugal",
        family: RegionalFamily::PortugueseAtlantic,
        names: &[
            "Braga",
            "Bragança",
            "Porto",
            "Viana do Castelo",
            "Vila Real",
        ],
    },
    SubdivisionRule {
        country: "Portugal",
        family: RegionalFamily::Iberian,
        names: &["Azores"],
    },
    SubdivisionRule {
        country: "Russia",
        family: RegionalFamily::Arctic,
        names: &[
            "Chukotka Autonomous Okrug",
            "Magadan",
            "Murmansk",
            "Nenets Autonomous Okrug",
            "Sakha Republic",
            "Yamalo-Nenets Autonomous Okrug",
        ],
    },
    SubdivisionRule {
        country: "Russia",
        family: RegionalFamily::CentralAsian,
        names: &[
            "Altai Republic",
            "Amur",
            "Irkutsk",
            "Jewish",
            "Kemerovo",
            "Khabarovsk Krai",
            "Khanty-Mansi Autonomous Okrug",
            "Krasnoyarsk Krai",
            "Novosibirsk",
            "Omsk",
            "Republic of Buryatia",
            "Republic of Khakassia",
            "Tomsk",
            "Tuva Republic",
            "Tyumen",
            "Zabaykalsky Krai",
        ],
    },
    SubdivisionRule {
        country: "Russia",
        family: RegionalFamily::EastEuropean,
        names: &["Kamchatka Krai", "Primorsky Krai", "Sakhalin"],
    },
    SubdivisionRule {
        country: "Russia",
        family: RegionalFamily::Caucasus,
        names: &[
            "Chechen Republic",
            "Kabardino-Balkaria",
            "Karachay-Cherkess Republic",
            "Republic of Adygea",
            "Republic of Dagestan",
            "Republic of Ingushetia",
            "Republic of North Ossetia-Alania",
        ],
    },
    SubdivisionRule {
        country: "Russia",
        family: RegionalFamily::Mediterranean,
        names: &[
            "Autonomous Republic of Crimea",
            "Krasnodar Krai",
            "Sevastopol",
        ],
    },
    SubdivisionRule {
        country: "Slovenia",
        family: RegionalFamily::JulianAlpine,
        names: &[
            "Bled",
            "Bohinj",
            "Jesenice",
            "Žirovnica",
            "Kranjska Gora",
            "Radovljica",
            "Kranj",
            "Naklo",
            "Tržič",
            "Preddvor",
            "Jezersko",
            "Škofja Loka",
            "Železniki",
        ],
    },
    SubdivisionRule {
        country: "Slovenia",
        family: RegionalFamily::Adriatic,
        names: &[
            "Piran",
            "Izola",
            "Koper",
            "Komen",
            "Sežana",
            "Divača",
            "Hrpelje-Kozina",
            "Miren–Kostanjevica",
            "Nova Gorica",
            "Šempeter–Vrtojba",
            "Brda",
            "Vipava",
            "Ajdovščina",
            "Kobarid",
            "Tolmin",
        ],
    },
    SubdivisionRule {
        country: "Slovenia",
        family: RegionalFamily::Alpine,
        names: &["Bovec", "Solčava", "Luče", "Kamnik", "Gornji Grad"],
    },
    SubdivisionRule {
        country: "South Africa",
        family: RegionalFamily::Cape,
        names: &["Western Cape"],
    },
    SubdivisionRule {
        country: "Spain",
        family: RegionalFamily::AtlanticEurope,
        names: &[
            "A Coruña",
            "Araba / Álava",
            "Asturias",
            "Biscay",
            "Cantabria",
            "Gipuzkoa",
            "Lugo",
            "Navarre",
            "Ourense",
            "Pontevedra",
        ],
    },
    SubdivisionRule {
        country: "Spain",
        family: RegionalFamily::Mediterranean,
        names: &["Balearic Islands", "Las Palmas", "Santa Cruz de Tenerife"],
    },
    SubdivisionRule {
        country: "Spain",
        family: RegionalFamily::Maghreb,
        names: &["Ceuta", "Melilla"],
    },
    SubdivisionRule {
        country: "Turkey",
        family: RegionalFamily::Mediterranean,
        names: &[
            "Adana",
            "Antalya",
            "Aydın",
            "Balıkesir",
            "Hatay",
            "Mersin",
            "Muğla",
            "Çanakkale",
            "İzmir",
        ],
    },
    SubdivisionRule {
        country: "Turkey",
        family: RegionalFamily::Caspian,
        names: &[
            "Artvin", "Giresun", "Ordu", "Rize", "Samsun", "Sinop", "Trabzon",
        ],
    },
    SubdivisionRule {
        country: "Turkey",
        family: RegionalFamily::Caucasus,
        names: &[
            "Ardahan", "Ağrı", "Erzurum", "Hakkâri", "Iğdır", "Kars", "Van",
        ],
    },
    SubdivisionRule {
        country: "United Kingdom",
        family: RegionalFamily::AtlanticSlate,
        names: &[
            "Cornwall",
            "Devon",
            "Plymouth",
            "Torbay",
            "Cumbria",
            "Gwynedd",
            "Conwy",
            "Anglesey",
            "Ceredigion",
            "Pembrokeshire",
            "Powys",
            "Aberdeen",
            "Aberdeenshire",
            "Moray",
            "Highland",
            "Outer Hebrides",
            "Orkney Islands",
            "Shetland Islands",
            "Argyll and Bute",
            "Perth and Kinross",
            "Angus",
            "Dundee",
            "Fife",
            "Stirling",
            "Falkirk",
            "Clackmannanshire",
            "Edinburgh",
            "East Lothian",
            "Midlothian",
            "West Lothian",
            "Scottish Borders",
            "Dumfries and Galloway",
            "Glasgow",
            "East Ayrshire",
            "North Ayrshire",
            "South Ayrshire",
            "East Dunbartonshire",
            "West Dunbartonshire",
            "East Renfrewshire",
            "Renfrewshire",
            "Inverclyde",
            "North Lanarkshire",
            "South Lanarkshire",
        ],
    },
    SubdivisionRule {
        country: "United Republic of Tanzania",
        family: RegionalFamily::Swahili,
        names: &[
            "Dar es Salaam",
            "Lindi",
            "Mjini Magharibi",
            "Mtwara",
            "North Pemba",
            "Pwani",
            "South Pemba",
            "Tanga",
            "Unguja North",
            "Unguja South",
        ],
    },
    SubdivisionRule {
        country: "United States Minor Outlying Islands",
        family: RegionalFamily::Caribbean,
        names: &["Navassa Island"],
    },
    SubdivisionRule {
        country: "United States of America",
        family: RegionalFamily::NewEngland,
        names: &[
            "Connecticut",
            "Maine",
            "Massachusetts",
            "New Hampshire",
            "Rhode Island",
            "Vermont",
        ],
    },
    SubdivisionRule {
        country: "United States of America",
        family: RegionalFamily::NorthAmericanSouth,
        names: &[
            "Alabama",
            "Arkansas",
            "Florida",
            "Georgia",
            "Kentucky",
            "Louisiana",
            "Mississippi",
            "North Carolina",
            "South Carolina",
            "Tennessee",
        ],
    },
    SubdivisionRule {
        country: "United States of America",
        family: RegionalFamily::NorthAmericanDry,
        names: &["Arizona", "Nevada", "Utah"],
    },
    SubdivisionRule {
        country: "United States of America",
        family: RegionalFamily::PacificNorthwest,
        names: &["Oregon"],
    },
    SubdivisionRule {
        country: "United States of America",
        family: RegionalFamily::Arctic,
        names: &["Alaska"],
    },
    SubdivisionRule {
        country: "United States of America",
        family: RegionalFamily::PacificIsland,
        names: &["Hawaii"],
    },
    SubdivisionRule {
        country: "United States of America",
        family: RegionalFamily::Pueblo,
        names: &["New Mexico"],
    },
    SubdivisionRule {
        country: "Yemen",
        family: RegionalFamily::GulfArabian,
        names: &[
            "'Adan",
            "Abyan",
            "Al Hudaydah",
            "Al Mahrah",
            "Hadhramaut",
            "Shabwah",
        ],
    },
];

/// Resolves source administrative geography into an everyday construction prior.
/// Unknown future country names deliberately return `None` for the caller's
/// neutral, climate-aware handling rather than inheriting another country's art.
pub fn family_for_context(context: AdminRegionContext) -> Option<RegionalFamily> {
    use RegionalFamily::*;
    // The source names Washington state and the District of Columbia identically.
    if context.country_name == "United States of America" && context.id == 1_159_309_547 {
        return Some(PacificNorthwest);
    }
    let start = SUBDIVISION_RULES.partition_point(|rule| rule.country < context.country_name);
    for rule in &SUBDIVISION_RULES[start..] {
        if rule.country != context.country_name {
            break;
        }
        if rule.names.contains(&context.name) {
            return Some(rule.family);
        }
    }
    country_family(context.country_name)
}

/// Default national construction prior before subdivision refinements.
pub fn country_family(country: &str) -> Option<RegionalFamily> {
    use RegionalFamily::*;
    Some(match country {
        "Belgium"
        | "France"
        | "Luxembourg"
        | "Netherlands"
        | "Saint Pierre and Miquelon"
        | "Saint Helena" => AtlanticEurope,
        "United Kingdom" | "Ireland" | "Guernsey" | "Jersey" | "Isle of Man" => BritishIsles,
        "Aland" | "Denmark" | "Faroe Islands" | "Finland" | "Iceland" | "Norway" | "Sweden" => {
            Nordic
        }
        "Estonia" | "Latvia" | "Lithuania" => Baltic,
        "Czech Republic" | "Germany" | "Hungary" | "Poland" | "Slovakia" => CentralEurope,
        "Andorra" | "Austria" | "Liechtenstein" | "Switzerland" => Alpine,
        "Akrotiri Sovereign Base Area"
        | "Cyprus"
        | "Dhekelia Sovereign Base Area"
        | "Greece"
        | "Monaco"
        | "Northern Cyprus" => Mediterranean,
        "Malta" => Maltese,
        "Gibraltar" | "Portugal" | "Spain" => Iberian,
        "Italy" | "San Marino" | "Vatican" => Italian,
        "Albania"
        | "Bosnia and Herzegovina"
        | "Bulgaria"
        | "Croatia"
        | "Kosovo"
        | "Macedonia"
        | "Montenegro"
        | "Republic of Serbia"
        | "Romania" => Balkan,
        "Slovenia" => CentralEurope,
        "Belarus" | "Moldova" | "Russia" | "Ukraine" => EastEuropean,
        "Armenia" | "Azerbaijan" | "Georgia" => Caucasus,
        "Turkey" => Anatolian,
        "Canada" | "United States of America" => NorthAmerican,
        "Anguilla"
        | "Antigua and Barbuda"
        | "Aruba"
        | "Barbados"
        | "Bermuda"
        | "British Virgin Islands"
        | "Caribbean Netherlands"
        | "Cayman Islands"
        | "Cuba"
        | "Curaçao"
        | "Dominica"
        | "Dominican Republic"
        | "Grenada"
        | "Haiti"
        | "Jamaica"
        | "Montserrat"
        | "Puerto Rico"
        | "Saint Barthelemy"
        | "Saint Kitts and Nevis"
        | "Saint Lucia"
        | "Saint Martin"
        | "Saint Vincent and the Grenadines"
        | "Sint Maarten"
        | "The Bahamas"
        | "Trinidad and Tobago"
        | "Turks and Caicos Islands"
        | "United States Virgin Islands"
        | "US Naval Base Guantanamo Bay" => Caribbean,
        "Mexico" => MexicanHighland,
        "Belize" | "Costa Rica" | "El Salvador" | "Guatemala" | "Honduras" | "Nicaragua"
        | "Panama" => MesoamericanTropical,
        "Bolivia" | "Ecuador" | "Peru" => Andean,
        "Colombia" | "Guyana" | "Suriname" | "Venezuela" => SouthAmericanTropical,
        "Argentina" | "Chile" | "Paraguay" | "Uruguay" => SouthernCone,
        "Falkland Islands" => Patagonian,
        "Brazil" => BrazilianAtlantic,
        "Algeria" | "Libya" | "Morocco" | "Tunisia" | "Cape Verde" => Maghreb,
        "Western Sahara" => Saharan,
        "Egypt" | "Sudan" => Nile,
        "Burkina Faso" | "Chad" | "Mali" | "Mauritania" | "Niger" => Sahel,
        "Benin" | "Gambia" | "Ghana" | "Guinea" | "Guinea Bissau" | "Ivory Coast" | "Liberia"
        | "Nigeria" | "Senegal" | "Sierra Leone" | "Togo" => WestAfrican,
        "Angola"
        | "Cameroon"
        | "Central African Republic"
        | "Democratic Republic of the Congo"
        | "Equatorial Guinea"
        | "Gabon"
        | "Republic of the Congo"
        | "Sao Tome and Principe"
        | "S. Sudan" => CentralAfrican,
        "Burundi"
        | "Kenya"
        | "Malawi"
        | "Rwanda"
        | "Uganda"
        | "United Republic of Tanzania"
        | "Zambia" => EastAfrican,
        "Mozambique" => Swahili,
        "Djibouti" | "Somalia" | "Somaliland" => Horn,
        "Eritrea" | "Ethiopia" => EthiopianHighland,
        "Botswana" | "Lesotho" | "Namibia" | "South Africa" | "Swaziland" | "Zimbabwe" => {
            SouthAfrican
        }
        "Madagascar" => Madagascan,
        "British Indian Ocean Territory"
        | "Comoros"
        | "Indian Ocean Territories"
        | "Maldives"
        | "Mauritius"
        | "Seychelles" => IndianOcean,
        "Bahrain" | "Kuwait" | "Oman" | "Qatar" | "Saudi Arabia" | "United Arab Emirates" => {
            GulfArabian
        }
        "Gaza Strip" | "Israel" | "Jordan" | "Lebanon" | "Syria" | "West Bank" => Levantine,
        "Iran" | "Iraq" => Iranian,
        "Yemen" => YemeniHighland,
        "Afghanistan"
        | "Baykonur Cosmodrome"
        | "Kazakhstan"
        | "Kyrgyzstan"
        | "Mongolia"
        | "Tajikistan"
        | "Turkmenistan"
        | "Uzbekistan" => CentralAsian,
        "India" | "Pakistan" => IndianPlain,
        "Sri Lanka" => MonsoonCoast,
        "Bangladesh" => BengalDelta,
        "Bhutan" => Bhutanese,
        "Nepal" => Himalayan,
        "Siachen Glacier" => HimalayanDry,
        "China" => ChineseNorth,
        "Hong Kong S.A.R." | "Macau S.A.R" | "Taiwan" => ChineseSouth,
        "Japan" => Japanese,
        "North Korea" | "South Korea" => Korean,
        "Cambodia" | "Laos" | "Myanmar" | "Thailand" => SoutheastAsian,
        "Brunei" | "Malaysia" | "Singapore" => Malay,
        "Indonesia" => Indonesian,
        "Philippines" => Philippine,
        "Vietnam" => Vietnamese,
        "Australia" | "Norfolk Island" => Australian,
        "Ashmore and Cartier Islands" | "Coral Sea Islands" => AustralianTropical,
        "New Zealand" => NewZealand,
        "American Samoa"
        | "Clipperton Island"
        | "Cook Islands"
        | "East Timor"
        | "Federated States of Micronesia"
        | "Fiji"
        | "French Polynesia"
        | "Guam"
        | "Kiribati"
        | "Marshall Islands"
        | "Nauru"
        | "New Caledonia"
        | "Niue"
        | "Northern Mariana Islands"
        | "Palau"
        | "Papua New Guinea"
        | "Pitcairn Islands"
        | "Samoa"
        | "Solomon Islands"
        | "Spratly Is."
        | "Tonga"
        | "Tuvalu"
        | "United States Minor Outlying Islands"
        | "Vanuatu"
        | "Wallis and Futuna" => PacificIsland,
        "Antarctica"
        | "French Southern and Antarctic Lands"
        | "Greenland"
        | "Heard Island and McDonald Islands"
        | "South Georgia and the Islands" => Arctic,

        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeSet, HashSet};
    use vearth_data::admin_regions::AdminRegionAtlas;

    fn resolved(country: &str, name: &str) -> RegionalFamily {
        let context = AdminRegionAtlas::bundled()
            .contexts()
            .find(|context| context.country_name == country && context.name == name)
            .unwrap_or_else(|| panic!("missing source subdivision {country}/{name}"));
        family_for_context(context).unwrap()
    }

    #[test]
    fn every_bundled_country_and_subdivision_has_an_explicit_prior() {
        let atlas = AdminRegionAtlas::bundled();
        let mut countries = BTreeSet::new();
        let mut families = HashSet::new();
        for context in atlas.contexts() {
            countries.insert(context.country_name);
            families.insert(family_for_context(context).unwrap_or_else(|| {
                panic!(
                    "uncovered source geography: {}/{} ({})",
                    context.country_name, context.name, context.id
                )
            }));
        }
        assert_eq!(countries.len(), 253);
        assert_eq!(atlas.contexts().len(), 4_589);
        assert_eq!(families.len(), REGIONAL_FAMILIES.len());
    }

    #[test]
    fn every_subdivision_override_resolves_and_no_rule_shadows_another() {
        let contexts: Vec<_> = AdminRegionAtlas::bundled().contexts().collect();
        let mut names = HashSet::new();
        assert!(
            SUBDIVISION_RULES
                .windows(2)
                .all(|pair| pair[0].country <= pair[1].country)
        );
        for rule in SUBDIVISION_RULES {
            assert!(!rule.names.is_empty());
            for name in rule.names {
                assert!(names.insert((rule.country, *name)));
                let matching: Vec<_> = contexts
                    .iter()
                    .filter(|context| context.country_name == rule.country && context.name == *name)
                    .collect();
                assert!(
                    !matching.is_empty(),
                    "stale subdivision override: {}/{name}",
                    rule.country
                );
                for context in matching {
                    assert_eq!(family_for_context(*context), Some(rule.family));
                }
            }
        }
    }

    #[test]
    fn large_countries_and_overseas_territories_retain_regional_differences() {
        use RegionalFamily::*;
        for (country, name, expected) in [
            ("India", "Maharashtra", Deccan),
            ("India", "Kerala", MonsoonCoast),
            ("India", "Assam", NortheastIndian),
            ("India", "Ladakh", HimalayanDry),
            ("China", "Guangdong", ChineseSouth),
            ("China", "Beijing", ChineseNorth),
            ("China", "Tibet", HimalayanDry),
            ("United States of America", "Maine", NewEngland),
            ("United States of America", "Arizona", NorthAmericanDry),
            ("Canada", "Quebec", Quebec),
            ("Canada", "Nunavut", Arctic),
            ("Brazil", "Amazonas", SouthAmericanTropical),
            ("Brazil", "Rio Grande do Sul", SouthernCone),
            ("Australia", "Northern Territory", AustralianTropical),
            ("Australia", "Victoria", Australian),
            ("Indonesia", "Bali", Indonesian),
            ("Indonesia", "Papua", PacificIsland),
            ("Mexico", "Sonora", MexicanDry),
            ("Mexico", "Yucatán", MexicanTropical),
            ("Russia", "Moscow", EastEuropean),
            ("Russia", "Sakha Republic", Arctic),
            ("Japan", "Okinawa Prefecture", Ryukyuan),
            ("Nigeria", "Borno State", Sahel),
            ("Nigeria", "Lagos", WestAfrican),
            ("France", "French Guiana", SouthAmericanTropical),
            ("France", "Réunion", IndianOcean),
            ("New Zealand", "Tokelau", PacificIsland),
            (
                "United States Minor Outlying Islands",
                "Navassa Island",
                Caribbean,
            ),
        ] {
            assert_eq!(resolved(country, name), expected, "{country}/{name}");
        }
    }

    #[test]
    fn researched_material_regions_remain_distinct_from_country_defaults() {
        use RegionalFamily::*;
        for (country, name, expected) in [
            ("United Kingdom", "Cornwall", AtlanticSlate),
            ("United Kingdom", "Edinburgh", AtlanticSlate),
            ("United Kingdom", "Kent", BritishIsles),
            ("France", "Finistère", AtlanticSlate),
            ("France", "Savoie", Alpine),
            ("Germany", "Hamburg", NorthEuropeanBrick),
            ("Slovenia", "Bled", JulianAlpine),
            ("Slovenia", "Bohinj", JulianAlpine),
            ("Slovenia", "Piran", Adriatic),
            ("Slovenia", "Tolmin", Adriatic),
            ("Slovenia", "Bovec", Alpine),
            ("Slovenia", "Murska Sobota", CentralEurope),
            ("Croatia", "Dubrovnik-Neretva", Adriatic),
            ("Croatia", "Zagreb", Balkan),
            ("Greece", "South Aegean", Aegean),
            ("Greece", "Attica", Mediterranean),
            ("Portugal", "Porto", PortugueseAtlantic),
            ("Portugal", "Azores", Iberian),
            ("United States of America", "New Mexico", Pueblo),
            ("United States of America", "Arizona", NorthAmericanDry),
            ("Peru", "Lima", PeruvianCoast),
            ("Peru", "Cusco Departament", Andean),
            ("Chile", "Los Lagos", ChileanLakes),
            ("Chile", "Aysén", Patagonian),
            ("India", "Tamil Nadu", Tamil),
            ("India", "Puducherry", Tamil),
            ("India", "Gujarat", Gujarati),
            ("India", "Kerala", MonsoonCoast),
            ("Bhutan", "Thimphu", Bhutanese),
            ("China", "Anhui", Hui),
            ("China", "Guangdong", ChineseSouth),
            ("South Africa", "Western Cape", Cape),
            ("South Africa", "Gauteng", SouthAfrican),
            ("Australia", "Tasmania", Tasmanian),
            ("Australia", "Victoria", Australian),
        ] {
            assert_eq!(resolved(country, name), expected, "{country}/{name}");
        }
    }

    #[test]
    fn identically_named_washington_records_use_their_source_identity() {
        let contexts: Vec<_> = AdminRegionAtlas::bundled()
            .contexts()
            .filter(|context| {
                context.country_name == "United States of America" && context.name == "Washington"
            })
            .collect();
        assert_eq!(contexts.len(), 2);
        for context in contexts {
            let expected = if context.id == 1_159_309_547 {
                RegionalFamily::PacificNorthwest
            } else {
                RegionalFamily::NorthAmerican
            };
            assert_eq!(family_for_context(context), Some(expected));
        }
    }

    #[test]
    fn missing_geography_does_not_silently_inherit_a_region() {
        assert_eq!(
            family_for_context(AdminRegionContext {
                id: 0,
                country_name: "unmapped country",
                name: "Assam",
            }),
            None
        );
    }
}
