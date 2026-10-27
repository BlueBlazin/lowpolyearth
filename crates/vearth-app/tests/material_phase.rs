const WORLD_SHADER: &str = include_str!("../src/shader.wgsl");
const RENDERER: &str = include_str!("../src/gfx.rs");

fn literal_after(source: &str, prefix: &str, end: char) -> f64 {
    source
        .split_once(prefix)
        .expect("material coordinate expression")
        .1
        .split_once(end)
        .expect("numeric expression terminator")
        .0
        .trim()
        .parse()
        .expect("literal physical coordinate scale")
}

fn circular_distance(a: f32, b: f32) -> f32 {
    let difference = (a - b).rem_euclid(1.0);
    difference.min(1.0 - difference)
}

#[test]
fn physical_material_phase_survives_camera_wrap_and_fractional_translation() {
    let initializer = RENDERER
        .rsplit_once("material_origin: [")
        .expect("material origin upload")
        .1;
    let modulus = literal_after(initializer, "cam_pos.x.rem_euclid(", ')');
    assert_eq!(
        modulus,
        literal_after(initializer, "cam_pos.y.rem_euclid(", ')')
    );
    assert_eq!(
        modulus,
        literal_after(initializer, "cam_pos.z.rem_euclid(", ')')
    );
    let mut periods: Vec<f64> = WORLD_SHADER
        .split("period_m = ")
        .skip(1)
        .map(|tail| tail.split_once(';').unwrap().0.parse().unwrap())
        .collect();
    assert!(!periods.is_empty());
    let terrain = include_str!("../src/terrain_material.wgsl");
    periods.extend(
        terrain
            .split_once("= array<f32, 4>(")
            .unwrap()
            .1
            .split_once(')')
            .unwrap()
            .0
            .split(',')
            .map(|value| value.trim().parse::<f64>().unwrap()),
    );
    let selector = WORLD_SHADER.split_once("fn material_selector(").unwrap().1;
    let selector_spacing = literal_after(selector, "let p = position_m / ", ';');
    let selector_mask = literal_after(WORLD_SHADER, "cell & vec3<i32>(", ')') as i64;
    assert!((selector_mask as u64 + 1).is_power_of_two());
    assert_eq!(
        modulus / selector_spacing,
        (selector_mask + 1) as f64,
        "selector hash and texture origin must have the same physical wrap"
    );
    periods.push(selector_spacing);
    assert!(WORLD_SHADER.contains("in.world_pos + g.material_origin.xyz"));

    for period in periods {
        let tiles_per_wrap = modulus / period;
        assert_eq!(
            tiles_per_wrap.fract(),
            0.0,
            "a camera wrap must span whole texture tiles and selector cells"
        );
        for wrap in [-769.0, -1.0, 0.0, 1.0, 777.0] {
            for point_offset in [-127.375, -0.15625, 0.03125, 21.1875, 193.6875] {
                let point = wrap * modulus + point_offset;
                let exact = (point / period).rem_euclid(1.0) as f32;
                let mut previous = None;
                // Sub-metre camera movement crosses the modulus boundary
                // while this physical surface point remains fixed. Arithmetic
                // matches the f64 CPU origin and f32 camera-relative shader.
                for camera_offset in [-0.75, -0.13, -0.001, 0.0, 0.001, 0.13, 0.75] {
                    let camera = wrap * modulus + camera_offset;
                    let relative = (point - camera) as f32;
                    let origin = camera.rem_euclid(modulus) as f32;
                    let coordinate = (relative + origin) / period as f32;
                    let uv = coordinate.rem_euclid(1.0);
                    if period == selector_spacing {
                        // Identical wrapped lattice corners and fractions
                        // preserve all eight selector noise inputs and
                        // interpolation weights, independent of hash values.
                        let expected_cell = (point / period).floor() as i64 & selector_mask;
                        let actual_cell = coordinate.floor() as i64 & selector_mask;
                        assert_eq!(
                            actual_cell, expected_cell,
                            "camera wrap changed selector noise cells"
                        );
                    }
                    assert!(
                        circular_distance(uv, exact) < 0.0006,
                        "physical phase drift: camera={camera} point={point} period={period} actual={uv} expected={exact}"
                    );
                    if let Some(before) = previous {
                        assert!(
                            circular_distance(uv, before) < 0.0012,
                            "camera wrap changed surface material phase"
                        );
                    }
                    previous = Some(uv);
                }
            }
        }
    }
}
