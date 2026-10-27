//! Shared physical-scale tiles. R stores painted tone or explicit ink coverage,
//! GB store signed height gradients, and A stores perceptual roughness.
//! Periodic multi-scale fields make the tiles seamless; linear mip filtering
//! averages the gradient toward zero so distant surfaces retain their source
//! color instead of changing texture scale with voxel LOD. All data is
//! deterministic. Terrain and construction layers load baked linear data;
//! other fields and all mip levels are generated at startup. Authoring sources
//! live with the assets.

use vearth_gen::material;

/// Tile edge in texels. Shader periods are measured in metres.
pub const TILE: usize = 128;
/// Shared physical families; persistent semantic material ids map onto these.
pub const LAYERS: usize = 55;
/// Complete mip chain, including the mean 1×1 texel.
pub const MIPS: usize = 8;
pub const CHANNELS: usize = 4;

/// Runtime material properties.  Semantic materials can share art while
/// retaining behavior in their material id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MaterialTraits {
    pub atlas_layer: u8,
    pub wettable: bool,
    pub snow_eligible: bool,
}

pub const fn material_traits(material_id: u8) -> MaterialTraits {
    if material::is_foliage(material_id) {
        return MaterialTraits {
            atlas_layer: material::LEAVES,
            wettable: true,
            snow_eligible: true,
        };
    }
    let atlas_layer = match material_id {
        material::SOIL | material::SALT => material::SAND,
        material::ICE => material::SNOW,
        material::VOLCANIC => material::ROCK,
        material::WETLAND => material::GRASS,
        material::AVIATION_BEACON
        | material::LANDMARK_LIGHT
        | material::LANDMARK_SPARKLE
        | material::CITY_LIGHT
        | material::SCULPTURE_METAL => material::STEEL,
        material::FIELD_CROP
        | material::FIELD_FLOWER
        | material::FIELD_ORCHARD
        | material::FIELD_VINEYARD
        | material::FIELD_MEADOW => material::GRASS,
        material::FIELD_DORMANT | material::FIELD_HARVESTED => material::SAND,
        material::BRICK_WALL => 23,
        material::STONE_WALL => 24,
        material::PLASTER_WALL => 25,
        material::TIMBER_WALL => 26,
        material::TILE_ROOF => 27,
        material::SLATE_ROOF => 28,
        material::PANEL_WALL => 29,
        material::METAL_ROOF => 30,
        material::SIDING_WALL => 31,
        material::SHINGLE_ROOF => 32,
        material::FLAT_ROOF => 33,
        material::THATCH_ROOF => 34,
        material::NAMAKO_WALL => 35,
        material::WEATHERED_LIMESTONE => 36,
        material::VEINED_MARBLE => 37,
        material::EARTHEN_PLASTER => 38,
        material::CONCRETE_PATH => 39,
        material::BRICK_PAVING => 40,
        material::STONE_PAVING => 41,
        material::GRAVEL_PATH => 42,
        material::COBBLESTONE => 43,
        material::STONE_SLAB => 44,
        id if vearth_gen::highrise::is_facade(id) => material::WINDOW,
        id if id < 22 => id,
        _ => 22,
    };
    MaterialTraits {
        atlas_layer,
        wettable: !vearth_gen::highrise::is_facade(material_id)
            && !matches!(
                material_id,
                material::AIR
                    | material::WATER
                    | material::SALT
                    | material::ICE
                    | material::WINDOW
                    | material::LIT_WINDOW
                    | material::LIT_FACADE_LOW
                    | material::LIT_FACADE_MEDIUM
                    | material::LIT_FACADE_HIGH
                    | material::AVIATION_BEACON
                    | material::LANDMARK_LIGHT
                    | material::LANDMARK_SPARKLE
                    | material::CITY_LIGHT
                    | material::SCULPTURE_METAL
            ),
        snow_eligible: !vearth_gen::highrise::is_facade(material_id)
            && !matches!(
                material_id,
                material::AIR
                    | material::WATER
                    | material::ICE
                    | material::WINDOW
                    | material::LIT_WINDOW
                    | material::LIT_FACADE_LOW
                    | material::LIT_FACADE_MEDIUM
                    | material::LIT_FACADE_HIGH
                    | material::AVIATION_BEACON
                    | material::LANDMARK_LIGHT
                    | material::LANDMARK_SPARKLE
                    | material::CITY_LIGHT
                    | material::SCULPTURE_METAL
            ),
    }
}

/// Explicit construction finishes share semantic behaviour and one atlas fetch.
/// Variant zero is the default; unassigned variants retain the default art.
pub const fn construction_layer(material_id: u8, variant: u8) -> u8 {
    if material_id == material::NAMAKO_WALL {
        match variant {
            1 => return 53,
            2 => return 54,
            _ => {}
        }
    }
    if material_id == material::ROOF {
        return match variant {
            1 => 51,
            2 => 52,
            3 => 33,
            _ => material::ROOF,
        };
    }
    if variant == 2 && material_id == material::TIMBER_WALL {
        return 50;
    }
    if variant == 3 && material_id == material::METAL_ROOF {
        return 27;
    }
    if variant == 2 && matches!(material_id, material::METAL_ROOF | material::PANEL_WALL) {
        return 45;
    }
    if variant == 1 {
        match material_id {
            material::METAL_ROOF | material::PANEL_WALL => return 45,
            material::TILE_ROOF => return 46,
            material::STONE_WALL => return 47,
            material::TIMBER_WALL => return 48,
            material::SHINGLE_ROOF => return 49,
            _ => {}
        }
    }
    material_traits(material_id).atlas_layer
}

fn hash2(x: usize, y: usize, salt: u64) -> f32 {
    let mut s = (x as u64) << 40 ^ (y as u64) << 20 ^ salt;
    s = s.wrapping_add(0x9E3779B97F4A7C15);
    s = (s ^ (s >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    s = (s ^ (s >> 27)).wrapping_mul(0x94D049BB133111EB);
    ((s >> 40) as f32) / (1u64 << 24) as f32 - 0.5 // -0.5..0.5
}

fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

fn periodic_noise(u: f32, v: f32, frequency: usize, salt: u64) -> f32 {
    let x = u * frequency as f32;
    let y = v * frequency as f32;
    let ix = x.floor() as i32;
    let iy = y.floor() as i32;
    let tx = smooth(x - x.floor());
    let ty = smooth(y - y.floor());
    let value = |dx: i32, dy: i32| {
        hash2(
            (ix + dx).rem_euclid(frequency as i32) as usize,
            (iy + dy).rem_euclid(frequency as i32) as usize,
            salt,
        )
    };
    let lo = value(0, 0) * (1.0 - tx) + value(1, 0) * tx;
    let hi = value(0, 1) * (1.0 - tx) + value(1, 1) * tx;
    lo * (1.0 - ty) + hi * ty
}

/// Tone and relief share broad features. Fine grain adds material character
/// without making every channel independent salt-and-pepper noise.
fn material_field(mat: u8, u: f32, v: f32) -> (f32, f32, f32) {
    let salt = 0x4d35_0000 | u64::from(mat);
    let broad = periodic_noise(u, v, 4, salt);
    let medium = periodic_noise(u, v, 12, salt ^ 0x3a51);
    let fine = periodic_noise(u, v, 32, salt ^ 0x57b9);
    let grain = periodic_noise(u, v, 64, salt ^ 0x841f);
    let field = broad * 0.64 + medium * 0.26 + fine * 0.10;
    let tau = std::f32::consts::TAU;
    let (tone, height, roughness) = match mat {
        material::LEAVES => {
            let clumps = (field * 3.2).tanh();
            (
                clumps * 0.32 + fine * 0.09,
                field * 0.24 + fine * 0.025,
                0.94,
            )
        }
        material::MARBLE => {
            let warped = periodic_noise(u + broad * 0.16, v + medium * 0.07, 7, salt ^ 47);
            let fissure = (1.0 - (warped + broad * 0.15).abs() / 0.035).clamp(0.0, 1.0);
            let relief = broad * 0.85 + medium * 0.18 - fissure * 0.06;
            let strength = 0.25;
            (
                (broad * 0.62 + medium * 0.18 - fissure * 0.085) * strength,
                relief * strength,
                0.88,
            )
        }
        material::SEABED => {
            let ripple = (tau * (v * 13.0 + broad * 0.32)).sin();
            (
                field * 0.38 + grain * 0.035,
                field * 0.08 + ripple * 0.012,
                0.96,
            )
        }
        material::ASPHALT => (
            field * 0.075 + grain * 0.04,
            field * 0.0015 + grain * 0.001,
            0.94,
        ),
        material::PATH => (
            field * 0.18 + fine * 0.04,
            field * 0.004 + fine * 0.001,
            0.97,
        ),
        material::GRAVEL_PATH => {
            let aggregate = periodic_noise(u, v, 48, salt ^ 0x7145);
            (
                field * 0.13 + aggregate * 0.14,
                field * 0.002 + aggregate * 0.007,
                0.96,
            )
        }
        material::CONCRETE_PATH
        | material::BRICK_PAVING
        | material::STONE_PAVING
        | material::STONE_SLAB => {
            // Unit dimensions and joint widths are in metres. Small pavers
            // repeat over 2 m; flags and concrete repeat over 4 m.
            let (columns, rows, period, depth, strength, joint_width, joint_tone) = match mat {
                material::CONCRETE_PATH => (4, 4, 4.0, 0.002, 0.055, 0.014, 0.14),
                material::BRICK_PAVING => (8, 16, 2.0, 0.004, 0.11, 0.010, 0.17),
                material::STONE_SLAB => (4, 8, 4.0, 0.004, 0.12, 0.015, 0.17),
                _ => (8, 10, 2.0, 0.006, 0.14, 0.012, 0.18),
            };
            let row = (v * rows as f32).floor() as i32;
            let offset = if mat == material::CONCRETE_PATH {
                0.0
            } else {
                row.rem_euclid(2) as f32 * 0.5
            };
            let x = u * columns as f32 + offset;
            let y = v * rows as f32;
            let fx = x.rem_euclid(1.0);
            let fy = y.rem_euclid(1.0);
            let edge_m = (fx.min(1.0 - fx) * period / columns as f32)
                .min(fy.min(1.0 - fy) * period / rows as f32);
            let joint = 1.0 - smooth((edge_m / joint_width).clamp(0.0, 1.0));
            // Per-unit colour and crown meet at the mortar, including at the
            // wrapped boundary. Larger flags retain shallow split-stone grain.
            let face = 1.0 - joint;
            let unit = hash2(
                (x.floor() as i32).rem_euclid(columns) as usize,
                row.rem_euclid(rows) as usize,
                salt ^ 0x5041_5645,
            ) * face;
            let crown = if mat == material::STONE_PAVING {
                (std::f32::consts::PI * fx).sin() * (std::f32::consts::PI * fy).sin() * 0.003
            } else {
                0.0
            };
            (
                field * 0.055 + unit * strength - joint * joint_tone,
                field * 0.001 + unit * 0.001 + crown * face - joint * depth,
                0.94,
            )
        }
        material::COBBLESTONE => {
            // Jittered periodic cells make rounded, irregular stones rather
            // than another rectangular bond. This work happens only at atlas
            // creation; runtime cost is the same as any other material tile.
            const CELLS: i32 = 8;
            let p = [u * CELLS as f32, v * CELLS as f32];
            let cell = [p[0].floor() as i32, p[1].floor() as i32];
            let mut nearest = f32::INFINITY;
            let mut second = f32::INFINITY;
            let mut piece = 0.0;
            for dy in -2..=2 {
                for dx in -2..=2 {
                    let x = cell[0] + dx;
                    let y = cell[1] + dy;
                    let wx = x.rem_euclid(CELLS) as usize;
                    let wy = y.rem_euclid(CELLS) as usize;
                    let centre = [
                        x as f32 + 0.5 + hash2(wx, wy, salt ^ 91) * 0.70,
                        y as f32 + 0.5 + hash2(wx, wy, salt ^ 317) * 0.70,
                    ];
                    let distance = ((p[0] - centre[0]).powi(2) + (p[1] - centre[1]).powi(2)).sqrt();
                    if distance < nearest {
                        second = nearest;
                        nearest = distance;
                        piece = hash2(wx, wy, salt ^ 701);
                    } else if distance < second {
                        second = distance;
                    }
                }
            }
            // Intersect the cell with a rounded crown: unconstrained Voronoi
            // corners read as flat polygonal mosaic rather than worn cobbles.
            let edge_m = ((second - nearest) * 0.125).min((0.68 - nearest) * 0.25);
            let face = smooth((edge_m / 0.012).clamp(0.0, 1.0));
            let crown = (1.0 - (nearest / 0.70).powi(2)).clamp(0.0, 1.0) * face;
            (
                piece * face * 0.22 + field * 0.045 - (1.0 - face) * 0.19,
                crown * 0.018 - (1.0 - face) * 0.006 + fine * face * 0.001,
                0.94,
            )
        }
        material::PAVED => (
            field * 0.30 + grain * 0.055,
            field * 0.07 + fine * 0.006,
            0.94,
        ),
        material::WALL
        | material::LIT_FACADE_LOW
        | material::LIT_FACADE_MEDIUM
        | material::LIT_FACADE_HIGH => (field * 0.19 + grain * 0.025, field * 0.028, 0.85),
        material::ROOF => {
            let tiles = (tau * (u * 16.0 + periodic_noise(u, v, 4, salt ^ 19) * 0.04)).cos();
            (
                field * 0.26 + tiles * 0.023,
                field * 0.055 + tiles * 0.007,
                0.83,
            )
        }
        // Metric triplanar axes are not aligned to each branch. Keep bark
        // irregular and shallow so narrow trunks cannot become striped rings.
        material::TRUNK => (field * 0.28, field * 0.012, 0.93),
        material::WATER => (
            0.0,
            periodic_noise(u, v, 5, salt ^ 0x781) * 0.10
                + periodic_noise(u, v, 13, salt ^ 0x915) * 0.04
                + periodic_noise(u, v, 31, salt ^ 0x461) * 0.009,
            0.12,
        ),
        material::WINDOW | material::LIT_WINDOW => (field * 0.035, 0.0, 0.18),
        material::STEEL => (field * 0.065, field * 0.008, 0.35),
        _ => (0.0, 0.0, 0.90),
    };
    let roughness_grain = if mat == material::WATER {
        0.0
    } else {
        fine * 0.05
    };
    (tone, height, (roughness + roughness_grain).clamp(0.08, 1.0))
}

// Raw base levels avoid image decoding and preserve the shared upload path.
fn terrain_tiles() -> &'static [u8] {
    let bytes = vearth_platform::asset_bytes!("../assets/materials/terrain-tiles.rgba8");
    assert_eq!(
        bytes.len(),
        4 * TILE * TILE * CHANNELS,
        "terrain material tiles"
    );
    bytes
}

fn construction_tiles() -> &'static [u8] {
    let bytes = vearth_platform::asset_bytes!("../assets/materials/construction-tiles.rgba8");
    assert_eq!(
        bytes.len(),
        26 * TILE * TILE * CHANNELS,
        "construction material tiles"
    );
    bytes
}

fn tile(layer: u8) -> Vec<u8> {
    if (23..=38).contains(&layer) || (45..=54).contains(&layer) {
        let bytes = TILE * TILE * CHANNELS;
        let index = if layer < 45 {
            layer - 23
        } else {
            layer - 45 + 16
        };
        let start = usize::from(index) * bytes;
        return construction_tiles()[start..start + bytes].to_vec();
    }
    if (material::SAND..=material::SNOW).contains(&layer) {
        let bytes = TILE * TILE * CHANNELS;
        let start = usize::from(layer - material::SAND) * bytes;
        return terrain_tiles()[start..start + bytes].to_vec();
    }
    let mat = match layer {
        39 => material::CONCRETE_PATH,
        40 => material::BRICK_PAVING,
        41 => material::STONE_PAVING,
        42 => material::GRAVEL_PATH,
        43 => material::COBBLESTONE,
        44 => material::STONE_SLAB,
        _ => layer,
    };
    let mut fields = vec![(0.0, 0.0, 1.0); TILE * TILE];
    for y in 0..TILE {
        for x in 0..TILE {
            fields[y * TILE + x] = if matches!(
                mat,
                material::CONCRETE_PATH
                    | material::BRICK_PAVING
                    | material::STONE_PAVING
                    | material::COBBLESTONE
                    | material::STONE_SLAB
            ) {
                // Area coverage keeps narrow joints visible even when a joint
                // falls between texel centres. This is a one-time bake; the
                // renderer still fetches one ordinary filtered atlas tile.
                let mut sum = (0.0, 0.0, 0.0);
                for dy in [0.25, 0.75] {
                    for dx in [0.25, 0.75] {
                        let f = material_field(
                            mat,
                            (x as f32 + dx) / TILE as f32,
                            (y as f32 + dy) / TILE as f32,
                        );
                        sum.0 += f.0 * 0.25;
                        sum.1 += f.1 * 0.25;
                        sum.2 += f.2 * 0.25;
                    }
                }
                sum
            } else {
                material_field(
                    mat,
                    (x as f32 + 0.5) / TILE as f32,
                    (y as f32 + 0.5) / TILE as f32,
                )
            };
        }
    }
    let mean = fields.iter().map(|f| f.0).sum::<f32>() / fields.len() as f32;
    let mut pixels = vec![0; TILE * TILE * CHANNELS];
    let encode = |v: f32| ((v.clamp(0.0, 1.0) * 255.0).round()) as u8;
    for y in 0..TILE {
        for x in 0..TILE {
            let (tone, _, roughness) = fields[y * TILE + x];
            // Central derivatives wrap exactly at the texture seam. Store
            // dH/dUV divided by four; the consumer applies metres-per-tile.
            let height = |x: usize, y: usize| fields[(y % TILE) * TILE + x % TILE].1;
            let dx = (height(x + 1, y) - height(x + TILE - 1, y)) * TILE as f32 * 0.125;
            let dy = (height(x, y + 1) - height(x, y + TILE - 1)) * TILE as f32 * 0.125;
            let at = (y * TILE + x) * CHANNELS;
            pixels[at..at + CHANNELS].copy_from_slice(&[
                encode(0.5 + tone - mean),
                encode(0.5 + dx),
                encode(0.5 + dy),
                encode(roughness),
            ]);
        }
    }
    pixels
}

/// `out[mip][layer]` contains interleaved RGBA bytes. Gradient channels are
/// signed linear data, so ordinary box filtering preserves their mean and
/// naturally reduces bump strength under minification.
pub fn build() -> Vec<Vec<Vec<u8>>> {
    let mut mips: Vec<Vec<Vec<u8>>> = Vec::with_capacity(MIPS);
    mips.push((0..LAYERS as u8).map(tile).collect());
    for mip in 1..MIPS {
        let side = TILE >> mip;
        let prev_side = side * 2;
        let level = mips[mip - 1]
            .iter()
            .enumerate()
            .map(|(layer, previous)| {
                let mut pixels = vec![0; side * side * CHANNELS];
                for y in 0..side {
                    for x in 0..side {
                        for c in 0..CHANNELS {
                            let at = |dx, dy| {
                                previous[((y * 2 + dy) * prev_side + x * 2 + dx) * CHANNELS + c]
                                    as u32
                            };
                            pixels[(y * side + x) * CHANNELS + c] =
                                ((at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1) + 2) / 4) as u8;
                        }
                        if layer >= 23 || layer == usize::from(material::WATER) {
                            // Preserve unresolved surface relief as a
                            // wider microfacet distribution instead of losing
                            // it when gradient mipmaps average toward zero.
                            let mut mean_gradient = [0.0_f32; 2];
                            let mut gradient_energy = 0.0;
                            let mut roughness_energy = 0.0;
                            for dy in 0..2 {
                                for dx in 0..2 {
                                    let at = ((y * 2 + dy) * prev_side + x * 2 + dx) * CHANNELS;
                                    let gradient = [previous[at + 1], previous[at + 2]]
                                        .map(|value| f32::from(value) / 255.0 - 0.5);
                                    for axis in 0..2 {
                                        mean_gradient[axis] += gradient[axis] * 0.25;
                                    }
                                    gradient_energy +=
                                        (gradient[0].powi(2) + gradient[1].powi(2)) * 0.25;
                                    roughness_energy +=
                                        (f32::from(previous[at + 3]) / 255.0).powi(4) * 0.25;
                                }
                            }
                            let variance = (gradient_energy
                                - mean_gradient[0].powi(2)
                                - mean_gradient[1].powi(2))
                            .max(0.0);
                            // Water uses the shared consumer's 1.5 gradient
                            // gain and a 16 m repeat. Convert encoded dH/dUV/4
                            // variance to its physical slope before filtering.
                            let variance_scale = if layer == usize::from(material::WATER) {
                                (4.0_f32 * 1.5 / 16.0).powi(2)
                            } else {
                                1.0
                            };
                            pixels[(y * side + x) * CHANNELS + 3] =
                                ((roughness_energy + variance * variance_scale)
                                    .sqrt()
                                    .sqrt()
                                    .min(1.0)
                                    * 255.0)
                                    .round() as u8;
                        }
                    }
                }
                pixels
            })
            .collect();
        mips.push(level);
    }
    mips
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baked_terrain_is_loaded_without_channel_conversion() {
        let atlas = build();
        for (index, bytes) in terrain_tiles()
            .chunks_exact(TILE * TILE * CHANNELS)
            .enumerate()
        {
            assert_eq!(atlas[0][index + usize::from(material::SAND)], bytes);
            let mut sums = [0_i64; 2];
            for pixel in bytes.chunks_exact(CHANNELS) {
                for axis in 0..2 {
                    assert!((1..255).contains(&pixel[axis + 1]), "clipped relief");
                    sums[axis] += i64::from(pixel[axis + 1]);
                }
            }
            for sum in sums {
                assert!((sum as f64 / (TILE * TILE) as f64 - 127.5).abs() < 0.02);
            }
        }
    }

    #[test]
    fn explicit_variants_are_distinct_and_keep_semantic_defaults() {
        for (material, layer) in [
            (material::METAL_ROOF, 45),
            (material::PANEL_WALL, 45),
            (material::TILE_ROOF, 46),
            (material::STONE_WALL, 47),
            (material::TIMBER_WALL, 48),
            (material::SHINGLE_ROOF, 49),
        ] {
            assert_eq!(construction_layer(material, 1), layer);
            assert_eq!(
                construction_layer(material, 0),
                material_traits(material).atlas_layer
            );
            assert_eq!(
                construction_layer(material, 2),
                if matches!(material, material::METAL_ROOF | material::PANEL_WALL) {
                    45
                } else if material == material::TIMBER_WALL {
                    50
                } else {
                    construction_layer(material, 0)
                }
            );
            assert_ne!(tile(layer), tile(construction_layer(material, 0)));
        }
        assert_eq!(construction_layer(material::TIMBER_WALL, 2), 50);
        assert_ne!(tile(50), tile(26));
        assert_eq!(construction_layer(material::METAL_ROOF, 3), 27);
        assert_eq!(construction_layer(material::PANEL_WALL, 3), 29);
        assert_eq!(construction_layer(material::SIDING_WALL, 1), 31);
        assert_eq!(construction_layer(material::WATER, 1), material::WATER);
        assert_eq!(construction_layer(material::ROOF, 1), 51);
        assert_eq!(construction_layer(material::ROOF, 2), 52);
        assert_eq!(construction_layer(material::ROOF, 3), 33);
        assert_ne!(tile(51), tile(45));
        assert_ne!(tile(52), tile(34));
        assert_eq!(construction_layer(material::NAMAKO_WALL, 0), 35);
        assert_eq!(construction_layer(material::NAMAKO_WALL, 1), 53);
        assert_eq!(construction_layer(material::NAMAKO_WALL, 2), 54);
        assert_eq!(construction_layer(material::NAMAKO_WALL, 3), 35);
        assert_ne!(tile(53), tile(35));
        assert_ne!(tile(54), tile(53));
    }

    #[test]
    fn baked_construction_matches_manifest_and_retains_metric_relief() {
        let manifest: serde_json::Value = serde_json::from_str(include_str!(
            "../assets/materials/construction-tiles.source.json"
        ))
        .unwrap();
        assert_eq!(manifest["dimensions"], serde_json::json!([TILE, TILE]));
        assert_eq!(manifest["bytes"], construction_tiles().len());
        assert_eq!(manifest["layers"].as_object().unwrap().len(), 26);
        let atlas = build();
        for (index, source) in construction_tiles()
            .chunks_exact(TILE * TILE * CHANNELS)
            .enumerate()
        {
            assert_eq!(
                atlas[0][if index < 16 { index + 23 } else { index + 29 }],
                source
            );
            let mut means = [0.0_f64; 3];
            let mut roughness = std::collections::BTreeSet::new();
            for pixel in source.chunks_exact(CHANNELS) {
                for channel in 0..3 {
                    assert!(
                        (1..255).contains(&pixel[channel]),
                        "clipped construction data"
                    );
                    means[channel] += f64::from(pixel[channel]) / (TILE * TILE) as f64;
                }
                roughness.insert(pixel[3]);
            }
            if index != 25 {
                assert!((means[0] - 127.5).abs() < 0.02, "construction tint drift");
            }
            assert!((means[1] - 127.5).abs() <= 0.5);
            assert!((means[2] - 127.5).abs() <= 0.5);
            assert!(roughness.len() > 8, "finish has uniform roughness");
            // A periodic height field has zero integrated horizontal slope
            // along every row and vertical slope along every column. The bake
            // checks curl before filtering; taking coarse central differences
            // here would incorrectly reject correctly integrated narrow ribs.
            for line in 0..TILE {
                let horizontal: f64 = (0..TILE)
                    .map(|x| f64::from(source[(line * TILE + x) * CHANNELS + 1]) - 127.5)
                    .sum();
                let vertical: f64 = (0..TILE)
                    .map(|y| f64::from(source[(y * TILE + line) * CHANNELS + 2]) - 127.5)
                    .sum();
                assert!(
                    horizontal.abs() <= TILE as f64 * 0.5,
                    "nonperiodic horizontal slope"
                );
                assert!(
                    vertical.abs() <= TILE as f64 * 0.5,
                    "nonperiodic vertical slope"
                );
            }
        }
    }

    #[test]
    fn narrow_corrugations_retain_their_filtered_metric_slope() {
        let atlas = build();
        let peak = atlas[0][45]
            .chunks_exact(CHANNELS)
            .map(|p| (f32::from(p[1]) / 255.0 - 0.5).abs())
            .fold(0.0_f32, f32::max);
        // A 16 mm deep, 76 mm pitch sinusoidal rib has a 0.65 peak slope.
        // Pixel integration reduces it to ~0.47; coarse differencing would
        // incorrectly reduce it again to ~0.11 before the GPU even samples it.
        assert!((0.43..0.49).contains(&peak), "corrugated slope {peak}");
    }

    #[test]
    fn construction_finishes_retain_material_character_without_distant_tint_drift() {
        let atlas = build();
        for layer in (23..=38).chain(45..=53) {
            let fine = &atlas[0][layer];
            assert!(atlas[0][23..layer].iter().all(|other| fine != other));
            let tones: Vec<_> = fine
                .chunks_exact(CHANNELS)
                .map(|p| f64::from(p[0]))
                .collect();
            let variance =
                tones.iter().map(|v| (v - 127.5).powi(2)).sum::<f64>() / tones.len() as f64;
            assert!(variance > 4.0, "construction finish lacks tone structure");
            let distant = &atlas[MIPS - 1][layer];
            for channel in 0..3 {
                assert!((i32::from(distant[channel]) - 128).abs() <= 4);
            }
        }
        let mean_roughness = |layer: usize| {
            atlas[0][layer]
                .chunks_exact(CHANNELS)
                .map(|p| f32::from(p[3]))
                .sum::<f32>()
                / (TILE * TILE) as f32
        };
        assert!(mean_roughness(30) < mean_roughness(27)); // sheet metal / clay tile
        assert!(mean_roughness(27) < mean_roughness(24)); // clay tile / masonry
        assert!(mean_roughness(37) < mean_roughness(36)); // marble / limestone
    }

    #[test]
    fn legacy_material_atlas_fingerprint() {
        let mut fingerprint = 0xcbf2_9ce4_8422_2325_u64;
        for layers in build() {
            for (layer, bytes) in layers.iter().take(23).enumerate() {
                if (usize::from(material::SAND)..=usize::from(material::SNOW)).contains(&layer) {
                    continue;
                }
                for byte in bytes {
                    fingerprint = (fingerprint ^ u64::from(*byte)).wrapping_mul(0x100_0000_01b3);
                }
            }
        }
        assert_eq!(fingerprint, 0xfeaf467ee6e5d71f);
    }

    #[test]
    fn atlas_has_complete_mips_and_preserves_mean_material_color() {
        let atlas = build();
        assert_eq!(atlas, build());
        assert_eq!(atlas.len(), MIPS);
        for (mip, layers) in atlas.iter().enumerate() {
            assert_eq!(layers.len(), LAYERS);
            for (layer, data) in layers.iter().enumerate() {
                assert_eq!(data.len(), (TILE >> mip).pow(2) * CHANNELS);
                let mean = data
                    .chunks_exact(CHANNELS)
                    .map(|p| f64::from(p[0]))
                    .sum::<f64>()
                    / (data.len() / CHANNELS) as f64;
                if layer != 54 {
                    assert!((124.0..132.0).contains(&mean), "mip {mip}: {mean}");
                }
            }
        }
        for mat in [
            material::GRASS,
            material::ROCK,
            material::SAND,
            material::TRUNK,
        ] {
            let p = &atlas[MIPS - 1][mat as usize];
            assert!((i32::from(p[1]) - 128).abs() < 9);
            assert!((i32::from(p[2]) - 128).abs() < 9);
        }
        let bytes: usize = atlas.iter().flatten().map(Vec::len).sum();
        assert_eq!(bytes, 4_805_900, "shared atlas GPU payload budget");
    }

    #[test]
    fn road_finishes_are_distinct_muted_and_filter_to_flat_surfaces() {
        let atlas = build();
        let ids = [
            material::ASPHALT,
            material::PATH,
            material::CONCRETE_PATH,
            material::BRICK_PAVING,
            material::STONE_PAVING,
            material::GRAVEL_PATH,
            material::COBBLESTONE,
            material::STONE_SLAB,
        ];
        for (i, id) in ids.into_iter().enumerate() {
            let layer = usize::from(material_traits(id).atlas_layer);
            assert!(material_traits(id).wettable && material_traits(id).snow_eligible);
            let fine = &atlas[0][layer];
            for previous in &ids[..i] {
                assert_ne!(
                    fine,
                    &atlas[0][usize::from(material_traits(*previous).atlas_layer)]
                );
            }
            for pixel in fine.chunks_exact(4) {
                let darkest = if id == material::COBBLESTONE { 80 } else { 85 };
                assert!(
                    (darkest..=170).contains(&pixel[0]),
                    "road finish contrast: {id}/{pixel:?}"
                );
                assert!(
                    (1..255).contains(&pixel[1]) && (1..255).contains(&pixel[2]),
                    "clipped road gradient"
                );
                assert!(pixel[3] >= 225, "road finish must stay matte");
            }
            let distant = &atlas[MIPS - 1][layer];
            assert!((i32::from(distant[1]) - 128).abs() <= 3);
            assert!((i32::from(distant[2]) - 128).abs() <= 3);
        }
    }

    #[test]
    fn paving_joints_survive_close_range_sampling_without_distant_colour_shift() {
        let atlas = build();
        for id in [
            material::CONCRETE_PATH,
            material::BRICK_PAVING,
            material::STONE_PAVING,
            material::COBBLESTONE,
            material::STONE_SLAB,
        ] {
            let layer = usize::from(material_traits(id).atlas_layer);
            let tones: Vec<_> = atlas[0][layer]
                .chunks_exact(CHANNELS)
                .map(|p| p[0])
                .collect();
            let mut sorted = tones.clone();
            sorted.sort_unstable();
            let contrast = sorted[sorted.len() * 9 / 10] - sorted[sorted.len() / 10];
            // A resolved joint needs a measurable lower tail, not merely a
            // different material id and imperceptible aggregate noise.
            assert!(
                contrast >= 12,
                "unreadable paving joints for {id}: {contrast}"
            );
            let distant = &atlas[MIPS - 1][layer];
            assert!((i32::from(distant[0]) - 128).abs() <= 3);
        }
        let concrete = &atlas[0][usize::from(material_traits(material::CONCRETE_PATH).atlas_layer)];
        let tone = |x: usize, y: usize| concrete[(y * TILE + x) * CHANNELS];
        assert!(
            tone(4, 16) > tone(0, 16) + 3,
            "subtexel concrete joint lost its area coverage"
        );
        let cobbles = material::COBBLESTONE;
        for i in 0..71 {
            let v = (i as f32 + 0.37) / 71.0;
            let epsilon = 0.0001;
            for axis in 0..2 {
                let f = |u| {
                    if axis == 0 {
                        material_field(cobbles, u, v)
                    } else {
                        material_field(cobbles, v, u)
                    }
                };
                let left = f(-epsilon);
                let wrapped = f(1.0 - epsilon);
                assert!((left.0 - wrapped.0).abs() < 0.0001);
                assert!((left.1 - wrapped.1).abs() < 0.00001);
                let before = f(0.0);
                let after = f(1.0);
                let slope0 = (before.1 - left.1) / epsilon;
                let slope1 = (after.1 - wrapped.1) / epsilon;
                assert!(
                    (slope0 - slope1).abs() < 0.002,
                    "cobble seam slope {slope0}/{slope1}"
                );
            }
        }
    }

    #[test]
    fn material_fields_are_periodic_in_value_and_slope() {
        for mat in [
            material::ASPHALT,
            material::PATH,
            material::CONCRETE_PATH,
            material::BRICK_PAVING,
            material::STONE_PAVING,
            material::GRAVEL_PATH,
            material::COBBLESTONE,
            material::STONE_SLAB,
            material::TRUNK,
            material::ROOF,
            material::WATER,
        ] {
            for step in 0..=32 {
                let v = step as f32 / 32.0;
                let left = material_field(mat, 0.0, v);
                let right = material_field(mat, 1.0, v);
                assert!((left.0 - right.0).abs() < 0.0001);
                assert!((left.1 - right.1).abs() < 0.0001);
                let bottom = material_field(mat, v, 0.0);
                let top = material_field(mat, v, 1.0);
                assert!((bottom.0 - top.0).abs() < 0.0001);
                assert!((bottom.1 - top.1).abs() < 0.0001);
            }
        }
    }

    #[test]
    fn construction_materials_preserve_relief_energy_under_minification() {
        let atlas = build();
        for id in (material::BRICK_WALL..=material::NAMAKO_WALL).chain([
            material::WEATHERED_LIMESTONE,
            material::VEINED_MARBLE,
            material::EARTHEN_PLASTER,
        ]) {
            let properties = material_traits(id);
            let layer = properties.atlas_layer as usize;
            assert!((23..LAYERS).contains(&layer));
            assert!(properties.wettable && properties.snow_eligible);
            let fine = &atlas[0][layer];
            let mean_energy = fine
                .chunks_exact(CHANNELS)
                .map(|pixel| (f32::from(pixel[3]) / 255.0).powi(4))
                .sum::<f32>()
                / (TILE * TILE) as f32;
            let filtered = (f32::from(atlas[MIPS - 1][layer][3]) / 255.0).powi(4);
            assert!(
                filtered + 0.012 >= mean_energy,
                "minification sharpened construction material {id}"
            );
            assert!(
                fine.chunks_exact(CHANNELS)
                    .any(|pixel| (i32::from(pixel[1]) - 128).abs()
                        + (i32::from(pixel[2]) - 128).abs()
                        > 2)
            );
        }
    }

    #[test]
    fn water_residual_filters_into_roughness_without_repainting_its_body() {
        let atlas = build();
        let layer = usize::from(material::WATER);
        let mut slope_energies = Vec::new();
        let mut total_energies = Vec::new();
        for mip in &atlas {
            let mut slopes = 0.0;
            let mut total = 0.0;
            for pixel in mip[layer].chunks_exact(CHANNELS) {
                assert_eq!(
                    pixel[0], 128,
                    "water residual must not recolor source water"
                );
                let slope = [pixel[1], pixel[2]]
                    .map(|value| (f32::from(value) / 255.0 - 0.5) * (4.0 * 1.5 / 16.0));
                let energy = slope[0].powi(2) + slope[1].powi(2);
                slopes += energy;
                total += energy + (f32::from(pixel[3]) / 255.0).powi(4);
            }
            let count = (mip[layer].len() / CHANNELS) as f32;
            slope_energies.push(slopes / count);
            total_energies.push(total / count);
        }
        eprintln!("water residual slope energies {slope_energies:?}; total {total_energies:?}");
        assert!(slope_energies[0] > 0.0001);
        assert!(slope_energies[MIPS - 1] < slope_energies[0] * 0.02);
        assert!(
            slope_energies
                .windows(2)
                .all(|pair| pair[1] <= pair[0] + 0.00001)
        );
        assert!(
            total_energies[MIPS - 1] > total_energies[0] * 0.85,
            "filtered residual slope energy vanished rather than broadening the lobe"
        );
        assert!(
            total_energies[MIPS - 1] < total_energies[0] * 1.20,
            "filtering invented excessive roughness"
        );
    }

    #[test]
    fn water_residual_has_no_dominant_straight_wave_component() {
        let atlas = build();
        let side = TILE >> 2;
        let gradients: Vec<[f32; 2]> = atlas[2][usize::from(material::WATER)]
            .chunks_exact(CHANNELS)
            .map(|pixel| [pixel[1], pixel[2]].map(|v| f32::from(v) / 255.0 - 0.5))
            .collect();
        let total = gradients
            .iter()
            .map(|g| g[0].powi(2) + g[1].powi(2))
            .sum::<f32>()
            / gradients.len() as f32;
        let mut largest = 0.0_f32;
        // Measure the rendered gradient tile's Fourier concentration. A pair
        // of planar sine waves fails even when its phases are randomized.
        for fy in -15_i32..=15 {
            for fx in 0_i32..=15 {
                if fx == 0 && fy <= 0 {
                    continue;
                }
                let mut cosine = [0.0_f32; 2];
                let mut sine = [0.0_f32; 2];
                for (index, gradient) in gradients.iter().enumerate() {
                    let angle = std::f32::consts::TAU
                        * (fx * (index % side) as i32 + fy * (index / side) as i32) as f32
                        / side as f32;
                    let (sin, cos) = angle.sin_cos();
                    for axis in 0..2 {
                        cosine[axis] += gradient[axis] * cos / gradients.len() as f32;
                        sine[axis] += gradient[axis] * sin / gradients.len() as f32;
                    }
                }
                let energy = 2.0
                    * (cosine[0].powi(2) + cosine[1].powi(2) + sine[0].powi(2) + sine[1].powi(2));
                largest = largest.max(energy / total);
            }
        }
        eprintln!("largest water residual Fourier component fraction {largest}");
        assert!(
            largest < 0.15,
            "water texture is dominated by a straight repeating wave"
        );
    }

    #[test]
    fn every_semantic_foliage_material_uses_leaf_art() {
        for id in material::FOLIAGE_BASE..material::FOLIAGE_END {
            assert_eq!(material_traits(id).atlas_layer, material::LEAVES);
            assert!(vearth_gen::material::foliage_traits(id).is_some());
        }
    }

    #[test]
    fn expanded_surface_semantics_reuse_the_intended_physical_art() {
        assert_eq!(material_traits(material::SOIL).atlas_layer, material::SAND);
        assert_eq!(material_traits(material::SALT).atlas_layer, material::SAND);
        assert_eq!(material_traits(material::ICE).atlas_layer, material::SNOW);
        assert_eq!(
            material_traits(material::VOLCANIC).atlas_layer,
            material::ROCK
        );
        assert_eq!(
            material_traits(material::WETLAND).atlas_layer,
            material::GRASS
        );
        assert_eq!(
            material_traits(material::AVIATION_BEACON).atlas_layer,
            material::STEEL
        );
        for id in [material::LANDMARK_LIGHT, material::LANDMARK_SPARKLE] {
            assert_eq!(material_traits(id).atlas_layer, material::STEEL);
            assert!(!material_traits(id).wettable);
            assert!(!material_traits(id).snow_eligible);
        }
        for id in material::FIELD_CROP..=material::FIELD_MEADOW {
            assert_eq!(material_traits(id).atlas_layer, material::GRASS);
        }
        for id in [material::FIELD_DORMANT, material::FIELD_HARVESTED] {
            assert_eq!(material_traits(id).atlas_layer, material::SAND);
        }
        assert!(!material_traits(material::AVIATION_BEACON).wettable);
        assert!(!material_traits(material::AVIATION_BEACON).snow_eligible);
        assert!(!material_traits(material::ICE).wettable);
        assert!(!material_traits(material::ICE).snow_eligible);
    }

    #[test]
    fn material_tiles_carry_continuous_relief_and_distinct_roughness() {
        let atlas = build();
        for mat in [
            material::GRASS,
            material::ROCK,
            material::SAND,
            material::TRUNK,
        ] {
            let tones = atlas[0][mat as usize]
                .chunks_exact(CHANNELS)
                .map(|p| p[0])
                .collect::<std::collections::BTreeSet<_>>();
            assert!(tones.len() > 24, "material {mat}: {} tones", tones.len());
            let gradient_range = atlas[0][mat as usize]
                .chunks_exact(CHANNELS)
                .map(|p| {
                    (i32::from(p[1]) - 128)
                        .abs()
                        .max((i32::from(p[2]) - 128).abs())
                })
                .max()
                .unwrap();
            if mat == material::TRUNK {
                assert!(
                    (1..=8).contains(&gradient_range),
                    "bark must have shallow relief"
                );
            } else {
                assert!(gradient_range > 8);
            }
        }
        let roughness = |mat: u8| atlas[MIPS - 1][mat as usize][3];
        assert!(roughness(material::ROCK) > roughness(material::STEEL));
        assert!(roughness(material::STEEL) > roughness(material::WATER));
    }

    #[test]
    fn glazed_pattern_mips_preserve_two_pigment_mean() {
        let atlas = build();
        let ground = [0.806952258_f64, 0.822785754, 0.752942217];
        let ink = [0.014443844_f64, 0.076185381, 0.300543794];
        let mean_coverage = |pixels: &[u8]| {
            pixels
                .chunks_exact(CHANNELS)
                .map(|p| f64::from(p[0]) / 255.0)
                .sum::<f64>()
                / (pixels.len() / CHANNELS) as f64
        };
        let source_mean = mean_coverage(&atlas[0][54]);
        assert!((0.2..0.65).contains(&source_mean));
        for layers in &atlas {
            let filtered = mean_coverage(&layers[54]);
            assert!((filtered - source_mean).abs() < 0.009, "ink coverage drift");
            for channel in 0..3 {
                let source_color =
                    ground[channel] * (1.0 - source_mean) + ink[channel] * source_mean;
                let filtered_color = ground[channel] * (1.0 - filtered) + ink[channel] * filtered;
                assert!((source_color - filtered_color).abs() < 0.008);
            }
        }
        let fine = &atlas[0][54];
        let range = fine.chunks_exact(CHANNELS).map(|p| p[0]);
        assert!(range.clone().max().unwrap() - range.min().unwrap() > 160);
        let roughness = |layer: usize| {
            atlas[0][layer]
                .chunks_exact(CHANNELS)
                .map(|p| f64::from(p[3]) / 255.0)
                .sum::<f64>()
                / (TILE * TILE) as f64
        };
        assert!(roughness(53) < 0.45 && roughness(54) < 0.45);
        assert!(
            roughness(35) > 0.7,
            "namako rough plaster joints must remain distinct"
        );
    }
}
