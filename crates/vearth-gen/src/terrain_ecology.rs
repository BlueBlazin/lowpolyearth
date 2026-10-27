//! Continuous ground palettes from mapped cover and observed climate normals.
//!
//! The one-degree climate atlas supports broad appearance variation, not exact
//! ecoregions or species. These weights only condition existing cover; they do
//! not place vegetation or replace source land-use boundaries.

use crate::{Climate, lerp3, palette::terrain as p, smoothstep01};

#[derive(Clone, Copy)]
pub(crate) enum Cover {
    Inferred,
    Forest,
    Grass,
    Scrub,
}

pub(crate) fn palette(cover: Cover, climate: Climate, elevation_m: f32) -> [f32; 3] {
    let temperature = climate.temp_c;
    let tropical = ramp(17.0, 25.0, temperature);
    let dry = ramp(0.30, 0.68, climate.arid);
    let humid = ramp(1.8, 4.5, climate.precipitation_mm_day) * ramp(0.58, 0.82, climate.humidity);
    let cold = 1.0 - ramp(0.0, 8.0, temperature);
    let dry_grass = lerp3(p::STEPPE, p::SAVANNA, tropical);

    match cover {
        Cover::Inferred => {
            let boreal = lerp3(p::BOREAL_DARK, p::BOREAL_GREEN, 0.48);
            let temperate = lerp3(p::TEMPERATE_FOREST, p::TEMPERATE_GREEN, 0.48);
            let rainforest = lerp3(p::RAINFOREST, p::TROPICAL_GREEN, 0.40);
            let green = lerp3(boreal, temperate, ramp(5.0, 13.0, temperature));
            let green = lerp3(green, rainforest, tropical);
            let tundra = lerp3(p::TUNDRA_DRY, p::TUNDRA_LICHEN, 0.55);
            let green = lerp3(green, tundra, cold);
            lerp3(green, dry_grass, dry * (1.0 - cold * 0.65))
        }
        Cover::Forest => {
            let boreal = lerp3(p::BOREAL_DARK, p::BOREAL_GREEN, 0.35);
            let green = lerp3(boreal, p::TEMPERATE_FOREST, ramp(5.0, 14.0, temperature));
            let green = lerp3(green, p::RAINFOREST, tropical);
            // A cool, humid upland tint is an appearance cue. This does not
            // assert cloud-forest ecology from annual climate alone.
            let humid_upland = humid * ramp(650.0, 1_800.0, elevation_m) * (1.0 - dry);
            let green = lerp3(green, p::CLOUD_FOREST, humid_upland * 0.70);
            lerp3(green, p::DRY_WOODLAND, dry)
        }
        Cover::Grass => {
            let meadow = lerp3(p::PRAIRIE, p::TEMPERATE_GREEN, 0.30);
            let meadow = lerp3(meadow, p::HUMID_MEADOW, humid * 0.70);
            let meadow = lerp3(meadow, p::TROPICAL_GREEN, tropical * (1.0 - dry) * 0.55);
            let alpine = ramp(900.0, 2_200.0, elevation_m)
                * (1.0 - ramp(5.0, 13.0, temperature))
                * (1.0 - dry);
            let meadow = lerp3(meadow, p::ALPINE_MEADOW, alpine);
            let meadow = lerp3(meadow, p::TUNDRA_LICHEN, cold * (1.0 - alpine * 0.70));
            lerp3(meadow, dry_grass, dry)
        }
        Cover::Scrub => {
            let temperate = lerp3(p::CHAPARRAL, p::MEDITERRANEAN, 0.42);
            let scrub = lerp3(temperate, p::HEATH, cold);
            let scrub = lerp3(scrub, p::TROPICAL_DARK, humid * tropical * 0.45);
            let dry_scrub = lerp3(p::DRY_SCRUB, p::SAVANNA, tropical * 0.50);
            lerp3(scrub, dry_scrub, dry)
        }
    }
}

fn ramp(low: f32, high: f32, value: f32) -> f32 {
    smoothstep01((value - low) / (high - low))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn climate(temperature: f32, arid: f32) -> Climate {
        Climate {
            temp_c: temperature,
            arid,
            precipitation_mm_day: 3.5 * (1.0 - arid),
            humidity: 0.82 - arid * 0.55,
        }
    }

    #[test]
    fn mapped_cover_retains_distinct_ground_character_in_the_same_climate() {
        let climate = climate(13.0, 0.15);
        let forest = palette(Cover::Forest, climate, 300.0);
        let grass = palette(Cover::Grass, climate, 300.0);
        let scrub = palette(Cover::Scrub, climate, 300.0);
        assert!(forest[1] + 20.0 < grass[1]);
        assert!(forest[0] + 20.0 < scrub[0]);
        assert!(grass[1] - grass[0] > scrub[1] - scrub[0]);
    }

    #[test]
    fn dry_cold_uplands_do_not_receive_the_moist_alpine_meadow_anchor() {
        let dry = climate(3.0, 0.88);
        assert_eq!(
            palette(Cover::Grass, dry, 300.0),
            palette(Cover::Grass, dry, 4_500.0)
        );
        let moist = climate(5.0, 0.15);
        assert_ne!(
            palette(Cover::Grass, moist, 300.0),
            palette(Cover::Grass, moist, 2_200.0)
        );
    }

    #[test]
    fn cover_palettes_have_no_temperature_aridity_or_altitude_steps() {
        for cover in [Cover::Inferred, Cover::Forest, Cover::Grass, Cover::Scrub] {
            let mut previous = palette(cover, climate(-15.0, 0.20), 1_400.0);
            for step in 1..=500 {
                let current = palette(cover, climate(-15.0 + step as f32 * 0.1, 0.20), 1_400.0);
                assert!(
                    current
                        .iter()
                        .zip(previous)
                        .all(|(a, b)| (a - b).abs() < 2.0)
                );
                previous = current;
            }
            previous = palette(cover, climate(8.0, 0.0), 1_400.0);
            for step in 1..=1000 {
                let current = palette(cover, climate(8.0, step as f32 / 1000.0), 1_400.0);
                assert!(
                    current
                        .iter()
                        .zip(previous)
                        .all(|(a, b)| (a - b).abs() < 1.0)
                );
                previous = current;
            }
            previous = palette(cover, climate(5.0, 0.15), 0.0);
            for step in 1..=500 {
                let current = palette(cover, climate(5.0, 0.15), step as f32 * 10.0);
                assert!(
                    current
                        .iter()
                        .zip(previous)
                        .all(|(a, b)| (a - b).abs() < 1.0)
                );
                assert!(
                    current
                        .iter()
                        .all(|value| value.is_finite() && (0.0..=255.0).contains(value))
                );
                previous = current;
            }
        }
    }
}
