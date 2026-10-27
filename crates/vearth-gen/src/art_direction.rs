//! Material-aware colour treatment for the faceted world.
//!
//! Inputs and outputs are unlit, display-referred sRGB bytes represented as
//! floats. Geographic imagery supplies the local identity; authored material
//! anchors keep nearby surfaces in the same visual family as physical props.
//! Lighting, weather, and exposure belong to the renderer.

use crate::material;
use crate::palette::Rgb;

/// Resolve one source image sample against its regional material palette.
///
/// `detail` is a continuous presentation weight: zero preserves the source
/// exactly for globe views and one applies the nearby-world treatment. Apply
/// this once when selecting a source sample, before semantic overlays and
/// procedural variation, so overlaid surfaces do not receive repeated grades.
pub fn imagery_surface(material: u8, observed: Rgb, regional: Rgb, detail: f32) -> Rgb {
    let detail = detail.clamp(0.0, 1.0);
    if detail == 0.0 {
        return observed;
    }
    let (source_value_weight, value_radius, source_chroma_weight, chroma_radius) = match material {
        material::GRASS | material::WETLAND => (0.58, 95.0, 0.28, 45.0),
        material::SOIL => (0.62, 110.0, 0.50, 65.0),
        material::ROCK | material::SAND | material::VOLCANIC | material::SEABED => {
            (0.72, 135.0, 0.60, 75.0)
        }
        material::SNOW | material::SALT | material::ICE => (0.35, 60.0, 0.30, 35.0),
        material::ASPHALT | material::PAVED | material::RAIL | material::PATH => {
            (0.44, 75.0, 0.25, 35.0)
        }
        mat if material::is_architecture_wall(mat) => (0.52, 90.0, 0.38, 55.0),
        mat if material::is_architecture_roof(mat) => (0.42, 75.0, 0.24, 45.0),
        mat if material::is_field_surface(mat) => (0.55, 90.0, 0.34, 45.0),
        _ => return observed,
    };
    let source_value = luminance(observed);
    let anchor_value = luminance(regional);
    // An aerial photograph already contains lighting. Bound its contribution
    // around the local material value so dark photographic shadows do not
    // become black albedo that receives a second shadow in the renderer.
    let value =
        anchor_value + soft_bound(source_value - anchor_value, value_radius) * source_value_weight;
    let styled: Rgb = std::array::from_fn(|channel| {
        let local_chroma = observed[channel] - source_value;
        let family_chroma = regional[channel] - anchor_value;
        value
            + family_chroma
            + soft_bound(local_chroma - family_chroma, chroma_radius) * source_chroma_weight
    });
    std::array::from_fn(|channel| {
        (observed[channel] + (styled[channel] - observed[channel]) * detail).clamp(0.0, 255.0)
    })
}

/// Sample two broad, continuous colour fields on the planet's surface.
///
/// Inputs are geodetic degrees. The sphere-space lattice avoids both longitude
/// seams and latitude-dependent stripe widths. Its 240 m and 960 m scales are
/// independent of chunk identity, voxel pitch, and camera distance. These are
/// presentation variations, not replacements for observed land cover.
pub fn landscape_variation(latitude_deg: f64, longitude_deg: f64) -> [f32; 2] {
    let (lat_sin, lat_cos) = latitude_deg.to_radians().sin_cos();
    let (lon_sin, lon_cos) = longitude_deg.to_radians().sin_cos();
    let position =
        [lat_cos * lon_cos, lat_sin, lat_cos * lon_sin].map(|component| component * 6_371_008.8);
    [
        value_noise(position.map(|v| v / 240.0), 0x28a5_921d),
        value_noise(position.map(|v| v / 960.0), 0x7d13_b9e5),
    ]
}

/// Apply broad analogous hue variation once to an unlit natural surface.
///
/// Use the same `detail` weight as [`imagery_surface`]. A zero weight preserves
/// globe imagery exactly. Semantic roads, buildings, crop rows and water retain
/// their authored object/material colours; this cannot change their identity.
pub fn landscape_surface(material: u8, color: Rgb, variation: [f32; 2], detail: f32) -> Rgb {
    let (hue_delta, value_amount) = match material {
        material::GRASS | material::WETLAND => ([15.0, 0.5, -15.0], 9.0),
        material::SOIL => ([8.0, -1.0, -6.0], 6.0),
        material::ROCK | material::SAND | material::SEABED => ([8.0, -0.5, -7.0], 7.0),
        material::VOLCANIC => ([3.0, -0.5, -2.0], 4.0),
        _ => return color,
    };
    let detail = detail.clamp(0.0, 1.0);
    let hue = variation[0].clamp(-1.0, 1.0);
    let value = variation[1].clamp(-1.0, 1.0) * value_amount;
    let hue_value = luminance(hue_delta);
    std::array::from_fn(|channel| {
        (color[channel] + ((hue_delta[channel] - hue_value) * hue + value) * detail)
            .clamp(0.0, 255.0)
    })
}

/// Keep opaque architectural colours inside a usable diffuse value range.
///
/// This is for authored wall/roof albedo before shading or source-image
/// blending. Glass, metals, lamps, and facade emission retain their own colour
/// contracts. Chromatic facades keep their hue. Roof planes use a narrower
/// chroma range than small painted accents, while white roof finishes and
/// Mediterranean plaster retain their deliberately high material values.
pub fn architecture_surface(material: u8, authored: [u8; 3]) -> [u8; 3] {
    if !material::is_architecture_wall(material) && !material::is_architecture_roof(material) {
        return authored;
    }
    let rgb = authored.map(f32::from);
    let value = luminance(rgb);
    let low = rgb.into_iter().fold(f32::INFINITY, f32::min);
    let high = rgb.into_iter().fold(f32::NEG_INFINITY, f32::max);
    let neutral = (1.0 - (high - low) / 48.0).clamp(0.0, 1.0);
    let target_value = if material::is_architecture_roof(material) {
        value.clamp(42.0, 225.0)
    } else {
        value.clamp(38.0, 232.0)
    };
    let chroma_scale = if material::is_architecture_roof(material) {
        0.74
    } else {
        0.96
    };
    let warm = [2.0, 0.5, -2.0];
    std::array::from_fn(|channel| {
        (target_value + (rgb[channel] - value) * chroma_scale + warm[channel] * neutral)
            .round()
            .clamp(0.0, 245.0) as u8
    })
}

fn soft_bound(value: f32, radius: f32) -> f32 {
    value / (1.0 + value.abs() / radius)
}

fn value_noise(position: [f64; 3], salt: u64) -> f32 {
    let cell = position.map(|value| value.floor() as i64);
    let weight: [f64; 3] = std::array::from_fn(|axis| {
        let t = position[axis] - cell[axis] as f64;
        t * t * (3.0 - 2.0 * t)
    });
    let mut value = 0.0;
    for z in 0..2 {
        for y in 0..2 {
            for x in 0..2 {
                let corner = [x, y, z];
                let mut seed = salt;
                let mut blend = 1.0;
                for axis in 0..3 {
                    seed ^= (cell[axis] + corner[axis]) as u64;
                    seed = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15);
                    seed ^= seed >> 29;
                    blend *= if corner[axis] == 0 {
                        1.0 - weight[axis]
                    } else {
                        weight[axis]
                    };
                }
                seed = (seed ^ (seed >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
                seed = (seed ^ (seed >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
                seed ^= seed >> 31;
                value += ((seed >> 40) as f64 / 8_388_607.5 - 1.0) * blend;
            }
        }
    }
    value as f32
}

fn luminance(rgb: Rgb) -> f32 {
    rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::architecture::ARCHITECTURE_PROFILES;
    use crate::palette::terrain;

    #[test]
    fn orbital_imagery_and_non_surface_materials_are_unchanged() {
        let source = [15.0, 180.0, 41.0];
        assert_eq!(
            imagery_surface(material::GRASS, source, terrain::TEMPERATE_GREEN, 0.0),
            source
        );
        for mat in [
            material::WATER,
            material::LIT_WINDOW,
            material::FOLIAGE_BASE,
        ] {
            assert_eq!(
                imagery_surface(mat, source, terrain::TEMPERATE_GREEN, 1.0),
                source
            );
        }
    }

    #[test]
    fn nearby_vegetation_reduces_source_contrast_without_erasing_it() {
        let anchor = terrain::TEMPERATE_GREEN;
        let sunlit = [90.0, 145.0, 46.0];
        let shadowed = [30.0, 56.0, 21.0];
        let a = imagery_surface(material::GRASS, sunlit, anchor, 1.0);
        let b = imagery_surface(material::GRASS, shadowed, anchor, 1.0);
        let source_contrast = luminance(sunlit) - luminance(shadowed);
        let result_contrast = luminance(a) - luminance(b);
        assert!(result_contrast > source_contrast * 0.15);
        assert!(result_contrast < source_contrast * 0.55);
        assert!(a[1] > a[0] && a[0] > a[2]);
        assert!(b[1] > b[0] && b[0] > b[2]);
        assert!(luminance(b) > luminance(shadowed));
    }

    #[test]
    fn saturated_source_and_photographic_shadows_do_not_define_near_albedo() {
        for source in [[0.0, 255.0, 0.0], [4.0, 12.0, 4.0], [240.0, 240.0, 80.0]] {
            let result = imagery_surface(material::GRASS, source, terrain::TEMPERATE_GREEN, 1.0);
            assert!(result[1] > result[0] && result[0] > result[2]);
            assert!((luminance(result) - luminance(terrain::TEMPERATE_GREEN)).abs() < 40.0);
            assert!(result[1] - result[2] < 90.0);
        }
        let red_roof = imagery_surface(
            material::ROOF,
            [245.0, 42.0, 12.0],
            architecture_surface(material::ROOF, [151, 95, 73]).map(f32::from),
            1.0,
        );
        assert!(red_roof[0] > red_roof[1] + 20.0);
        assert!(red_roof[0] - red_roof[2] < 90.0);
    }

    #[test]
    fn geographic_variation_is_bounded_continuous_and_has_no_longitude_seam() {
        let mut low = [1.0_f32; 2];
        let mut high = [-1.0_f32; 2];
        for step in 0..200 {
            let latitude = 37.735 + step as f64 * 0.0001;
            let value = landscape_variation(latitude, -119.59);
            let neighbor = landscape_variation(latitude + 0.000001, -119.59);
            assert_eq!(value, landscape_variation(latitude, -119.59));
            for channel in 0..2 {
                assert!((-1.0..=1.0).contains(&value[channel]));
                assert!((value[channel] - neighbor[channel]).abs() < 0.002);
                low[channel] = low[channel].min(value[channel]);
                high[channel] = high[channel].max(value[channel]);
            }
        }
        assert!(high[0] - low[0] > 0.5);
        assert!(high[1] - low[1] > 0.2);
        for latitude in [-90.0, -45.0, 0.0, 45.0, 90.0] {
            let east = landscape_variation(latitude, 180.0);
            let west = landscape_variation(latitude, -180.0);
            for channel in 0..2 {
                assert!((east[channel] - west[channel]).abs() < 0.00001);
            }
        }
    }

    #[test]
    fn landscape_variation_changes_hue_without_creating_grid_contrast() {
        let anchor = terrain::TEMPERATE_GREEN;
        let warm = landscape_surface(material::GRASS, anchor, [1.0, 0.0], 1.0);
        let cool = landscape_surface(material::GRASS, anchor, [-1.0, 0.0], 1.0);
        assert!(warm[0] > cool[0] + 20.0 && cool[2] > warm[2] + 20.0);
        assert!((luminance(warm) - luminance(cool)).abs() < 0.001);
        for mat in [
            material::ROOF,
            material::ASPHALT,
            material::WATER,
            material::SNOW,
        ] {
            assert_eq!(landscape_surface(mat, anchor, [1.0; 2], 1.0), anchor);
        }
        assert_eq!(
            landscape_surface(material::GRASS, anchor, [1.0; 2], 0.0),
            anchor
        );
        for value in [[0.0; 3], [255.0; 3], anchor] {
            let output = landscape_surface(material::GRASS, value, [1.0, -1.0], 1.0);
            assert!(output.into_iter().all(|c| (0.0..=255.0).contains(&c)));
        }
    }

    #[test]
    fn regional_materials_retain_distinctive_hue_and_value() {
        let red_rock = imagery_surface(
            material::ROCK,
            [157.0, 82.0, 51.0],
            terrain::RED_SANDSTONE,
            1.0,
        );
        let pale_rock = imagery_surface(
            material::ROCK,
            [192.0, 189.0, 173.0],
            terrain::LIMESTONE,
            1.0,
        );
        let basalt = imagery_surface(material::VOLCANIC, [48.0, 50.0, 49.0], terrain::BASALT, 1.0);
        assert!(red_rock[0] - red_rock[1] > 50.0);
        assert!(pale_rock[0] - pale_rock[2] < 35.0);
        assert!(luminance(pale_rock) > luminance(red_rock) + 50.0);
        assert!(luminance(red_rock) > luminance(basalt) + 35.0);
    }

    #[test]
    fn imagery_treatment_is_continuous_and_bounded() {
        for source in [
            [0.0; 3],
            [255.0; 3],
            [0.0, 255.0, 0.0],
            [250.0, 20.0, 180.0],
        ] {
            for mat in [material::GRASS, material::SAND, material::ROOF] {
                let close = imagery_surface(mat, source, terrain::TEMPERATE_GREEN, 1.0);
                let mut previous = source;
                for step in 1..=100 {
                    let result =
                        imagery_surface(mat, source, terrain::TEMPERATE_GREEN, step as f32 / 100.0);
                    for channel in 0..3 {
                        assert!((0.0..=255.0).contains(&result[channel]));
                        assert!((result[channel] - previous[channel]).abs() < 2.0);
                        assert!(result[channel] >= source[channel].min(close[channel]));
                        assert!(result[channel] <= source[channel].max(close[channel]));
                    }
                    previous = result;
                }
            }
        }
    }

    #[test]
    fn architectural_regions_keep_their_authored_identity() {
        for profile in ARCHITECTURE_PROFILES {
            for (mat, colors) in [
                (material::WALL, profile.walls.as_slice()),
                (material::ROOF, profile.roofs.as_slice()),
            ] {
                for original in colors {
                    let result = architecture_surface(mat, *original);
                    let original_chroma =
                        original.iter().max().unwrap() - original.iter().min().unwrap();
                    for i in 0..3 {
                        for j in i + 1..3 {
                            if original_chroma >= 48 {
                                assert_eq!(
                                    original[i].cmp(&original[j]),
                                    result[i].cmp(&result[j])
                                );
                            }
                        }
                    }
                    assert!((25.0..=236.0).contains(&luminance(result.map(f32::from))));
                }
            }
        }
        let red = architecture_surface(material::WALL, [139, 55, 45]);
        assert!(red[0] > red[1] * 2);
        assert_eq!(
            architecture_surface(material::LIT_WINDOW, [255, 190, 64]),
            [255, 190, 64]
        );
    }
}
