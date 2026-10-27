fn linear_reference(srgb: f32) -> f32 {
    if srgb <= 0.04045 {
        srgb / 12.92
    } else {
        ((srgb + 0.055) / 1.055).powf(2.4)
    }
}

fn literal_after(source: &str, marker: &str) -> [f32; 3] {
    let tail = source.split_once(marker).expect("shader marker").1;
    let literal = tail.split_once("vec3<f32>(").expect("RGB literal").1;
    let values: Vec<f32> = literal
        .split_once(')')
        .expect("RGB terminator")
        .0
        .split(',')
        .map(|channel| channel.trim().parse().expect("numeric RGB channel"))
        .collect();
    values.try_into().expect("three RGB channels")
}

fn shader_coefficients() -> [f32; 3] {
    literal_after(
        include_str!("../src/flora.wgsl"),
        "let decode_coefficients =",
    )
}

fn shader_decode(srgb: f32, coefficients: [f32; 3]) -> f32 {
    srgb * (srgb * (srgb * coefficients[0] + coefficients[1]) + coefficients[2])
}

#[test]
fn flora_decode_tracks_the_terrain_srgb_transfer_with_bounded_error() {
    let mut previous = -1.0;
    let coefficients = shader_coefficients();
    for step in 0..=65_536 {
        let input = step as f32 / 65_536.0;
        let output = shader_decode(input, coefficients);
        assert!(output.is_finite() && (0.0..=1.000001).contains(&output));
        assert!(output >= previous);
        assert!(
            (output - linear_reference(input)).abs() < 0.0018,
            "sRGB {input} produced {output}"
        );
        previous = output;
    }
    assert_eq!(shader_decode(0.0, coefficients), 0.0);
    assert!((shader_decode(1.0, coefficients) - 1.0).abs() < 0.000001);
}

#[test]
fn procedural_crop_rows_and_terrain_rows_agree_before_lighting() {
    let coefficients = shader_coefficients();
    let function = include_str!("../src/flora.wgsl")
        .split_once("fn crop_field_color")
        .unwrap()
        .1
        .split_once("fn archetype_color")
        .unwrap()
        .0;
    for (palette, marker) in ["var color =", "case 1u:", "case 2u:", "case 3u:"]
        .into_iter()
        .enumerate()
    {
        let shader = literal_after(function, marker);
        let terrain = vearth_gen::palette::field::CROP[palette].1;
        for channel in 0..3 {
            let authored = terrain[channel] / 255.0;
            assert!((shader[channel] - authored).abs() < 0.000001);
            assert!(
                (shader_decode(shader[channel], coefficients) - linear_reference(authored)).abs()
                    < 0.0018
            );
        }
    }
}
