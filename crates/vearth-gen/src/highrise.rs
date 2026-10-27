//! Production facade systems for mapped tall envelopes. Geometry and measured
//! roofs remain source-owned; these systems supply finishes and bounded relief.
//! Panel RGB owns the opaque wall pigment. Coarse variant 1 instead
//! stores the integrated wall/glass mean, with no additional mesh, vertex bytes,
//! texture samples, or atlas layers. Unresolved reflections use a profile
//! estimate; the resolver owns the selected system and every surface pigment.

use crate::regional_architecture::BuildingDesign;
use vearth_data::vectors::BuildingUse;

pub(crate) const MIN_HEIGHT_M: f32 = 36.0;

/// Established procedural prior when no explicit high-rise recipe is applicable.
/// This chooses a representation; it does not relabel unknown source use.
pub(crate) fn fallback_system(
    use_kind: BuildingUse,
    height: f32,
    seed: u64,
    urban_glazed_prior: bool,
) -> System {
    let key = seed ^ (seed >> 29) ^ (seed >> 47);
    let office = matches!(use_kind, BuildingUse::Commercial | BuildingUse::Civic)
        || (use_kind == BuildingUse::Unknown
            && (urban_glazed_prior || (height >= 90.0 && key % 4 != 0)));
    if office {
        match key % 5 {
            0 => System::ClearOffice,
            1 => System::SilverOffice,
            2 => System::BronzeOffice,
            3 => System::StoneOffice,
            _ => System::HorizontalOffice,
        }
    } else {
        match key % 4 {
            0 => System::ConcreteFrame,
            1 => System::RecessedResidential,
            _ => System::Residential,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum System {
    ClearOffice,
    SilverOffice,
    BronzeOffice,
    StoneOffice,
    HorizontalOffice,
    Residential,
    RecessedResidential,
    ConcreteFrame,
}

#[derive(Clone, Copy, Debug)]
pub struct Profile {
    pub floor_m: f64,
    pub bay_m: f64,
    pub glazing: (f64, f64),
    pub pier_m: f64,
    pub balcony_m: f64,
    pub mechanical_every: u32,
    pub wall: [u8; 3],
    pub glass: [u8; 3],
    pub trim: [u8; 3],
}

pub(crate) struct PanelLayout {
    pub count: u32,
    pub width: f64,
    pub pier: f64,
    pub bottom_inset: f64,
    pub top_inset: f64,
}

impl System {
    pub const ALL: [Self; 8] = [
        Self::ClearOffice,
        Self::SilverOffice,
        Self::BronzeOffice,
        Self::StoneOffice,
        Self::HorizontalOffice,
        Self::Residential,
        Self::RecessedResidential,
        Self::ConcreteFrame,
    ];
    pub fn profile(self) -> Profile {
        use System::*;
        let mut p = Profile {
            floor_m: 4.0,
            bay_m: 2.0,
            glazing: (0.12, 0.90),
            pier_m: 0.12,
            balcony_m: 0.0,
            mechanical_every: 16,
            wall: [104, 117, 123],
            glass: [91, 129, 145],
            trim: [181, 187, 185],
        };
        match self {
            ClearOffice => {}
            SilverOffice => {
                p.glass = [137, 157, 165];
                p.wall = [165, 170, 169];
                p.pier_m = 0.20;
            }
            BronzeOffice => {
                p.glass = [129, 113, 91];
                p.wall = [92, 83, 70];
                p.trim = [151, 135, 111];
            }
            StoneOffice => {
                p.bay_m = 4.0;
                p.glass = [78, 99, 108];
                p.wall = [179, 166, 143];
                p.trim = [207, 194, 172];
                p.pier_m = 0.70;
                p.glazing = (0.22, 0.88);
            }
            HorizontalOffice => {
                p.glass = [85, 114, 126];
                p.wall = [163, 170, 167];
                p.glazing = (0.30, 0.88);
                p.pier_m = 0.08;
            }
            Residential | RecessedResidential => {
                p.floor_m = 3.2;
                p.bay_m = 4.0;
                p.mechanical_every = 0;
                p.wall = [188, 184, 172];
                p.glass = [77, 104, 118];
                p.trim = [212, 207, 193];
                p.glazing = (0.20, 0.83);
                p.pier_m = 0.65;
                p.balcony_m = if self == Residential { 0.75 } else { 0.0 };
                if self == RecessedResidential {
                    p.wall = [164, 158, 143];
                    p.pier_m = 0.95;
                }
            }
            ConcreteFrame => {
                p.floor_m = 3.2;
                p.bay_m = 4.0;
                p.mechanical_every = 0;
                p.wall = [151, 152, 145];
                p.glass = [73, 96, 106];
                p.trim = [184, 181, 168];
                p.glazing = (0.28, 0.82);
                p.pier_m = 1.1;
            }
        }
        p
    }

    pub const fn residential(self) -> bool {
        matches!(
            self,
            Self::Residential | Self::RecessedResidential | Self::ConcreteFrame
        )
    }

    pub(crate) fn panel_layout(self, source_length: f64, height: f64) -> PanelLayout {
        let p = self.profile();
        let count = (source_length / p.bay_m.max(4.0)).round().clamp(1.0, 12.0) as u32;
        let width = source_length / f64::from(count);
        PanelLayout {
            count,
            width,
            pier: p
                .pier_m
                .max(if self.residential() { 0.65 } else { 0.32 })
                .min(width * 0.30),
            bottom_inset: p.floor_m.min(height * 0.12),
            top_inset: (p.floor_m * 0.55).min(height * 0.10),
        }
    }

    // Variant 1 on facade materials stores the already integrated
    // linear-light wall/glass mean in sRGB bytes. It is not a glass tint.
    pub(crate) fn coarse_tint(
        self,
        wall: [u8; 3],
        source_length: f64,
        along: [f64; 2],
        base: f64,
        floor: f64,
        eave: f64,
    ) -> [u8; 3] {
        if eave - floor < 4.0 || along[1] - along[0] < 0.02 {
            return wall;
        }
        let p = self.profile();
        let layout = self.panel_layout(source_length, eave - floor);
        // Integrate partial floors with the shader's geodetic phase, rather
        // than assuming the panel span contains an integer number of floors.
        // Its 256 m phase is a common multiple of all floor periods.
        let integral = |x: f64, low: f64, high: f64| {
            let x = x - low;
            let whole = x.floor();
            whole * (high - low) + (x - whole).min(high - low)
        };
        let row_glazed_height = (integral(
            (eave - layout.top_inset) / p.floor_m,
            p.glazing.0,
            p.glazing.1,
        ) - integral(
            (floor + layout.bottom_inset) / p.floor_m,
            p.glazing.0,
            p.glazing.1,
        ))
        .max(0.0)
            * p.floor_m;
        let panel_height = (eave - layout.top_inset - floor - layout.bottom_inset).max(0.0);
        let glazed_height = if self.residential() {
            row_glazed_height
        } else {
            0.72 * panel_height + 0.28 * row_glazed_height
        };
        let low = layout.pier / layout.width;
        let span = along.map(|x| x.clamp(0.0, source_length));
        let glazed_width = (integral(span[1] / layout.width, low, 1.0 - low)
            - integral(span[0] / layout.width, low, 1.0 - low))
        .max(0.0)
            * layout.width;
        let coverage =
            (glazed_width / (along[1] - along[0]) * glazed_height / (eave - base)).clamp(0.0, 1.0);
        let linear = |c: u8| {
            let c = f64::from(c) / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        std::array::from_fn(|i| {
            let c = linear(wall[i]) * (1.0 - coverage) + linear(p.glass[i]) * coverage;
            let srgb = if c <= 0.0031308 {
                c * 12.92
            } else {
                1.055 * c.powf(1.0 / 2.4) - 0.055
            };
            (srgb * 255.0).round().clamp(0.0, 255.0) as u8
        })
    }

    pub const fn facade_material(self, night: bool) -> u8 {
        // Two contiguous ranges share shader behavior; unlit source parts can
        // never gain night emission through a facade fallback.
        103 + self as u8 + if night { 8 } else { 0 }
    }
}

/// Read an already resolved contract. No use classification or palette choice
/// belongs to a geometry consumer.
pub fn resolve(design: BuildingDesign, height: f32, _seed: u64) -> Option<System> {
    (height.is_finite() && height >= MIN_HEIGHT_M)
        .then_some(design.facade)
        .flatten()
}

pub const fn is_facade(material: u8) -> bool {
    material >= 103 && material <= 118
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn coarse_residential_mean_preserves_linear_light_and_actual_panel_area() {
        let system = System::Residential;
        let wall = [224, 156, 64];
        // A 24 m edge has six 4 m bays. A 120 m edge reaches the existing
        // twelve-bay cap, so its larger panel fraction must change the mean.
        assert_eq!(
            system.coarse_tint(wall, 24.0, [0.0, 24.0], 100.0, 100.0, 280.0),
            [182, 137, 91]
        );
        assert_eq!(
            system.coarse_tint(wall, 120.0, [0.0, 120.0], 100.0, 100.0, 280.0),
            [167, 131, 97]
        );
        let layout = system.panel_layout(24.0, 180.0);
        assert_eq!(layout.count, 6);
        assert_eq!(layout.width, 4.0);
        assert_eq!(layout.pier, 0.65);
        // Equal-height envelopes at different floor phases have different
        // clipped openings. A fragment wholly within a pier has no glass.
        assert_eq!(
            system.coarse_tint(wall, 24.0, [0.0, 24.0], 100.0, 100.0, 137.7),
            [187, 139, 89]
        );
        assert_eq!(
            system.coarse_tint(wall, 24.0, [0.0, 24.0], 102.0, 102.0, 139.7),
            [188, 140, 88]
        );
        assert_eq!(
            system.coarse_tint(wall, 24.0, [0.1, 0.5], 100.0, 100.0, 280.0),
            wall
        );
    }

    #[test]
    fn coarse_pigment_matches_area_sampled_panels_for_every_system() {
        let wall = [220, 142, 68];
        let linear = |c: u8| {
            let c = f64::from(c) / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        for system in System::ALL {
            let profile = system.profile();
            for base in [-29.3, 100.7] {
                let floor = base + 1.2;
                let eave = base + 37.7;
                let layout = system.panel_layout(24.0, eave - floor);
                let mut sampled_glass = 0.0;
                const SAMPLES: usize = 600;
                for x_index in 0..SAMPLES {
                    let x = 3.2 + (19.7 - 3.2) * (x_index as f64 + 0.5) / SAMPLES as f64;
                    let in_panel = (0..layout.count).any(|bay| {
                        x > f64::from(bay) * layout.width + layout.pier
                            && x < f64::from(bay + 1) * layout.width - layout.pier
                    });
                    if !in_panel {
                        continue;
                    }
                    for y_index in 0..SAMPLES {
                        let y = base + (eave - base) * (y_index as f64 + 0.5) / SAMPLES as f64;
                        if y <= floor + layout.bottom_inset || y >= eave - layout.top_inset {
                            continue;
                        }
                        let phase = (y / profile.floor_m).rem_euclid(1.0);
                        let opening =
                            f64::from(phase > profile.glazing.0 && phase < profile.glazing.1);
                        sampled_glass += if system.residential() {
                            opening
                        } else {
                            0.72 + 0.28 * opening
                        };
                    }
                }
                sampled_glass /= (SAMPLES * SAMPLES) as f64;
                let actual = system.coarse_tint(wall, 24.0, [3.2, 19.7], base, floor, eave);
                for channel in 0..3 {
                    let expected = linear(wall[channel]) * (1.0 - sampled_glass)
                        + linear(profile.glass[channel]) * sampled_glass;
                    // Quantization and numerical area sampling together stay
                    // below one byte in the encoded pigment.
                    let encoded = if expected <= 0.0031308 {
                        expected * 12.92
                    } else {
                        1.055 * expected.powf(1.0 / 2.4) - 0.055
                    } * 255.0;
                    assert!(
                        (f64::from(actual[channel]) - encoded).abs() < 1.0,
                        "{system:?} channel {channel}"
                    );
                }
            }
        }
    }

    #[test]
    fn geometry_reads_the_resolved_system_without_reclassifying_source_use() {
        use crate::regional_architecture::ConstructionSystem;
        use vearth_data::vectors::BuildingUse;
        for system in System::ALL {
            for use_kind in [
                BuildingUse::Unknown,
                BuildingUse::Commercial,
                BuildingUse::Residential,
            ] {
                let d = BuildingDesign {
                    use_kind,
                    construction: ConstructionSystem::Glazed,
                    facade: Some(system),
                    ..Default::default()
                };
                for seed in 0..100 {
                    assert_eq!(resolve(d, 180.0, seed), Some(system));
                    assert_eq!(resolve(d, 20.0, seed), None);
                    assert_eq!(resolve(d, f32::NAN, seed), None);
                }
                assert_eq!(
                    system.facade_material(true) - system.facade_material(false),
                    8
                );
            }
        }
        assert_eq!(resolve(BuildingDesign::default(), 180.0, 0), None);
    }
}
