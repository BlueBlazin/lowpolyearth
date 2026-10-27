//! Deterministic PNG comparison rules for render-contract captures.

use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImageThresholds {
    pub require_exact: bool,
    /// Normalized channel error (`0..=1`).
    pub max_absolute: f64,
    pub p99_absolute: f64,
    pub root_mean_square: f64,
    pub max_relative: f64,
    pub max_changed_pixel_fraction: f64,
}

impl ImageThresholds {
    pub const EXACT: Self = Self {
        require_exact: true,
        max_absolute: 0.0,
        p99_absolute: 0.0,
        root_mean_square: 0.0,
        max_relative: 0.0,
        max_changed_pixel_fraction: 0.0,
    };

    /// Fixed before shader optimization. This admits at most two 8-bit values
    /// of adapter-dependent floating-point drift, with only 0.1% of pixels
    /// allowed to differ at all. Structural/material/AO changes exceed it by
    /// orders of magnitude in the intentional-failure tests.
    pub const DOCUMENTED_GPU_FLOAT: Self = Self {
        require_exact: false,
        max_absolute: 2.0 / 255.0,
        p99_absolute: 1.0 / 255.0,
        root_mean_square: 0.5 / 255.0,
        max_relative: 0.02,
        max_changed_pixel_fraction: 0.001,
    };
}

fn portable_thresholds(value: &serde_json::Value) -> Result<ImageThresholds, String> {
    let thresholds = value
        .pointer("/fixed_metadata/thresholds/documented_gpu_float")
        .and_then(serde_json::Value::as_object)
        .ok_or("capture manifest needs documented_gpu_float thresholds")?;
    let number = |field: &str| {
        thresholds
            .get(field)
            .and_then(serde_json::Value::as_f64)
            .ok_or_else(|| format!("capture manifest threshold {field} must be a number"))
    };
    Ok(ImageThresholds {
        require_exact: false,
        max_absolute: number("max_absolute")?,
        p99_absolute: number("p99_absolute")?,
        root_mean_square: number("root_mean_square")?,
        max_relative: number("max_relative")?,
        max_changed_pixel_fraction: number("max_changed_pixel_fraction")?,
    })
}

pub fn portable_thresholds_from_manifest_json(json: &str) -> Result<ImageThresholds, String> {
    let value: serde_json::Value = serde_json::from_str(json).map_err(|error| error.to_string())?;
    portable_thresholds(&value)
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ImageMetrics {
    pub width: u32,
    pub height: u32,
    pub exact: bool,
    pub max_absolute: f64,
    pub p99_absolute: f64,
    pub root_mean_square: f64,
    pub max_relative: f64,
    pub changed_pixels: u64,
    pub changed_pixel_fraction: f64,
}

impl ImageMetrics {
    pub fn accepted(self, thresholds: ImageThresholds) -> bool {
        if thresholds.require_exact {
            return self.exact;
        }
        self.max_absolute <= thresholds.max_absolute
            && self.p99_absolute <= thresholds.p99_absolute
            && self.root_mean_square <= thresholds.root_mean_square
            && self.max_relative <= thresholds.max_relative
            && self.changed_pixel_fraction <= thresholds.max_changed_pixel_fraction
    }
}

#[derive(Clone, Debug)]
pub struct ImageComparison {
    pub metrics: ImageMetrics,
    pub difference_rgba: Vec<u8>,
}

pub fn compare_rgba(
    width: u32,
    height: u32,
    baseline: &[u8],
    candidate: &[u8],
) -> Result<ImageComparison, String> {
    let expected = width as usize * height as usize * 4;
    if baseline.len() != expected || candidate.len() != expected {
        return Err(format!(
            "{width}x{height} RGBA comparison needs {expected} bytes (got {} and {})",
            baseline.len(),
            candidate.len()
        ));
    }
    let mut errors = Vec::with_capacity(expected);
    let mut difference = Vec::with_capacity(expected);
    let mut sum_sq = 0.0;
    let mut max_absolute = 0.0f64;
    let mut max_relative = 0.0f64;
    let mut changed_pixels = 0u64;
    for (base, candidate) in baseline.chunks_exact(4).zip(candidate.chunks_exact(4)) {
        let mut pixel_changed = false;
        for channel in 0..4 {
            let absolute_byte = base[channel].abs_diff(candidate[channel]);
            let absolute = f64::from(absolute_byte) / 255.0;
            let relative = f64::from(absolute_byte) / f64::from(base[channel].max(1));
            errors.push(absolute);
            sum_sq += absolute * absolute;
            max_absolute = max_absolute.max(absolute);
            max_relative = max_relative.max(relative);
            pixel_changed |= absolute_byte != 0;
            difference.push(absolute_byte.saturating_mul(8));
        }
        *difference.last_mut().expect("one RGBA pixel was appended") = 255;
        changed_pixels += u64::from(pixel_changed);
    }
    errors.sort_unstable_by(f64::total_cmp);
    let p99_index = ((errors.len().saturating_sub(1)) as f64 * 0.99).ceil() as usize;
    let pixels = u64::from(width) * u64::from(height);
    Ok(ImageComparison {
        metrics: ImageMetrics {
            width,
            height,
            exact: changed_pixels == 0,
            max_absolute,
            p99_absolute: errors.get(p99_index).copied().unwrap_or_default(),
            root_mean_square: (sum_sq / errors.len().max(1) as f64).sqrt(),
            max_relative,
            changed_pixels,
            changed_pixel_fraction: changed_pixels as f64 / pixels.max(1) as f64,
        },
        difference_rgba: difference,
    })
}

fn read_png(path: &Path) -> Result<(u32, u32, Vec<u8>), Box<dyn std::error::Error>> {
    let file = std::fs::File::open(path)?;
    let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info()?;
    let mut bytes = vec![
        0;
        reader
            .output_buffer_size()
            .ok_or("PNG output is too large")?
    ];
    let info = reader.next_frame(&mut bytes)?;
    let source = &bytes[..info.buffer_size()];
    let mut rgba = Vec::with_capacity(info.width as usize * info.height as usize * 4);
    match info.color_type {
        png::ColorType::Rgba => rgba.extend_from_slice(source),
        png::ColorType::Rgb => {
            for pixel in source.chunks_exact(3) {
                rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
            }
        }
        png::ColorType::Grayscale => {
            for &value in source {
                rgba.extend_from_slice(&[value, value, value, 255]);
            }
        }
        png::ColorType::GrayscaleAlpha => {
            for pixel in source.chunks_exact(2) {
                rgba.extend_from_slice(&[pixel[0], pixel[0], pixel[0], pixel[1]]);
            }
        }
        png::ColorType::Indexed => {
            return Err("normalized PNG unexpectedly remained indexed".into());
        }
    }
    Ok((info.width, info.height, rgba))
}

pub fn compare_pngs(
    baseline: impl AsRef<Path>,
    candidate: impl AsRef<Path>,
    difference: impl AsRef<Path>,
    thresholds: ImageThresholds,
) -> Result<ImageMetrics, Box<dyn std::error::Error>> {
    let (width, height, baseline) = read_png(baseline.as_ref())?;
    let (candidate_width, candidate_height, candidate) = read_png(candidate.as_ref())?;
    if (width, height) != (candidate_width, candidate_height) {
        return Err(format!(
            "image dimensions differ: {width}x{height} vs {candidate_width}x{candidate_height}"
        )
        .into());
    }
    let comparison = compare_rgba(width, height, &baseline, &candidate)?;
    if !comparison.metrics.accepted(thresholds) {
        crate::capture::write_png(difference, width, height, &comparison.difference_rgba)?;
    }
    Ok(comparison.metrics)
}

pub const REQUIRED_CAPTURE_TAGS: [&str; 25] = [
    "face-orientations",
    "camera-distance",
    "lod-transition",
    "near-plane",
    "water",
    "glass",
    "emissive",
    "foliage",
    "fields",
    "snow",
    "wetness",
    "day",
    "night",
    "fog",
    "lightning",
    "landmarks",
    "skirts",
    "lips",
    "calibration",
    "tint",
    "material",
    "ambient-occlusion",
    "geometric-face",
    "shading-face",
    "lod-predicate",
];

pub fn validate_capture_manifest_json(json: &str) -> Result<(), String> {
    let value: serde_json::Value = serde_json::from_str(json).map_err(|error| error.to_string())?;
    if value.get("schema_version").and_then(|value| value.as_u64()) != Some(1) {
        return Err("capture manifest schema_version must be 1".into());
    }
    let metadata = value
        .get("fixed_metadata")
        .and_then(|value| value.as_object())
        .ok_or("capture manifest needs fixed_metadata")?;
    for field in [
        "resolution",
        "adapter_selection",
        "time",
        "weather",
        "exposure",
        "random_seed",
        "asset_version",
        "thresholds",
    ] {
        if !metadata.contains_key(field) {
            return Err(format!("capture manifest fixed_metadata needs {field}"));
        }
    }
    portable_thresholds(&value)?;
    let cases = value
        .get("cases")
        .and_then(|value| value.as_array())
        .ok_or("capture manifest needs cases")?;
    if cases.is_empty() {
        return Err("capture manifest has no cases".into());
    }
    let mut covered = std::collections::BTreeSet::new();
    for case in cases {
        let object = case.as_object().ok_or("capture case must be an object")?;
        if object.get("id").and_then(|value| value.as_str()).is_none() {
            return Err("each capture case needs id and command".into());
        }
        let command = object
            .get("command")
            .and_then(|value| value.as_array())
            .ok_or("each capture case needs id and command")?
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .ok_or("capture command arguments must be strings")
            })
            .collect::<Result<Vec<_>, _>>()?;
        for (index, argument) in command.iter().enumerate() {
            let pitch = if *argument == "--pitch" {
                Some(
                    command
                        .get(index + 1)
                        .ok_or("capture --pitch needs a value")?
                        .parse::<f64>()
                        .map_err(|_| "capture --pitch must be a number")?,
                )
            } else if *argument == "--at" {
                command
                    .get(index + 1)
                    .and_then(|position| position.split(',').nth(4))
                    .map(|pitch| {
                        pitch
                            .parse::<f64>()
                            .map_err(|_| "capture --at pitch must be a number")
                    })
                    .transpose()?
            } else {
                None
            };
            if pitch.is_some_and(|pitch| !pitch.is_finite() || !(0.0..=89.9).contains(&pitch)) {
                return Err("capture pitch must be in [0, 89.9]".into());
            }
        }
        for tag in object
            .get("tags")
            .and_then(|value| value.as_array())
            .ok_or("each capture case needs tags")?
        {
            covered.insert(tag.as_str().ok_or("capture tag must be a string")?);
        }
    }
    let missing: Vec<_> = REQUIRED_CAPTURE_TAGS
        .into_iter()
        .filter(|tag| !covered.contains(tag))
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "capture manifest misses tags: {}",
            missing.join(", ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn baseline() -> Vec<u8> {
        let mut image = vec![0u8; 40 * 40 * 4];
        for (index, pixel) in image.chunks_exact_mut(4).enumerate() {
            pixel.copy_from_slice(&[
                (index * 3) as u8,
                80 + (index % 40) as u8,
                180 - (index % 30) as u8,
                255,
            ]);
        }
        image
    }

    fn assert_mutation_fails(mut mutate: impl FnMut(&mut [u8])) {
        let expected = baseline();
        let mut actual = expected.clone();
        mutate(&mut actual);
        let comparison = compare_rgba(40, 40, &expected, &actual).unwrap();
        assert!(
            !comparison
                .metrics
                .accepted(ImageThresholds::DOCUMENTED_GPU_FLOAT)
        );
        assert!(!comparison.metrics.exact);
    }

    #[test]
    fn intentional_tint_material_ao_face_and_lod_changes_fail() {
        // Tint.
        assert_mutation_fails(|pixels| pixels.chunks_exact_mut(4).for_each(|p| p[0] ^= 32));
        // Material/atlas response.
        assert_mutation_fails(|pixels| pixels[0..32].fill(210));
        // One AO corner changes a visible gradient region.
        assert_mutation_fails(|pixels| pixels[32..96].iter_mut().for_each(|p| *p /= 2));
        // Geometric/shading face response changes a directional band.
        assert_mutation_fails(|pixels| pixels[96..160].rotate_left(4));
        // LOD predicate removes a primitive band.
        assert_mutation_fails(|pixels| pixels[160..224].fill(0));
    }

    #[test]
    fn documented_threshold_accepts_only_sparse_rounding_drift() {
        let expected = baseline();
        let mut actual = expected.clone();
        actual[1] = actual[1].saturating_add(1);
        let metrics = compare_rgba(40, 40, &expected, &actual).unwrap().metrics;
        assert!(!metrics.exact);
        assert!(metrics.accepted(ImageThresholds::DOCUMENTED_GPU_FLOAT));
    }

    #[test]
    fn checked_in_capture_manifest_is_complete() {
        let manifest = include_str!("../testdata/contracts/capture-manifest.json");
        validate_capture_manifest_json(manifest).unwrap();
        assert_eq!(
            portable_thresholds_from_manifest_json(manifest).unwrap(),
            ImageThresholds::DOCUMENTED_GPU_FLOAT
        );
    }

    #[test]
    fn capture_manifest_rejects_out_of_domain_pitch() {
        let manifest = include_str!("../testdata/contracts/capture-manifest.json").replacen(
            "\"--pitch\", \"28\"",
            "\"--pitch\", \"-28\"",
            1,
        );
        assert_eq!(
            validate_capture_manifest_json(&manifest).unwrap_err(),
            "capture pitch must be in [0, 89.9]"
        );
    }
}
