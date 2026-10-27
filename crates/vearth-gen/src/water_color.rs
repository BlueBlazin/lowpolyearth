use crate::{lerp3, palette, smoothstep01};

pub(crate) fn variation(lat: f64, lon: f64, sample_pitch_m: f32) -> [f32; 2] {
    // Suppress unresolved fields before mesh sampling, including orbital LODs.
    let detail = 1.0 - smoothstep01((sample_pitch_m - 48.0) / 464.0);
    if detail == 0.0 {
        return [0.0; 2];
    }
    crate::art_direction::landscape_variation(lat.to_degrees(), lon.to_degrees())
        .map(|value| value * detail)
}

fn linear(rgb: palette::Rgb) -> palette::Rgb {
    rgb.map(|channel| {
        let value = channel / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    })
}

fn srgb(rgb: palette::Rgb) -> [u8; 3] {
    rgb.map(|value| {
        let value = if value <= 0.003_130_8 {
            value * 12.92
        } else {
            1.055 * value.powf(1.0 / 2.4) - 0.055
        };
        (value.clamp(0.0, 1.0) * 255.0).round() as u8
    })
}

pub(crate) fn blend_tints(a: [u8; 3], b: [u8; 3], weight: f32) -> [u8; 3] {
    if weight >= 1.0 {
        b
    } else {
        srgb(lerp3(
            linear(a.map(f32::from)),
            linear(b.map(f32::from)),
            weight,
        ))
    }
}

pub(crate) fn inland_column_color(
    depth_m: f32,
    bottom: palette::Rgb,
    body: palette::Rgb,
    extinction: palette::Rgb,
) -> [u8; 3] {
    // Suspended pigments scatter near the surface even above a deep bed;
    // catalog lakes therefore retain their own optically thick endpoint.
    let bottom = linear(bottom);
    let body = linear(body);
    srgb(std::array::from_fn(|channel| {
        let transmission = (-extinction[channel] * 2.0 * depth_m.max(0.0)).exp();
        bottom[channel] * transmission + body[channel] * (1.0 - transmission)
    }))
}

// Beer-Lambert transport is baked into the existing sRGB vertex tint, then
// decoded by the world shader before lighting and Fresnel. The two-way path
// approximates light reaching the bed and returning to the surface; this is
// an opaque optical proxy, not scene refraction or measured substrate data.
pub(crate) fn column_color(
    depth_m: f32,
    bottom: palette::Rgb,
    body: palette::Rgb,
    extinction: palette::Rgb,
    variation: [f32; 2],
    clear: f32,
) -> [u8; 3] {
    let depth = depth_m.max(0.0);
    let shelf = smoothstep01((depth - 45.0) / 180.0);
    let abyss = smoothstep01((depth - 225.0) / 3_775.0);
    let body = lerp3(
        linear(body),
        linear(palette::water::DEEP),
        0.65 * shelf + 0.35 * abyss,
    );

    // Shared geographic fields describe broad sand banks. Their optical
    // relief vanishes offshore without displacing geometry or changing
    // the contact-query water depth.
    let bank = smoothstep01((-variation[1] - variation[0] * 0.25 - 0.05) / 0.55);
    let optical_depth = depth * (1.0 - bank * 0.78 * clear * (1.0 - shelf));
    // The immediate contact band is wet substrate, not a bright contour
    // painted around every shoreline. Broad submerged shelves retain sand.
    let contact = 0.38 + 0.62 * smoothstep01(depth / 3.0);
    let bottom = linear(bottom).map(|channel| channel * contact);
    let path_m = 2.0 * optical_depth;
    srgb(std::array::from_fn(|channel| {
        let transmission = (-extinction[channel] * path_m).exp();
        bottom[channel] * transmission + body[channel] * (1.0 - transmission)
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clear_water(depth: f32, field: [f32; 2]) -> [u8; 3] {
        column_color(
            depth,
            palette::water::LAGOON,
            palette::water::TROPICAL_MID,
            [0.36, 0.075, 0.043],
            field,
            1.0,
        )
    }

    #[test]
    fn contact_is_wet_substrate_and_the_abyss_loses_all_bed_variation() {
        let bed = [173.0, 219.0, 184.0];
        assert_eq!(
            column_color(0.0, bed, [12.0, 80.0, 100.0], [0.2; 3], [0.0; 2], 0.0),
            [111, 142, 118]
        );
        for depth in [4_000.0, 11_000.0] {
            for field in [[-1.0; 2], [0.0; 2], [1.0; 2]] {
                assert_eq!(
                    clear_water(depth, field),
                    palette::water::DEEP.map(|v| v as u8)
                );
            }
        }
        assert_eq!(clear_water(-1.0, [-1.0; 2]), clear_water(0.0, [-1.0; 2]));
    }

    #[test]
    fn clear_water_runs_from_mint_through_turquoise_to_blue() {
        let mint = clear_water(1.5, [0.0; 2]);
        let turquoise = clear_water(5.0, [0.0; 2]);
        let blue = clear_water(30.0, [0.0; 2]);
        assert!(
            mint[1] > mint[2] && mint[0] > turquoise[0] + 40,
            "{mint:?}, {turquoise:?}"
        );
        assert!(turquoise[1] > turquoise[0] + 80, "{turquoise:?}");
        assert!(blue[2] > blue[1] && blue[1] > blue[0] + 40, "{blue:?}");
        assert!(mint[1] > blue[1] + 40, "{mint:?}, {blue:?}");
    }

    #[test]
    fn shoal_banks_are_visible_only_where_light_can_reach_them() {
        let difference = |depth| {
            let sand = clear_water(depth, [-0.6, -0.3]);
            let rock = clear_water(depth, [0.8, 0.3]);
            sand.into_iter()
                .zip(rock)
                .map(|(a, b)| u32::from(a.abs_diff(b)))
                .sum::<u32>()
        };
        assert!(difference(5.0) > 60);
        assert!(difference(20.0) > 60);
        assert!(difference(200.0) < 2);
    }

    #[test]
    fn geographic_fields_are_continuous_at_the_dateline_and_filter_at_orbit() {
        for lat in [-1.4, 0.0, 1.4] {
            let a = variation(lat, std::f64::consts::PI, 1.0);
            let b = variation(lat, -std::f64::consts::PI, 1.0);
            for channel in 0..2 {
                assert!((a[channel] - b[channel]).abs() < 1e-5);
            }
            assert_eq!(variation(lat, 0.2, 512.0), [0.0; 2]);
            assert_eq!(variation(lat, 0.2, 10_000.0), [0.0; 2]);
            let near = variation(lat, 0.2, 1.0);
            let neighbor = variation(lat + 1e-8, 0.2, 1.0);
            for channel in 0..2 {
                assert!((near[channel] - neighbor[channel]).abs() < 0.002);
            }
            assert_eq!(near, variation(lat, 0.2, 48.0));
        }
    }
}
