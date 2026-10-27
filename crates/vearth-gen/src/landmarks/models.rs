//! Curated geographic placement and source replacement footprints.
use super::{Base, Landmark, Suppress};
const SUPPRESS_ALL: Suppress = Suppress {
    buildings: true,
    roads: true,
    trees: true,
};
const SUPPRESS_BLDG: Suppress = Suppress {
    buildings: true,
    roads: false,
    trees: true,
};

const EXISTING_LANDMARKS: &[Landmark] = &[
    Landmark {
        name: "sydney-opera-house",
        lat_deg: -33.85712,
        lon_deg: 151.21510,
        heading_deg: 20.0, // podium axis toward the harbor tip
        // Align the authored 3.1 m concourse with clean perimeter terrain;
        // model y=0 is the source sea datum.
        base: Base::TerrainOffset(-3.1),
        bbox_min: [-100.0, -20.0, -116.0],
        bbox_max: [100.0, 100.0, 116.0],
        suppress: SUPPRESS_ALL,
        suppress_bbox_xz: None,
    },
    Landmark {
        name: "sydney-harbour-bridge",
        // Anchor/heading fitted to the OSM Bradfield Highway *over-water*
        // vertices (least-squares axis, both carriageways within ±15 m of
        // it). The first fit used the full way's endpoints — the curving
        // southern approach ramp dragged that chord 120+ m off the true
        // span and the whole model missed the real crossing.
        lat_deg: -33.85182,
        lon_deg: 151.21093,
        heading_deg: 26.8,
        base: Base::Fixed(0.0),
        bbox_min: [-30.0, 0.0, -580.0],
        bbox_max: [30.0, 137.0, 580.0],
        suppress: Suppress {
            buildings: true,
            roads: true,
            trees: false,
        },
        suppress_bbox_xz: Some(([-26.0, -580.0], [26.0, 580.0])),
    },
    Landmark {
        name: "eiffel-tower",
        lat_deg: 48.85837,
        lon_deg: 2.29448,
        heading_deg: 45.0, // legs on the cardinal diagonals of the Champ axis
        base: Base::Terrain,
        bbox_min: [-66.0, 0.0, -66.0],
        bbox_max: [66.0, 331.0, 66.0],
        suppress: SUPPRESS_ALL,
        // The source OSM tower relation has a very thin outlying member just
        // beyond the authored feet; own the whole Champ-de-Mars tower pad so
        // it cannot appear as a detached vertical line at oblique angles.
        suppress_bbox_xz: Some(([-82.0, -82.0], [82.0, 82.0])),
    },
    Landmark {
        name: "statue-of-liberty",
        lat_deg: 40.68925,
        lon_deg: -74.04445,
        heading_deg: 145.0, // facing the harbor entrance
        base: Base::Fixed(3.0),
        bbox_min: [-48.0, 0.0, -48.0],
        bbox_max: [48.0, 94.0, 48.0],
        suppress: SUPPRESS_ALL,
        suppress_bbox_xz: None,
    },
    Landmark {
        name: "brooklyn-bridge",
        // OSM architectural outline centroid; source +X is baked to model +z.
        lat_deg: 40.7062175,
        lon_deg: -73.9970208,
        heading_deg: 316.2, // +z toward Manhattan along the crossing
        base: Base::Fixed(0.0),
        bbox_min: [-24.0, 0.0, -570.0],
        bbox_max: [24.0, 87.0, 570.0],
        suppress: Suppress {
            buildings: true,
            roads: true,
            trees: false,
        },
        suppress_bbox_xz: Some(([-24.0, -570.0], [24.0, 570.0])),
    },
    Landmark {
        name: "palace-of-westminster",
        // Similarity fit from the supplied model's Elizabeth/Victoria Tower
        // centres to their OSM building footprints, then manually calibrated
        // 4 m west and 6° clockwise against the rendered surroundings.
        lat_deg: 51.49948556998585,
        lon_deg: -0.12477409260956815,
        heading_deg: 6.03449999999998,
        base: Base::Terrain,
        // Activation includes the feathered DEM cleanup; source-feature
        // ownership below stays restricted to the architectural footprint.
        bbox_min: [-80.0, -5.0, -164.0],
        bbox_max: [80.0, 98.0, 164.0],
        suppress: SUPPRESS_ALL,
        // Building replacement is inferred from the terrain footprint above;
        // no second hand-tuned suppression rectangle is needed.
        suppress_bbox_xz: None,
    },
    Landmark {
        name: "london-eye",
        // Manually calibrated 20 m east; the wheel plane follows the South
        // Bank at an absolute +Z bearing of 99°.
        lat_deg: 51.503359999646364,
        lon_deg: -0.11922196252910618,
        heading_deg: 99.0,
        base: Base::Terrain,
        bbox_min: [-72.0, -8.0, -18.0],
        bbox_max: [72.0, 136.0, 15.0],
        suppress: SUPPRESS_ALL,
        // Kill the ordinary County Hall/boarding-pavilion extrusion that
        // otherwise cuts through the wheel and A-frame.
        suppress_bbox_xz: Some(([-80.0, -42.0], [80.0, 42.0])),
    },
    Landmark {
        name: "tower-bridge",
        lat_deg: 51.5055158,
        lon_deg: -0.0753665,
        heading_deg: 22.8, // surveyed long axis; +z toward the north tower
        base: Base::Fixed(0.0),
        bbox_min: [-30.0, 0.0, -158.0],
        bbox_max: [30.0, 68.0, 158.0],
        suppress: Suppress {
            buildings: true,
            roads: true,
            trees: false,
        },
        suppress_bbox_xz: Some(([-30.0, -158.0], [30.0, 158.0])),
    },
    Landmark {
        name: "golden-gate-bridge",
        lat_deg: 37.8202408,
        lon_deg: -122.4785700,
        heading_deg: 353.7, // surveyed OSM axis; +z toward Marin
        base: Base::Fixed(0.0),
        bbox_min: [-25.0, 0.0, -1370.0],
        bbox_max: [25.0, 231.0, 1370.0],
        suppress: Suppress {
            buildings: true,
            roads: true,
            trees: false,
        },
        suppress_bbox_xz: Some(([-25.0, -1370.0], [25.0, 1370.0])),
    },
    Landmark {
        name: "colosseum",
        lat_deg: 41.89094,
        lon_deg: 12.49190,
        heading_deg: 40.0, // major axis bearing
        base: Base::Terrain,
        bbox_min: [-96.0, -10.0, -80.0],
        bbox_max: [96.0, 50.0, 80.0],
        suppress: SUPPRESS_ALL,
        suppress_bbox_xz: None,
    },
    Landmark {
        name: "giza-necropolis",
        lat_deg: 29.97913,
        lon_deg: 31.13418,
        heading_deg: 0.0, // the pyramids are cardinally aligned
        base: Base::Terrain,
        bbox_min: [-900.0, 0.0, -860.0],
        // Includes the clean-ground feather around Khufu; the other two
        // correction regions already fit within the negative extents.
        bbox_max: [170.0, 142.0, 170.0],
        suppress: SUPPRESS_BLDG,
        suppress_bbox_xz: None,
    },
    Landmark {
        name: "space-needle",
        lat_deg: 47.62051,
        lon_deg: -122.34930,
        heading_deg: 0.0,
        base: Base::Terrain,
        bbox_min: [-23.0, 0.0, -24.0],
        bbox_max: [23.0, 185.0, 24.0],
        suppress: SUPPRESS_ALL,
        suppress_bbox_xz: None,
    },
    Landmark {
        name: "gateway-arch",
        lat_deg: 38.62451999972727,
        lon_deg: -90.1849626343462,
        heading_deg: 18.0,
        base: Base::Terrain,
        bbox_min: [-10.0, 0.0, -101.0],
        bbox_max: [10.0, 193.0, 101.0],
        suppress: SUPPRESS_BLDG,
        suppress_bbox_xz: None,
    },
    Landmark {
        name: "christ-the-redeemer",
        lat_deg: -22.95191,
        lon_deg: -43.21051,
        heading_deg: 100.0, // facing the bay
        base: Base::Terrain,
        bbox_min: [-16.0, 0.0, -15.0],
        bbox_max: [16.0, 39.0, 15.0],
        suppress: SUPPRESS_BLDG,
        suppress_bbox_xz: None,
    },
    Landmark {
        name: "burj-khalifa",
        lat_deg: 25.19720,
        lon_deg: 55.27441,
        heading_deg: 30.0,
        base: Base::Terrain,
        bbox_min: [-91.0, 0.0, -84.0],
        bbox_max: [91.0, 831.0, 84.0],
        suppress: SUPPRESS_ALL,
        suppress_bbox_xz: None,
    },
    Landmark {
        name: "empire-state-building",
        lat_deg: 40.74844,
        lon_deg: -73.98565,
        heading_deg: 29.0, // Manhattan grid
        base: Base::Terrain,
        bbox_min: [-63.0, 0.0, -33.0],
        bbox_max: [63.0, 444.0, 33.0],
        suppress: SUPPRESS_BLDG,
        suppress_bbox_xz: None,
    },
    Landmark {
        name: "arc-de-triomphe",
        lat_deg: 48.87379,
        lon_deg: 2.29496,
        heading_deg: 116.0, // +z toward the Louvre along the Champs-Élysées axis
        base: Base::Terrain,
        bbox_min: [-23.0, -3.0, -12.0],
        bbox_max: [23.0, 51.0, 12.0],
        suppress: SUPPRESS_ALL,
        suppress_bbox_xz: None,
    },
    Landmark {
        name: "brandenburg-gate",
        lat_deg: 52.51629,
        lon_deg: 13.37769,
        heading_deg: 84.0, // gate passage toward Pariser Platz / Unter den Linden
        base: Base::Terrain,
        bbox_min: [-35.0, -3.0, -13.0],
        bbox_max: [35.0, 33.0, 13.0],
        suppress: SUPPRESS_BLDG,
        suppress_bbox_xz: None,
    },
    Landmark {
        name: "tokyo-tower",
        lat_deg: 35.65858,
        lon_deg: 139.74543,
        heading_deg: 0.0,
        base: Base::Terrain,
        // The wider activation bbox includes the feathered DEM correction;
        // ordinary-feature ownership remains limited to the tower itself.
        bbox_min: [-66.0, -1.0, -66.0],
        bbox_max: [66.0, 334.0, 66.0],
        suppress: SUPPRESS_ALL,
        suppress_bbox_xz: Some(([-43.0, -43.0], [43.0, 43.0])),
    },
    Landmark {
        name: "washington-monument",
        lat_deg: 38.88946,
        lon_deg: -77.03525,
        heading_deg: 0.0,
        base: Base::Terrain,
        bbox_min: [-9.0, 0.0, -9.0],
        bbox_max: [9.0, 170.0, 9.0],
        suppress: SUPPRESS_BLDG,
        suppress_bbox_xz: None,
    },
    Landmark {
        name: "singapore-merlion",
        lat_deg: 1.2867841,
        lon_deg: 103.8545035,
        heading_deg: 90.0, // authored facing axis points into Marina Bay
        base: Base::Terrain,
        // Includes the small fountain-pad terrain correction and feather.
        bbox_min: [-10.0, -1.0, -11.0],
        bbox_max: [10.0, 11.0, 11.0],
        suppress: SUPPRESS_ALL,
        suppress_bbox_xz: None,
    },
    Landmark {
        name: "angkor-wat",
        lat_deg: 13.41247,
        lon_deg: 103.86699,
        heading_deg: 0.0,
        base: Base::Terrain,
        bbox_min: [-180.0, -1.0, -129.0],
        bbox_max: [129.0, 66.0, 129.0],
        suppress: SUPPRESS_ALL,
        suppress_bbox_xz: None,
    },
    Landmark {
        name: "buckingham-palace",
        lat_deg: 51.501054401969675,
        lon_deg: -0.14250924931357423,
        heading_deg: 326.0,
        base: Base::Terrain,
        bbox_min: [-86.0, 0.0, -75.0],
        bbox_max: [86.0, 42.0, 75.0],
        suppress: SUPPRESS_ALL,
        suppress_bbox_xz: None,
    },
    Landmark {
        name: "burj-al-arab",
        lat_deg: 25.141289300368246,
        lon_deg: 55.18532933805024,
        // The authored +z axis points landward from the sail toward Jumeirah.
        heading_deg: 126.0,
        base: Base::Fixed(0.0),
        bbox_min: [-57.0, 0.0, -69.0],
        bbox_max: [57.0, 322.0, 69.0],
        suppress: SUPPRESS_ALL,
        suppress_bbox_xz: None,
    },
    Landmark {
        name: "cn-tower",
        lat_deg: 43.64257,
        lon_deg: -79.38705,
        heading_deg: 0.0,
        base: Base::Terrain,
        bbox_min: [-49.0, 0.0, -45.0],
        bbox_max: [49.0, 555.0, 45.0],
        suppress: SUPPRESS_ALL,
        suppress_bbox_xz: None,
    },
    Landmark {
        name: "golden-temple",
        lat_deg: 31.6199729437563,
        lon_deg: 74.87653269769807,
        heading_deg: 33.0,
        base: Base::TerrainOffset(1.0),
        bbox_min: [-24.0, -2.0, -24.0],
        bbox_max: [24.0, 21.0, 24.0],
        suppress: SUPPRESS_ALL,
        suppress_bbox_xz: None,
    },
    Landmark {
        name: "himeji-castle",
        lat_deg: 34.83945,
        lon_deg: 134.69390,
        heading_deg: 0.0,
        base: Base::Terrain,
        bbox_min: [-220.0, -1.0, -125.0],
        bbox_max: [88.0, 67.0, 70.0],
        suppress: SUPPRESS_ALL,
        suppress_bbox_xz: None,
    },
    Landmark {
        name: "moscow-kremlin",
        lat_deg: 55.75167,
        lon_deg: 37.61778,
        // Three-corner OSM fit: the authored south wall was too long and its
        // plan was about 17 degrees off the real wall axes.  The asset bakes
        // the small anisotropic correction; heading supplies the rigid turn.
        heading_deg: 343.0687212864675,
        base: Base::Terrain,
        bbox_min: [-423.0, 0.0, -360.0],
        bbox_max: [314.0, 82.0, 448.0],
        suppress: SUPPRESS_ALL,
        suppress_bbox_xz: None,
    },
    Landmark {
        name: "leaning-tower-of-pisa",
        lat_deg: 43.72295,
        lon_deg: 10.39660,
        heading_deg: 0.0,
        base: Base::Terrain,
        bbox_min: [-23.0, -3.0, -24.0],
        bbox_max: [23.0, 58.0, 24.0],
        suppress: SUPPRESS_ALL,
        suppress_bbox_xz: None,
    },
    Landmark {
        name: "louvre-pyramid",
        lat_deg: 48.860556046566394,
        lon_deg: 2.3376945138546144,
        heading_deg: 20.0,
        base: Base::Terrain,
        bbox_min: [-34.0, 0.0, -34.0],
        bbox_max: [34.0, 23.0, 34.0],
        suppress: SUPPRESS_ALL,
        suppress_bbox_xz: None,
    },
    Landmark {
        name: "marina-bay-sands",
        lat_deg: 1.2835718293333551,
        lon_deg: 103.86078985392429,
        heading_deg: 13.0,
        base: Base::Terrain,
        bbox_min: [-60.0, 0.0, -197.0],
        bbox_max: [60.0, 196.0, 197.0],
        suppress: SUPPRESS_ALL,
        suppress_bbox_xz: None,
    },
    Landmark {
        name: "notre-dame",
        lat_deg: 48.85297,
        lon_deg: 2.34990,
        // Model +x follows the nave toward the apse; +z is its north side.
        heading_deg: 25.0,
        base: Base::Terrain,
        bbox_min: [-87.0, 0.0, -47.0],
        bbox_max: [87.0, 97.0, 47.0],
        suppress: SUPPRESS_ALL,
        suppress_bbox_xz: None,
    },
    Landmark {
        name: "nyhavn",
        // The asset is centred, while Nyhavn 1 is the west endpoint of the
        // authored row. Move 72.25 m along the surveyed No. 1 -> No. 17
        // frontage axis so model x=0 lands at the row midpoint. Model +x
        // follows that 113.66° ESE axis; therefore model +z (the runtime
        // heading) is 90° counter-clockwise, leaving facade -z toward the
        // canal on the south side.
        lat_deg: 55.68055871,
        lon_deg: 12.58900560,
        heading_deg: 24.0,
        base: Base::Terrain,
        bbox_min: [-94.0, -2.0, -27.0],
        bbox_max: [94.0, 22.0, 27.0],
        suppress: SUPPRESS_ALL,
        suppress_bbox_xz: None,
    },
    Landmark {
        name: "parthenon",
        lat_deg: 37.97153,
        lon_deg: 23.72673,
        // Model +x is the long east-west axis, approximately bearing 102°.
        heading_deg: 356.0,
        base: Base::Terrain,
        bbox_min: [-54.0, 0.0, -35.0],
        bbox_max: [54.0, 24.0, 35.0],
        suppress: SUPPRESS_ALL,
        suppress_bbox_xz: None,
    },
    Landmark {
        name: "st-basils-cathedral",
        lat_deg: 55.75252,
        lon_deg: 37.62310,
        // Rotate the idealized compass plan to the cathedral's WSW axis.
        heading_deg: 340.0,
        base: Base::Terrain,
        bbox_min: [-45.0, 0.0, -42.0],
        bbox_max: [45.0, 66.0, 42.0],
        suppress: SUPPRESS_ALL,
        suppress_bbox_xz: None,
    },
    Landmark {
        name: "taipei-101",
        lat_deg: 25.03397,
        lon_deg: 121.56447,
        heading_deg: 0.0,
        base: Base::Terrain,
        bbox_min: [-92.0, 0.0, -100.0],
        bbox_max: [108.0, 511.0, 94.0],
        suppress: SUPPRESS_ALL,
        suppress_bbox_xz: None,
    },
    Landmark {
        name: "white-house",
        lat_deg: 38.89768,
        lon_deg: -77.03653,
        heading_deg: 0.0,
        base: Base::Terrain,
        bbox_min: [-120.0, 0.0, -61.0],
        bbox_max: [120.0, 27.0, 61.0],
        suppress: SUPPRESS_ALL,
        suppress_bbox_xz: None,
    },
    Landmark {
        name: "chrysler-building",
        lat_deg: 40.7516028,
        lon_deg: -73.9753441,
        heading_deg: 29.1, // authored +Y follows Lexington Avenue north
        base: Base::Terrain,
        bbox_min: [-54.0, -1.0, -54.0],
        bbox_max: [54.0, 321.0, 54.0],
        suppress: SUPPRESS_ALL,
        suppress_bbox_xz: None,
    },
    Landmark {
        name: "flatiron-building",
        // Area centroid of the OSM architectural footprint. Authored +Y
        // follows the Broadway sides toward the north prow.
        lat_deg: 40.74100242,
        lon_deg: -73.98965289,
        heading_deg: 6.1,
        base: Base::Terrain,
        bbox_min: [-30.0, -1.0, -40.0],
        bbox_max: [36.0, 89.0, 62.0],
        suppress: SUPPRESS_ALL,
        suppress_bbox_xz: None,
    },
    Landmark {
        name: "merdeka-118",
        lat_deg: 3.14161165,
        lon_deg: 101.70068015,
        heading_deg: 31.2, // OSM footprint long axis, baked from authored +X
        base: Base::Terrain,
        bbox_min: [-64.0, -1.0, -72.0],
        bbox_max: [64.0, 683.0, 72.0],
        suppress: SUPPRESS_ALL,
        suppress_bbox_xz: None,
    },
    Landmark {
        name: "one-world-trade-center",
        lat_deg: 40.71299920,
        lon_deg: -74.01318944,
        heading_deg: 27.5, // one base face follows the mapped Manhattan grid
        base: Base::Terrain,
        bbox_min: [-56.0, -1.0, -56.0],
        bbox_max: [56.0, 544.0, 56.0],
        suppress: SUPPRESS_ALL,
        suppress_bbox_xz: None,
    },
    Landmark {
        name: "petronas-towers",
        // Centroid of the combined mapped twin-tower/skybridge outline.
        lat_deg: 3.15796513,
        lon_deg: 101.71175043,
        heading_deg: 49.9, // tower-to-tower authored +X axis
        base: Base::Terrain,
        bbox_min: [-54.0, -1.0, -122.0],
        bbox_max: [54.0, 454.0, 122.0],
        suppress: SUPPRESS_ALL,
        suppress_bbox_xz: None,
    },
    Landmark {
        name: "steinway-tower",
        lat_deg: 40.7648474,
        lon_deg: -73.9775286,
        heading_deg: 28.9, // authored +Y toward West 58th Street
        base: Base::Terrain,
        bbox_min: [-35.0, -1.0, -52.0],
        bbox_max: [35.0, 438.0, 48.0],
        suppress: SUPPRESS_ALL,
        suppress_bbox_xz: None,
    },
    Landmark {
        name: "the-gherkin",
        lat_deg: 51.51447516,
        lon_deg: -0.08033251,
        heading_deg: 0.0, // rotationally symmetric footprint
        base: Base::Terrain,
        bbox_min: [-50.0, -2.0, -50.0],
        bbox_max: [50.0, 183.0, 50.0],
        suppress: SUPPRESS_ALL,
        suppress_bbox_xz: None,
    },
    Landmark {
        name: "the-shard",
        lat_deg: 51.50440762,
        lon_deg: -0.08643823,
        heading_deg: 33.6, // maps authored north/south planes to the site
        base: Base::Terrain,
        bbox_min: [-54.0, -1.0, -49.0],
        bbox_max: [58.0, 312.0, 50.0],
        suppress: SUPPRESS_ALL,
        suppress_bbox_xz: None,
    },
];

const fn combined_catalog<const N: usize>() -> [Landmark; N] {
    let mut result = [EXISTING_LANDMARKS[0]; N];
    let mut i = 0;
    while i < EXISTING_LANDMARKS.len() {
        result[i] = EXISTING_LANDMARKS[i];
        i += 1;
    }
    let mut j = 0;
    while j < super::city_generated::CITY_MODELS.len() {
        result[i + j] = super::city_generated::CITY_MODELS[j];
        j += 1;
    }
    i += j;
    let mut k = 0;
    while k < super::heritage_generated::HERITAGE_MODELS.len() {
        result[i + k] = super::heritage_generated::HERITAGE_MODELS[k];
        k += 1;
    }
    i += k;
    let mut s = 0;
    while s < super::skyscraper_generated::SKYSCRAPER_MODELS.len() {
        result[i + s] = super::skyscraper_generated::SKYSCRAPER_MODELS[s];
        s += 1;
    }
    i += s;
    let mut m = 0;
    while m < super::monument_generated::MONUMENT_MODELS.len() {
        result[i + m] = super::monument_generated::MONUMENT_MODELS[m];
        m += 1;
    }
    i += m;
    let mut a = 0;
    while a < super::island_generated::ISLAND_MODELS.len() {
        result[i + a] = super::island_generated::ISLAND_MODELS[a];
        a += 1;
    }
    result
}

pub static LANDMARKS: &[Landmark] = &combined_catalog::<
    {
        EXISTING_LANDMARKS.len()
            + super::city_generated::CITY_MODELS.len()
            + super::heritage_generated::HERITAGE_MODELS.len()
            + super::skyscraper_generated::SKYSCRAPER_MODELS.len()
            + super::monument_generated::MONUMENT_MODELS.len()
            + super::island_generated::ISLAND_MODELS.len()
    },
>();
