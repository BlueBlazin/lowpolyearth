//! Terrain pigment: colour structure below the resolution of source imagery.
//!
//! One global heuristic of weathering processes adds bedding, fall-line
//! runnels and gullies, moisture patches and slope aspect to natural terrain
//! corners. It is conditioned only on the corner's physical family, surface
//! normal, altitude and position; it never selects a family, crosses a
//! semantic boundary or references a place. Fields are evaluated in ECEF
//! metres, so shared corners of neighbouring cells agree exactly. Each octave
//! fades before it reaches the corner spacing (or the vertical step between
//! corners, for altitude bands), so coarse cells converge to the source colour
//! instead of aliasing.

use glam::DVec3;

/// Physical response family, matching the renderer's terrain blend weights.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Family {
    Aggregate,
    Vegetation,
    Rock,
    Snow,
}

/// Geometry of one shared terrain corner.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Corner {
    /// ECEF position of the terrain surface, metres.
    pub position: DVec3,
    /// Unit geodetic up.
    pub up: DVec3,
    /// Unit world-space surface normal.
    pub normal: DVec3,
    /// Geodetic height, metres.
    pub altitude_m: f64,
    /// Horizontal spacing to neighbouring corners, metres.
    pub spacing_m: f64,
    /// Largest height change to a neighbouring corner, metres.
    pub altitude_step_m: f64,
}

/// Linear-light multiplier encoded as `round(128 * m)`; 128 is neutral.
pub(crate) const NEUTRAL: [u8; 3] = [128; 3];

/// Apply the pigment to an unlit sRGB tint. `strength` in [0, 1] scales it.
pub(crate) fn pigment(tint: [u8; 3], family: Family, corner: &Corner, strength: f64) -> [u8; 3] {
    let strength = strength * orbital_fade(corner.spacing_m);
    if strength <= 0.0 {
        return tint;
    }
    let albedo = tint.map(srgb_to_linear);
    let result = evaluate(albedo, family, corner);
    std::array::from_fn(|channel| {
        linear_to_srgb(albedo[channel] + (result[channel] - albedo[channel]) * strength)
    })
}

/// Encoded linear-light ratio of the pigment for a surface whose own colour
/// is owned elsewhere (cultivated fields keep a per-field tint).
pub(crate) fn multiplier(tint: [u8; 3], family: Family, corner: &Corner, strength: f64) -> [u8; 3] {
    let strength = strength * orbital_fade(corner.spacing_m);
    if strength <= 0.0 {
        return NEUTRAL;
    }
    let albedo = tint.map(srgb_to_linear);
    let result = evaluate(albedo, family, corner);
    std::array::from_fn(|channel| {
        let ratio = result[channel] / albedo[channel].max(1e-4);
        (128.0 * (1.0 + (ratio - 1.0) * strength))
            .round()
            .clamp(0.0, 255.0) as u8
    })
}

/// Scale an sRGB tint by an encoded linear-light multiplier.
pub(crate) fn apply_multiplier(tint: [u8; 3], multiplier: [u8; 3]) -> [u8; 3] {
    if multiplier == NEUTRAL {
        return tint;
    }
    std::array::from_fn(|channel| {
        linear_to_srgb(srgb_to_linear(tint[channel]) * f64::from(multiplier[channel]) / 128.0)
    })
}

fn orbital_fade(spacing_m: f64) -> f64 {
    1.0 - smoothstep(384.0, 1536.0, spacing_m)
}

/// An octave with `cell_m` lattice cells needs several corners per cell.
fn visibility(cell_m: f64, spacing_m: f64) -> f64 {
    1.0 - smoothstep(cell_m * 0.20, cell_m * 0.45, spacing_m)
}

fn evaluate(albedo: [f64; 3], family: Family, corner: &Corner) -> [f64; 3] {
    let Corner {
        position,
        up,
        normal,
        spacing_m,
        ..
    } = *corner;
    let steep = 1.0 - normal.dot(up).clamp(0.0, 1.0);
    // 1 - cos: 0.03 at 14 degrees, 0.13 at 30, 0.29 at 45, 0.5 at 60.
    let cliff = smoothstep(0.14, 0.55, steep);
    let sloped = smoothstep(0.03, 0.12, steep);
    let value = luminance(albedo);

    let v1024 = visibility(1024.0, spacing_m);
    let v256 = visibility(256.0, spacing_m);
    let v64 = visibility(64.0, spacing_m);
    let v16 = visibility(16.0, spacing_m);
    let octave = |cell: f64, visible: f64, salt: u64| {
        if visible > 0.0 {
            noise(position / cell, salt) * visible
        } else {
            0.0
        }
    };
    let regional = octave(1024.0, v1024, 0x6d2b);
    let broad = octave(256.0, v256, 0x51a3);
    let meso = octave(64.0, v64, 0x93c1);
    let clump = octave(16.0, v16, 0x2e67);
    // Patchiness renormalized over the resolved octaves, so coarse cells keep
    // kilometre-scale variation instead of fading to one flat tint. Value
    // noise has a standard deviation near 0.15; this scale gives the macro
    // fields a deviation near 0.4.
    let resolved = (0.49 * v1024 * v1024 + 0.36 * v256 * v256 + 0.25 * v64 * v64).sqrt();
    let scale = 2.6 / resolved.max(0.25);
    let moisture = (regional * 0.7 + broad * 0.6 + meso * 0.5) * scale;
    let tone = (regional * 0.5 - broad * 0.4 + meso * 0.6) * scale;

    // Fall-line field shared by every family: rills, gullies, chutes and runnels.
    let (flow, runnel) = if sloped > 0.0 {
        let vf16 = v16;
        let vf64 = v64;
        let f16 = if vf16 > 0.0 {
            fall_line_noise(corner, 16.0, 0x22) * vf16
        } else {
            0.0
        };
        let f64_ = if vf64 > 0.0 {
            fall_line_noise(corner, 64.0, 0x23) * vf64
        } else {
            0.0
        };
        let resolved = (0.20 * vf16 * vf16 + 0.12 * vf64 * vf64).sqrt();
        if resolved > 0.0 {
            let norm = 0.62 / resolved.max(0.12);
            let flow = (f16 * 0.45 + f64_ * 0.35) * norm * sloped;
            // Thin runnels follow the zero crossings; their width grows with
            // the field's change between corners so they stay resolvable.
            let step = spacing_m * (0.028 * vf16 + 0.0055 * vf64) * norm;
            let width = 0.03 + 1.5 * step;
            let runnel = (1.0 - smoothstep(width * 0.5, width * 1.5, flow.abs())) * sloped;
            (flow, runnel)
        } else {
            (0.0, 0.0)
        }
    } else {
        (0.0, 0.0)
    };

    match family {
        Family::Vegetation => {
            // Moisture patches: drier ground yellows, wetter hollows deepen.
            // Slopes facing the equator dry first; gullies stay wetter.
            let north = DVec3::Z - up * up.z;
            let equatorward = -north * up.z.signum() / north.length().max(1e-4);
            let sunward = (normal - up * normal.dot(up)).dot(equatorward)
                * smoothstep(0.10, 0.45, up.z.abs());
            let dry = (-moisture - clump * 1.7 - flow * 0.6 + sunward * 0.9).clamp(-1.0, 1.0);
            let tint = if dry > 0.0 {
                [1.28, 1.07, 0.66]
            } else {
                [0.72, 0.89, 0.80]
            };
            let gain = (tone * 0.35 + clump * 0.95 + sunward * 0.12 - flow * 0.22).exp2();
            let c: [f64; 3] = std::array::from_fn(|i| {
                albedo[i] * (1.0 + (tint[i] - 1.0) * dry.abs() * 0.80) * gain
            });
            // Steep turf breaks into outcrops before the rock family owns it.
            let exposure = smoothstep(0.10, 0.42, steep);
            let outcrop = smoothstep(
                0.0,
                0.08,
                clump + meso * 0.7 - flow * 0.35 + exposure * 0.6 - 0.62,
            ) * exposure;
            let stone = (value * 1.25).clamp(0.035, 0.18);
            let stone = [stone * 1.03, stone, stone * 0.94];
            clamp_albedo(std::array::from_fn(|i| {
                c[i] + (stone[i] - c[i]) * outcrop * 0.6
            }))
        }
        Family::Rock | Family::Aggregate => {
            let rock = family == Family::Rock;
            let bedding = strata(albedo, value, corner, cliff, broad, meso, clump);
            // Staining runs down steep faces, strongest below member contacts.
            let run = 1.0
                + (smoothstep(0.05, 0.95, (bedding.bedded / 31.0).rem_euclid(1.0)) - 1.0)
                    * 0.25
                    * bedding.member_visible;
            let envelope =
                smoothstep(-0.18, 0.18, moisture * 0.15 + meso * 0.6 + clump * 0.3) * run * cliff;
            let streak = (runnel * 0.7).max(smoothstep(0.04, 0.22, flow)) * envelope;
            let bloom = smoothstep(0.10, 0.26, -flow) * envelope * 0.6;
            let iron = smoothstep(0.02, 0.24, broad + meso * 0.6 + clump * 0.5);
            if rock {
                let gain = (bedding.tone * 0.75 + tone * 0.22 + clump * 0.60
                    - flow * 0.12 * (1.0 - cliff))
                    .exp2();
                let hue = if bedding.hue > 0.0 {
                    [1.16, 1.0, 0.80]
                } else {
                    [0.90, 0.97, 1.08]
                };
                let hue_amount = (bedding.hue.abs() * 0.9).min(0.6);
                let stain = lerp3([0.40, 0.40, 0.42], [1.02, 0.70, 0.46], iron);
                clamp_albedo(std::array::from_fn(|i| {
                    let c = albedo[i] * gain * (1.0 + (hue[i] - 1.0) * hue_amount);
                    let c = c * (1.0 + (stain[i] - 1.0) * streak * 0.85);
                    c * (1.0 + ([1.30, 1.28, 1.22][i] - 1.0) * bloom)
                }))
            } else {
                // Bare ground: warm/pale soil and sand patches, darker fines in
                // rills, fresh pale debris down chutes.
                let gain =
                    (tone * 0.40 + clump * 0.85 + bedding.tone * 0.35 * cliff - flow * 0.30).exp2();
                let hue = if moisture > 0.0 {
                    [1.16, 0.99, 0.78]
                } else {
                    [0.90, 0.97, 1.08]
                };
                let hue_amount = (moisture.abs() * 0.80).min(0.70);
                let stain = lerp3([0.62, 0.61, 0.62], [1.08, 0.84, 0.62], iron);
                clamp_albedo(std::array::from_fn(|i| {
                    let c = albedo[i] * gain * (1.0 + (hue[i] - 1.0) * hue_amount);
                    c * (1.0 + (stain[i] - 1.0) * streak * 0.5 * cliff)
                }))
            }
        }
        Family::Snow => {
            let gain = (tone * 0.05 + clump * 0.14 - flow * 0.08).exp2();
            clamp_albedo(albedo.map(|channel| channel * gain))
        }
    }
}

struct Bedding {
    tone: f64,
    hue: f64,
    bedded: f64,
    member_visible: f64,
}

/// Folded strata: altitude warped by the broad fields. There is no lithology
/// source, so chroma is the cue: warm, chromatic rock (red beds, sandstone,
/// shale) is layered; neutral rock (granite, gneiss, massive limestone) keeps
/// only faint, broken banding and its runoff.
fn strata(
    albedo: [f64; 3],
    value: f64,
    corner: &Corner,
    cliff: f64,
    broad: f64,
    meso: f64,
    clump: f64,
) -> Bedding {
    let warmth = (albedo[0] - albedo[2]) / value.max(0.01);
    let layered = smoothstep(0.15, 0.60, warmth);
    let bedded = corner.altitude_m + broad * 70.0 + meso * 16.0;
    let band_visibility = |band: f64| visibility(band, corner.altitude_step_m);
    let vb = band_visibility(9.0);
    let vm = band_visibility(31.0);
    let vc = band_visibility(120.0);
    let vg = band_visibility(400.0) * layered;
    let continuity = 0.35 * smoothstep(-0.05, 0.25, meso + clump * 0.6) * (1.0 - layered) + layered;
    let amount = (0.15 + 0.85 * cliff) * continuity;
    let band = |band_m: f64, visible: f64, salt: u64| {
        if visible > 0.0 {
            bed(bedded, band_m, salt)
        } else {
            [0.0, 0.0]
        }
    };
    let b = band(9.0, vb, 0x12);
    let m = band(31.0, vm, 0x13);
    let c = band(120.0, vc, 0x14);
    let g = band(400.0, vg, 0x17);
    Bedding {
        tone: (b[0] * 0.60 * vb + m[0] * 0.75 * vm + c[0] * 0.70 * vc + g[0] * 0.60 * vg) * amount,
        hue: (m[1] * vm + c[1] * 0.8 * vc + g[1] * 0.7 * vg) * amount,
        bedded,
        member_visible: vm,
    }
}

/// Signed bed values `[tone, hue]` along altitude. Each bed holds one value
/// and meets the next over the upper quarter of its band, like a sedimentary
/// contact.
fn bed(altitude_m: f64, band_m: f64, salt: u64) -> [f64; 2] {
    let q = altitude_m / band_m;
    let index = q.floor();
    let t = smoothstep(0.72, 1.0, q - index);
    let index = index as i64;
    std::array::from_fn(|channel| {
        let salt = salt ^ ((channel as u64) << 40);
        let a = lattice([index, 0, 0], salt);
        let b = lattice([index + 1, 0, 0], salt);
        a + (b - a) * t - 0.5
    })
}

const FLOW_PHASE_M: f64 = 128.0;

/// Two-phase fall-line noise. Offsetting the lookup by the altitude phase
/// along the downhill direction cancels motion along the local fall line, so
/// the field forms rills and gullies on slopes and vertical runnels on cliffs.
/// The phase wraps every [`FLOW_PHASE_M`]; the two phases are complementary,
/// so the wrap seams are hidden and the offset stays short where curvature
/// bends it.
fn fall_line_noise(corner: &Corner, cell_m: f64, salt: u64) -> f64 {
    let Corner {
        position,
        up,
        normal,
        altitude_m,
        ..
    } = *corner;
    let horizontal = normal - up * normal.dot(up);
    let run = horizontal.length().max(1e-3);
    // Level ground has no fall line; cap the offset below about 14 degrees.
    let downhill = horizontal / run * (normal.dot(up) / run).min(4.0);
    let t = (altitude_m / FLOW_PHASE_M).rem_euclid(1.0);
    let a = t * FLOW_PHASE_M;
    let b = (t + 0.5).rem_euclid(1.0) * FLOW_PHASE_M;
    let wa = 1.0 - (2.0 * t - 1.0).abs();
    let na = noise((position - up * a + downhill * a) / cell_m, salt);
    let nb = noise((position - up * b + downhill * b) / cell_m, salt ^ 0x5bd1);
    (na * wa + nb * (1.0 - wa)) / (wa * wa + (1.0 - wa) * (1.0 - wa)).sqrt()
}

/// Signed C1 value noise on the integer lattice, roughly in [-0.5, 0.5].
fn noise(p: DVec3, salt: u64) -> f64 {
    let base = p.floor();
    let f = p - base;
    let w = f * f * (DVec3::splat(3.0) - 2.0 * f);
    let cell = [base.x as i64, base.y as i64, base.z as i64];
    let mut value = 0.0;
    for corner in 0..8 {
        let offset = [corner & 1, (corner >> 1) & 1, (corner >> 2) & 1];
        let weight = [w.x, w.y, w.z]
            .iter()
            .zip(offset)
            .map(|(&w, o)| if o == 1 { w } else { 1.0 - w })
            .product::<f64>();
        value += lattice(
            [
                cell[0] + offset[0],
                cell[1] + offset[1],
                cell[2] + offset[2],
            ],
            salt,
        ) * weight;
    }
    value - 0.5
}

fn lattice(cell: [i64; 3], salt: u64) -> f64 {
    let mut x = salt.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    for axis in cell {
        x ^= axis as u64;
        x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        x ^= x >> 31;
    }
    (x >> 11) as f64 / (1u64 << 53) as f64
}

fn smoothstep(edge0: f64, edge1: f64, x: f64) -> f64 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn lerp3(a: [f64; 3], b: [f64; 3], t: f64) -> [f64; 3] {
    std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
}

fn luminance(rgb: [f64; 3]) -> f64 {
    rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722
}

fn clamp_albedo(rgb: [f64; 3]) -> [f64; 3] {
    rgb.map(|channel| channel.clamp(0.0, 0.92))
}

fn srgb_to_linear(channel: u8) -> f64 {
    let c = f64::from(channel) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(channel: f64) -> u8 {
    let c = channel.clamp(0.0, 1.0);
    let s = if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    (s * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corner(position: DVec3, normal: DVec3, spacing_m: f64) -> Corner {
        let up = position.normalize();
        Corner {
            position,
            up,
            normal,
            altitude_m: position.length() - 6_371_000.0,
            spacing_m,
            altitude_step_m: spacing_m * 2.0,
        }
    }

    #[test]
    fn neutral_multiplier_and_zero_strength_preserve_the_tint() {
        let p = DVec3::new(4_000_000.0, 600_000.0, 4_900_000.0);
        let c = corner(p, p.normalize(), 3.0);
        let tint = [96, 128, 64];
        assert_eq!(pigment(tint, Family::Vegetation, &c, 0.0), tint);
        assert_eq!(apply_multiplier(tint, NEUTRAL), tint);
        assert_eq!(multiplier(tint, Family::Rock, &c, 0.0), NEUTRAL);
    }

    #[test]
    fn orbital_spacing_retains_source_colour() {
        let p = DVec3::new(4_000_000.0, 600_000.0, 4_900_000.0);
        let c = corner(p, p.normalize(), 4_000.0);
        let tint = [180, 90, 60];
        for family in [
            Family::Aggregate,
            Family::Vegetation,
            Family::Rock,
            Family::Snow,
        ] {
            assert_eq!(pigment(tint, family, &c, 1.0), tint);
        }
    }

    #[test]
    fn pigment_varies_continuously_and_boundedly_over_the_ground() {
        let origin = DVec3::new(4_000_000.0, 600_000.0, 4_900_000.0);
        let up = origin.normalize();
        let east = DVec3::Z.cross(up).normalize();
        let tint = [110, 140, 70];
        let mut low = [255u8; 3];
        let mut high = [0u8; 3];
        let mut previous: Option<[u8; 3]> = None;
        for step in 0..2_000 {
            let p = origin + east * (step as f64 * 1.5);
            let out = pigment(tint, Family::Vegetation, &corner(p, up, 2.4), 1.0);
            for channel in 0..3 {
                low[channel] = low[channel].min(out[channel]);
                high[channel] = high[channel].max(out[channel]);
            }
            if let Some(previous) = previous {
                for channel in 0..3 {
                    assert!(
                        out[channel].abs_diff(previous[channel]) <= 12,
                        "step {step}"
                    );
                }
            }
            previous = Some(out);
        }
        assert!(high[1] - low[1] > 20, "too flat: {low:?}..{high:?}");
        assert!(high[1] - low[1] < 160, "too extreme: {low:?}..{high:?}");
    }

    #[test]
    fn cliff_runoff_is_elongated_along_the_fall_line() {
        let origin = DVec3::new(4_000_000.0, 600_000.0, 4_900_000.0);
        let up = origin.normalize();
        let east = DVec3::Z.cross(up).normalize();
        // A near-vertical face whose normal points east.
        let normal = (east + up * 0.05).normalize();
        let sample = |p: DVec3| {
            fall_line_noise(
                &Corner {
                    position: p,
                    up,
                    normal,
                    altitude_m: p.dot(up) - 6_371_000.0,
                    spacing_m: 2.0,
                    altitude_step_m: 2.0,
                },
                16.0,
                0x22,
            )
        };
        let north = up.cross(east);
        let (mut along, mut across) = (0.0, 0.0);
        for step in 0..400 {
            let p = origin + north * (step as f64 * 7.0);
            along += (sample(p + up * 4.0) - sample(p)).abs();
            across += (sample(p + north * 4.0) - sample(p)).abs();
        }
        assert!(along * 2.5 < across, "along {along} across {across}");
    }
}
