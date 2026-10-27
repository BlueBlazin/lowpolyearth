//! Generic urban observation/communications tower grammar.
//!
//! This is intentionally a typology rather than a landmark copy. Production
//! only invokes it after a mapped viewpoint/attraction has been joined to a
//! tall, compact source structure; curated landmark assets remain authoritative.

#[derive(Clone, Copy, Debug)]
pub struct ObservationTowerProfile {
    pub height_m: f32,
    pub pod_radius_m: f32,
    pub shaft_base_radius_m: f32,
    pub shaft_top_radius_m: f32,
}

pub fn observation_tower_profile(height_m: f32) -> ObservationTowerProfile {
    let height_m = height_m.clamp(45.0, 420.0);
    let pod_radius_m = (height_m * 0.040).clamp(6.5, 14.0);
    ObservationTowerProfile {
        height_m,
        pod_radius_m,
        shaft_base_radius_m: (pod_radius_m * 0.64).clamp(4.2, 7.5),
        shaft_top_radius_m: (pod_radius_m * 0.31).clamp(2.2, 4.2),
    }
}

pub fn observation_tower_envelope_radius(height_m: f32) -> f32 {
    observation_tower_profile(height_m).pod_radius_m + 1.4
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_keeps_a_slender_shaft_and_readable_pod() {
        let profile = observation_tower_profile(217.0);
        assert!(profile.shaft_base_radius_m < profile.pod_radius_m);
        assert!(profile.shaft_top_radius_m < profile.shaft_base_radius_m);
        assert!(observation_tower_envelope_radius(217.0) < 12.0);
    }
}
