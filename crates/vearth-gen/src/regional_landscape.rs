//! Continuous regional priors layered on the existing evidence-based ecology.
use crate::{Climate, lerp3, material, palette::Rgb, smoothstep01, terrain_ecology::Cover};
use vearth_data::climate::ClimateAtlas;

#[path = "landscape_catalog.rs"]
mod catalog;
use catalog::{PROFILES, RULES};

#[derive(Clone, Copy)]
enum Season {
    Evergreen,
    Temperate,
    Cool,
    Short,
    WinterRain,
    Rain,
    Savanna,
    Dry,
}
#[derive(Clone, Copy)]
struct Profile {
    #[allow(dead_code)]
    name: &'static str,
    vegetation: [[u8; 3]; 3],
    dormant: [u8; 3],
    substrate: [i8; 3],
    season: Season,
    strength: f32,
}
#[derive(Clone, Copy)]
enum Gate {
    Moist,
    Cool,
    Mild,
    Upland,
    Alpine,
    Cold,
    Dry,
    HotDry,
    TropicalMoist,
    HumidUpland,
    Warm,
}
#[derive(Clone, Copy)]
struct Rule {
    #[allow(dead_code)]
    name: &'static str,
    profile: catalog::Id,
    centre: [f64; 2],
    radius: [f64; 2],
    gate: Gate,
}

// The index is built by the compiler. Its cells only shortlist exact smooth
// footprint evaluations: crossing a bin edge never changes the palette.
const NX: usize = 72;
const NY: usize = 36;
const fn abs(x: f64) -> f64 {
    if x < 0.0 { -x } else { x }
}
const fn wrapped_distance(a: f64, b: f64) -> f64 {
    let d = abs(a - b);
    if d > 180.0 { 360.0 - d } else { d }
}
const fn intersects(rule: &Rule, bin: usize) -> bool {
    let lat = (bin / NX) as f64 * 5.0 - 87.5;
    let lon = (bin % NX) as f64 * 5.0 - 177.5;
    abs(lat - rule.centre[0]) <= rule.radius[0] + 2.5
        && wrapped_distance(lon, rule.centre[1]) <= rule.radius[1] + 2.5
}
const fn capacity() -> usize {
    let mut max = 0;
    let mut bin = 0;
    while bin < NX * NY {
        let mut count = 0;
        let mut i = 0;
        while i < RULES.len() {
            if intersects(&RULES[i], bin) {
                count += 1;
            }
            i += 1;
        }
        if count > max {
            max = count;
        }
        bin += 1;
    }
    max
}
const CAPACITY: usize = capacity();
const fn index() -> [[u8; CAPACITY]; NX * NY] {
    assert!(RULES.len() < 255);
    let mut bins = [[255; CAPACITY]; NX * NY];
    let mut bin = 0;
    while bin < NX * NY {
        let mut n = 0;
        let mut i = 0;
        while i < RULES.len() {
            if intersects(&RULES[i], bin) {
                bins[bin][n] = i as u8;
                n += 1;
            }
            i += 1;
        }
        bin += 1;
    }
    bins
}
static INDEX: [[u8; CAPACITY]; NX * NY] = index();

fn ramp(a: f32, b: f32, x: f32) -> f32 {
    smoothstep01((x - a) / (b - a))
}
impl Gate {
    fn weight(self, c: Climate, elevation: f32) -> f32 {
        let moist = ramp(0.7, 2.4, c.precipitation_mm_day);
        let cool = 1.0 - ramp(8.0, 15.0, c.temp_c);
        let warm = ramp(8.0, 18.0, c.temp_c);
        let upland = ramp(500.0, 1700.0, elevation);
        match self {
            Self::Moist => moist,
            Self::Cool => cool,
            Self::Mild => ramp(-4.0, 8.0, c.temp_c) * (1.0 - ramp(24.0, 32.0, c.temp_c)),
            Self::Upland => upland,
            Self::Alpine => ramp(900.0, 2300.0, elevation) * (1.0 - ramp(8.0, 17.0, c.temp_c)),
            Self::Cold => 1.0 - ramp(-1.0, 7.0, c.temp_c),
            Self::Dry => ramp(0.25, 0.65, c.arid),
            Self::HotDry => warm * ramp(0.50, 0.80, c.arid),
            Self::TropicalMoist => warm * moist,
            Self::HumidUpland => moist * upland,
            Self::Warm => warm,
        }
    }
}
impl Rule {
    fn weight(self, lat: f64, lon: f64, c: Climate, elevation: f32) -> f32 {
        let y = (lat - self.centre[0]) / self.radius[0];
        let x = wrapped_distance(lon, self.centre[1]) / self.radius[1];
        let radius2 = x * x + y * y;
        if radius2 >= 1.0 {
            return 0.0;
        }
        // Central half-radius is full strength; the outer half is a C1 fade.
        (1.0 - ramp(0.25, 1.0, radius2 as f32)) * self.gate.weight(c, elevation)
    }
}
impl Season {
    fn dormancy(self, temperature: f32, rain: f32) -> f32 {
        match self {
            Self::Evergreen => (1.0 - ramp(0.4, 2.5, rain)) * 0.16,
            Self::Temperate => (1.0 - ramp(2.0, 15.0, temperature)) * 0.45,
            Self::Cool => (1.0 - ramp(0.0, 11.0, temperature)) * 0.35,
            Self::Short => (1.0 - ramp(1.0, 9.0, temperature)) * 0.48,
            Self::WinterRain => (1.0 - ramp(0.3, 2.2, rain)) * ramp(6.0, 18.0, temperature) * 0.55,
            Self::Rain => (1.0 - ramp(0.5, 3.5, rain)) * 0.55,
            Self::Savanna => (1.0 - ramp(0.3, 2.8, rain)) * 0.60,
            Self::Dry => (1.0 - ramp(0.15, 1.4, rain)) * 0.25,
        }
    }
}

/// Premultiplied anchors resolved once per geographic sample, reused by all
/// material endpoints. This is CPU scratch state, never a mesh/cache payload.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Palette {
    vegetation: [Rgb; 3],
    strength: f32,
    substrate: Rgb,
}
impl Palette {
    pub(crate) const EMPTY: Self = Self {
        vegetation: [[0.0; 3]; 3],
        strength: 0.0,
        substrate: [0.0; 3],
    };

    pub(crate) fn scaled(mut self, detail: f32) -> Self {
        self.strength *= detail;
        for color in &mut self.vegetation {
            for channel in color {
                *channel *= detail;
            }
        }
        for channel in &mut self.substrate {
            *channel *= detail;
        }
        self
    }

    pub(crate) fn vegetation(self, cover: Cover, fallback: Rgb, observed: bool) -> Rgb {
        let anchor = match cover {
            Cover::Forest => self.vegetation[0],
            Cover::Grass => self.vegetation[1],
            Cover::Scrub => self.vegetation[2],
            Cover::Inferred => lerp3(self.vegetation[0], self.vegetation[1], 0.55),
        };
        // Ortho carries stronger local evidence than the regional prior.
        let weight = if observed { 0.55 } else { 1.0 };
        std::array::from_fn(|i| fallback[i] * (1.0 - self.strength * weight) + anchor[i] * weight)
    }

    pub(crate) fn substrate(self, mat: u8, fallback: Rgb) -> Rgb {
        let strength = match mat {
            material::SOIL => 1.0,
            material::ROCK | material::SAND => 0.65,
            material::VOLCANIC => 0.30,
            _ => return fallback,
        };
        std::array::from_fn(|i| (fallback[i] + self.substrate[i] * strength).clamp(0.0, 255.0))
    }
}

pub(crate) fn resolve(
    lat_rad: f64,
    lon_rad: f64,
    elevation: f32,
    c: Climate,
    month: Option<u8>,
) -> Palette {
    if !lat_rad.is_finite() || !lon_rad.is_finite() || !elevation.is_finite() || elevation < 0.0 {
        return Palette::EMPTY;
    }
    let lat = lat_rad.to_degrees().clamp(-90.0, 90.0);
    let lon = (lon_rad.to_degrees() + 180.0).rem_euclid(360.0) - 180.0;
    let x = ((lon + 180.0) / 5.0).floor() as usize;
    let y = (((lat + 90.0) / 5.0).floor() as usize).min(NY - 1);
    let candidates = &INDEX[y * NX + x.min(NX - 1)];
    let mut weights = [0.0_f32; CAPACITY];
    let mut total = 0.0;
    for (slot, &id) in candidates.iter().enumerate() {
        if id == 255 {
            break;
        }
        weights[slot] = RULES[id as usize].weight(lat, lon, c, elevation);
        total += weights[slot];
    }
    if total == 0.0 {
        return Palette::EMPTY;
    }
    // No winner-takes-all precedence: compatible overlapping rules share a
    // capped budget. Uncovered weight stays with the existing climate palette.
    let normalization = 1.0 / total.max(1.0);
    let monthly = month
        .filter(|m| (1..=12).contains(m))
        .map(|m| ClimateAtlas::bundled().monthly_surface(lat_rad, lon_rad, m));
    let mut result = Palette::EMPTY;
    for (slot, &id) in candidates.iter().enumerate() {
        if id == 255 {
            break;
        }
        if weights[slot] == 0.0 {
            continue;
        }
        let profile = PROFILES[RULES[id as usize].profile as usize];
        let weight = weights[slot] * normalization * profile.strength;
        let dormancy = monthly.map_or(0.0, |m| {
            profile.season.dormancy(
                m.mean_temperature_c - 2.4 * elevation.max(0.0) / 1000.0,
                m.precipitation_mm_day,
            )
        });
        result.strength += weight;
        for cover in 0..3 {
            // Forest floor changes weakly; no canopy autumn pigment is baked
            // into soil. Monthly imagery remains the measured seasonal signal.
            let seasonal_weight = dormancy * if cover == 0 { 0.18 } else { 0.50 };
            let anchor = lerp3(
                profile.vegetation[cover].map(f32::from),
                profile.dormant.map(f32::from),
                seasonal_weight,
            );
            for (channel, value) in anchor.iter().enumerate() {
                result.vegetation[cover][channel] += value * weight;
            }
        }
        for channel in 0..3 {
            result.substrate[channel] += f32::from(profile.substrate[channel]) * weight;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::climate_at;

    fn sample(lat: f64, lon: f64, elevation: f32, month: u8) -> Palette {
        let (lat, lon) = (lat.to_radians(), lon.to_radians());
        resolve(
            lat,
            lon,
            elevation,
            climate_at(lat, lon, elevation),
            Some(month),
        )
    }

    #[test]
    fn catalog_is_distinct_bounded_and_every_profile_is_used() {
        assert_eq!(PROFILES.len(), 35);
        let mut used = vec![false; PROFILES.len()];
        for (i, p) in PROFILES.iter().enumerate() {
            assert!((0.0..=0.60).contains(&p.strength));
            for color in p.vegetation.into_iter().chain([p.dormant]) {
                assert!(color.into_iter().all(|v| (40..=195).contains(&v)));
                assert!(color.iter().max().unwrap() - color.iter().min().unwrap() < 100);
            }
            assert!(p.substrate.into_iter().all(|v| v.abs() <= 8));
            assert!(
                PROFILES[..i]
                    .iter()
                    .all(|other| other.name != p.name && other.vegetation != p.vegetation)
            );
        }
        for (i, rule) in RULES.iter().enumerate() {
            assert!(RULES[..i].iter().all(|other| other.name != rule.name));
            assert!(rule.radius[0] > 0.0 && rule.radius[1] > 0.0);
            assert!(rule.centre[0].abs() + rule.radius[0] < 90.0);
            used[rule.profile as usize] = true;
        }
        assert!(used.into_iter().all(|used| used));
        // A catalog extension must retain sparse lookup; do not silently grow
        // per-sample work into a full scan by adding giant overlapping domes.
        assert!(CAPACITY <= 16, "{CAPACITY}");
    }

    #[test]
    fn index_contains_every_rule_whose_support_touches_each_bin() {
        for (bin, candidates) in INDEX.iter().enumerate() {
            let lat0 = (bin / NX) as f64 * 5.0 - 90.0;
            let lon0 = (bin % NX) as f64 * 5.0 - 180.0;
            for dy in [0.0, 0.001, 2.5, 4.999, 5.0] {
                for dx in [0.0, 0.001, 2.5, 4.999, 5.0] {
                    for (id, rule) in RULES.iter().enumerate() {
                        let y = (lat0 + dy - rule.centre[0]) / rule.radius[0];
                        let x = wrapped_distance(lon0 + dx, rule.centre[1]) / rule.radius[1];
                        if x * x + y * y < 1.0 {
                            assert!(candidates.contains(&(id as u8)));
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn representative_geographies_select_their_intended_profiles() {
        use catalog::Id;
        for (lat, lon, elev, expected) in [
            (53.0, -9.0, 100.0, Id::Atlantic),
            (37.0, -4.0, 200.0, Id::Mediterranean),
            (46.5, 10.0, 2600.0, Id::Alpine),
            (64.9, -18.8, 300.0, Id::VolcanicHeath),
            (63.0, 21.0, 200.0, Id::Boreal),
            (69.0, 177.0, 200.0, Id::Tundra),
            (37.5, -110.5, 1200.0, Id::Sandstone),
            (-3.0, -59.0, 100.0, Id::HumidTropical),
            (-15.0, -48.0, 700.0, Id::Cerrado),
            (14.0, -6.0, 200.0, Id::Sahel),
            (17.0, 77.0, 500.0, Id::Deccan),
            (-24.0, 130.0, 500.0, Id::Spinifex),
        ] {
            let c = climate_at(f64::to_radians(lat), f64::to_radians(lon), elev);
            let best = RULES
                .iter()
                .max_by(|a, b| {
                    a.weight(lat, lon, c, elev)
                        .total_cmp(&b.weight(lat, lon, c, elev))
                })
                .unwrap();
            assert_eq!(best.profile, expected, "{lat}/{lon}: {}", best.name);
            assert!(sample(lat, lon, elev, 7).strength > 0.05);
        }
    }

    #[test]
    fn overlap_shares_a_capped_budget_and_fades_to_fallback() {
        let c = Climate {
            temp_c: 10.0,
            arid: 0.3,
            precipitation_mm_day: 3.0,
            humidity: 0.7,
        };
        let mut strongest = 0.0_f32;
        for lat in 35..60 {
            for lon in -10..35 {
                let p = resolve(
                    (lat as f64).to_radians(),
                    (lon as f64).to_radians(),
                    1700.0,
                    c,
                    Some(4),
                );
                strongest = strongest.max(p.strength);
                assert!((0.0..=0.600001).contains(&p.strength));
            }
        }
        assert!(strongest > 0.5);
        let fallback = sample(-18.0, -140.0, 100.0, 7);
        assert_eq!(fallback, Palette::EMPTY);
        let original = [100.0, 120.0, 90.0];
        assert_eq!(fallback.vegetation(Cover::Grass, original, false), original);
        assert_eq!(sample(0.0, 0.0, -1.0, 7), Palette::EMPTY);
    }

    #[test]
    fn geographic_boundaries_dateline_and_poles_are_continuous() {
        for lat in (-85..=85).step_by(5) {
            for lon in (-180..=180).step_by(5) {
                let a = sample(lat as f64 - 1e-7, lon as f64 - 1e-7, 1400.0, 4);
                let b = sample(lat as f64 + 1e-7, lon as f64 + 1e-7, 1400.0, 4);
                for cover in 0..3 {
                    for channel in 0..3 {
                        assert!(
                            (a.vegetation[cover][channel] - b.vegetation[cover][channel]).abs()
                                < 0.002
                        );
                    }
                }
            }
        }
        for lat in [-90.0, -45.0, 0.0, 69.0, 90.0] {
            assert_eq!(sample(lat, 180.0, 300.0, 1), sample(lat, -180.0, 300.0, 1));
            assert_eq!(sample(lat, 180.0, 300.0, 1), sample(lat, 540.0, 300.0, 1));
        }
    }

    #[test]
    fn seasons_follow_local_climate_and_leave_substrate_stable() {
        let grass = |p: Palette| p.vegetation(Cover::Grass, [130.0, 140.0, 90.0], false);
        // Rain-driven Indian dry/wet phases are local observations, not an
        // inverted Northern/Southern Hemisphere four-season calendar.
        let dry = grass(sample(18.0, 76.0, 500.0, 4));
        let wet = grass(sample(18.0, 76.0, 500.0, 7));
        assert!(dry[0] - dry[1] > wet[0] - wet[1] + 1.0, "{dry:?} {wet:?}");
        for (lat, lon, elev) in [(46.5, 10.0, 2200.0), (-43.5, 169.8, 2200.0)] {
            let winter = grass(sample(lat, lon, elev, if lat > 0.0 { 1 } else { 7 }));
            let summer = grass(sample(lat, lon, elev, if lat > 0.0 { 7 } else { 1 }));
            assert!(winter[0] - winter[1] >= summer[0] - summer[1]);
        }
        for (lat, lon) in [(-3.0, -59.0), (18.0, 76.0), (64.9, -18.8), (37.0, -4.0)] {
            let baseline = sample(lat, lon, 300.0, 1);
            for month in 1..=12 {
                let p = sample(lat, lon, 300.0, month);
                for mat in [
                    material::ROCK,
                    material::SOIL,
                    material::SAND,
                    material::VOLCANIC,
                ] {
                    assert_eq!(
                        p.substrate(mat, [120.0; 3]),
                        baseline.substrate(mat, [120.0; 3])
                    );
                }
                for mat in [
                    material::SNOW,
                    material::ICE,
                    material::WATER,
                    material::ASPHALT,
                    material::PAVED,
                    material::FIELD_CROP,
                    material::WALL,
                ] {
                    assert_eq!(p.substrate(mat, [120.0; 3]), [120.0; 3]);
                }
            }
        }
        let jan = grass(sample(-3.0, -59.0, 100.0, 1));
        let jul = grass(sample(-3.0, -59.0, 100.0, 7));
        assert!(jan.iter().zip(jul).all(|(a, b)| (a - b).abs() < 2.0));
    }
}
