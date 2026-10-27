//! Shared structural detail for regional architectural fallbacks.
//!
//! These rules fit existing walls, openings and roofs. They do not establish
//! building footprints, storeys, historic identities or terrain elevations.

/// Metric components fitted to the source envelope. These are construction
/// details, not permission to replace a mapped building with a catalogue model.
#[derive(Clone, Copy, Debug)]
pub struct ProductionFacadeProfile {
    pub reveal_m: f64,
    pub frame_m: f64,
    pub sill_m: f64,
    pub shade_m: f64,
    pub cornice_m: f64,
    pub eave_m: f64,
    pub coping_m: f64,
    pub fascia_m: f64,
}

pub fn production_detail_profile(
    design: &crate::regional_architecture::BuildingDesign,
) -> ProductionFacadeProfile {
    use crate::regional_architecture::DetailFamily::*;
    let (reveal_m, frame_m, sill_m, shade_m, cornice_m, eave_m, coping_m, fascia_m) =
        match design.detail {
            Plain => (0.10, 0.055, 0.14, 0.0, 0.12, 0.24, 0.12, 0.12),
            Classical => (0.18, 0.10, 0.24, 0.0, 0.30, 0.26, 0.17, 0.17),
            Timber => (0.13, 0.09, 0.19, 0.0, 0.16, 0.42, 0.12, 0.18),
            Shaded => (0.21, 0.065, 0.23, 0.55, 0.19, 0.45, 0.15, 0.17),
            Balcony => (0.17, 0.065, 0.21, 0.0, 0.22, 0.30, 0.18, 0.16),
            Modern => (0.07, 0.04, 0.11, 0.0, 0.11, 0.23, 0.12, 0.10),
            Industrial => (0.08, 0.045, 0.13, 0.22, 0.10, 0.28, 0.11, 0.11),
        };
    ProductionFacadeProfile {
        reveal_m,
        frame_m,
        sill_m,
        shade_m,
        cornice_m,
        eave_m,
        coping_m,
        fascia_m,
    }
}

/// Roof pitch for an inferred covering; measured roof geometry bypasses this
/// prior and reported height still caps every roof surface.
pub fn production_hip_pitch(design: &crate::regional_architecture::BuildingDesign) -> f32 {
    use crate::regional_architecture::{ConstructionSystem, DetailFamily};
    if design.expression >= 96
        && design.style == crate::architecture_styles::ArchitectureStyle::ShirakawaOgimachi
    {
        return 1.35;
    }
    if design.construction == ConstructionSystem::Earth {
        return 0.32;
    }
    match design.detail {
        DetailFamily::Timber => 0.65,
        DetailFamily::Shaded => 0.50,
        DetailFamily::Modern | DetailFamily::Industrial => 0.40,
        DetailFamily::Plain => 0.45,
        DetailFamily::Classical | DetailFamily::Balcony => 0.55,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_construction_components_fit_a_shallow_source_envelope() {
        use crate::regional_architecture::{BuildingDesign, DetailFamily};
        for family in [
            DetailFamily::Plain,
            DetailFamily::Classical,
            DetailFamily::Timber,
            DetailFamily::Shaded,
            DetailFamily::Balcony,
            DetailFamily::Modern,
            DetailFamily::Industrial,
        ] {
            let profile = production_detail_profile(&BuildingDesign {
                detail: family,
                ..Default::default()
            });
            assert!((0.04..=0.25).contains(&profile.reveal_m));
            assert!(profile.sill_m >= profile.reveal_m);
            assert!(profile.eave_m <= 0.5 && profile.shade_m <= 0.6);
        }
        let modern = production_detail_profile(&BuildingDesign {
            detail: DetailFamily::Modern,
            ..Default::default()
        });
        let shaded = production_detail_profile(&BuildingDesign {
            detail: DetailFamily::Shaded,
            ..Default::default()
        });
        assert!(shaded.reveal_m > modern.reveal_m * 2.0);
        assert!(shaded.shade_m > shaded.reveal_m);
    }
}
