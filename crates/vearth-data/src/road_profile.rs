//! Physical defaults for the transportation subtypes retained by Protomaps.
//! Widths and grading are authored metres and strengths, not surveyed values.

use crate::vectors::RoadClass;
use unicode_normalization::UnicodeNormalization;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum RoadDetail {
    #[default]
    Generic,
    Ramp,
    LivingStreet,
    Pedestrian,
    Footway,
    Sidewalk,
    Crossing,
    Cycleway,
    Track,
    Bridleway,
    Alley,
    Driveway,
    ParkingAisle,
    Steps,
    Pier,
    Corridor,
}

impl RoadDetail {
    pub fn from_protomaps(detail: Option<&str>, is_link: bool) -> Self {
        if is_link
            || matches!(
                detail,
                Some(
                    "motorway_link"
                        | "trunk_link"
                        | "primary_link"
                        | "secondary_link"
                        | "tertiary_link"
                )
            )
        {
            return Self::Ramp;
        }
        match detail {
            Some("living_street") => Self::LivingStreet,
            Some("pedestrian") => Self::Pedestrian,
            Some("footway") => Self::Footway,
            Some("sidewalk") => Self::Sidewalk,
            Some("crossing") => Self::Crossing,
            Some("cycleway") => Self::Cycleway,
            Some("track") => Self::Track,
            Some("bridleway") => Self::Bridleway,
            Some("alley") => Self::Alley,
            Some("driveway") => Self::Driveway,
            Some("parking_aisle") => Self::ParkingAisle,
            Some("steps") => Self::Steps,
            Some("pier") => Self::Pier,
            Some("corridor") => Self::Corridor,
            _ => Self::Generic,
        }
    }

    pub fn width_m(self, class: RoadClass) -> f32 {
        if class == RoadClass::Rail {
            return class.width_m();
        }
        match self {
            Self::Ramp => match class {
                RoadClass::Highway | RoadClass::Major => 6.0,
                _ => 4.5,
            },
            Self::LivingStreet => 5.0,
            Self::Pedestrian => 4.0,
            Self::Footway | Self::Sidewalk | Self::Steps | Self::Corridor => 1.8,
            Self::Crossing => 2.0,
            Self::Cycleway => 2.8,
            Self::Track => 3.2,
            Self::Bridleway => 2.2,
            Self::Alley => 3.2,
            Self::Driveway => 3.0,
            Self::ParkingAisle => 4.0,
            Self::Pier => 2.4,
            Self::Generic => class.width_m(),
        }
    }

    /// Along-way smoothing distance, strength, and shoulder width in metres.
    pub fn grading(self, class: RoadClass) -> (f64, f32, f64) {
        match self {
            Self::Footway | Self::Bridleway => (8.0, 0.22, 0.6),
            Self::Track => (14.0, 0.45, 1.0),
            Self::Sidewalk | Self::Crossing | Self::Steps | Self::Corridor => (10.0, 0.4, 0.6),
            Self::Cycleway => (18.0, 0.65, 0.8),
            Self::Generic if class == RoadClass::Path => (8.0, 0.22, 0.6),
            _ => (45.0, 1.0, f64::from(self.width_m(class)) * 0.4),
        }
    }
}

/// Route style identity and location are retained before projection into render cells.
/// The location describes the supplied source fragment, not an invented full way.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RoadProfile {
    pub detail: RoadDetail,
    /// Normalized route-name hash, or zero for unnamed routes. Protomaps MVT ids
    /// can identify merged collections of lines and are not whole-way identities.
    pub seed: u64,
    pub location_rad: Option<[f64; 2]>,
}

/// Stable style identity across tile clipping, feature merging and source zooms.
/// Whitespace, Unicode case and canonical accents are normalized; spelling and
/// language are retained so distinct named routes do not collapse to one identity.
pub fn name_seed(name: Option<&str>) -> u64 {
    let Some(name) = name.filter(|name| !name.trim().is_empty()) else {
        return 0;
    };
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for (index, word) in name.split_whitespace().enumerate() {
        if index > 0 {
            hash = (hash ^ u64::from(b' ')).wrapping_mul(0x100_0000_01b3);
        }
        for ch in word.chars().flat_map(char::to_lowercase).nfc() {
            let mut bytes = [0; 4];
            for byte in ch.encode_utf8(&mut bytes).bytes() {
                hash = (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3);
            }
        }
    }
    hash.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_names_ignore_layout_and_case_but_preserve_spelling() {
        assert_eq!(name_seed(None), 0);
        assert_eq!(name_seed(Some(" \t ")), 0);
        assert_eq!(
            name_seed(Some("  RUE\tDE LA PAIX ")),
            name_seed(Some("rue de la paix"))
        );
        assert_eq!(
            name_seed(Some("Äußere Straße")),
            name_seed(Some("ÄUẞERE STRAẞE"))
        );
        assert_ne!(name_seed(Some("North Road")), name_seed(Some("South Road")));
        assert_eq!(
            name_seed(Some("Rua do Comércio")),
            name_seed(Some("RUA DO COME\u{301}RCIO"))
        );
    }

    #[test]
    fn functional_widths_keep_ramps_and_walking_routes_subordinate() {
        assert_eq!(
            RoadDetail::from_protomaps(Some("motorway_link"), false),
            RoadDetail::Ramp
        );
        assert_eq!(
            RoadDetail::from_protomaps(Some("primary"), true),
            RoadDetail::Ramp
        );
        assert!(RoadDetail::Ramp.width_m(RoadClass::Highway) < RoadClass::Highway.width_m() / 2.0);
        assert!(
            RoadDetail::Footway.width_m(RoadClass::Path)
                < RoadDetail::Cycleway.width_m(RoadClass::Path)
        );
        assert!(
            RoadDetail::Cycleway.width_m(RoadClass::Path)
                < RoadDetail::Pedestrian.width_m(RoadClass::Path)
        );
        assert_eq!(
            RoadDetail::from_protomaps(Some("future"), false),
            RoadDetail::Generic
        );
        assert_eq!(
            RoadDetail::Ramp.width_m(RoadClass::Rail),
            RoadClass::Rail.width_m()
        );
    }
}
