//! Local architectural expression around documented places.
//!
//! Source points identify places. Circular and corridor extents are authored;
//! pinned polygons retain documented boundaries with an authored inward fade.
//! Geographic membership is not evidence about an individual building. Callers retain source use, footprint, height and measured roof facts;
//! facade finishes are inferred because current source metadata omits them.

const EARTH_RADIUS_M: f64 = 6_371_008.8;
use crate::architecture_styles::ArchitectureStyle;

/// Geographic specificity of an authored recipe, independent of its source ID.
/// A city recipe is an ordinary-stock fallback for narrower district recipes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaceScope {
    District,
    City,
}

/// An immutable place identity with an explicitly authored influence envelope.
/// Its focus may use a documented historic-center coordinate instead of the
/// search point; both coordinates remain in the source catalog provenance.
#[derive(Clone, Copy, Debug)]
pub struct ArchitecturalPlace {
    pub source_id: &'static str,
    pub source_name: &'static str,
    pub country_name: &'static str,
    pub style_key: &'static str,
    pub style: ArchitectureStyle,
    pub scope_tier: PlaceScope,
    /// Separate authored areas share one identity and recipe. Their influence
    /// is the maximum, so the space between quarters is not filled in.
    pub areas: &'static [InfluenceArea],
    pub palette_confidence: f32,
    pub character_confidence: f32,
    pub contemporary_infill: f32,
    pub max_height_m: f32,
    pub max_footprint_m2: f32,
}

/// A circular/corridor art envelope or a pinned polygon with authored inward fade.
#[derive(Clone, Copy, Debug)]
pub enum InfluenceArea {
    Circle {
        latitude_deg: f64,
        longitude_deg: f64,
        core_radius_m: f32,
        outer_radius_m: f32,
    },
    /// A connected street line with round endcaps. Points are geodetic degrees;
    /// each consecutive pair follows the shorter great-circle arc.
    Corridor {
        points: &'static [(f64, f64)],
        core_half_width_m: f32,
        outer_half_width_m: f32,
    },
    /// Pinned local boundary geometry. Holes and disconnected components are
    /// preserved; fade is inward only and never admits a point outside.
    Polygon {
        polygons: &'static [PolygonArea],
        boundary_falloff_m: f32,
    },
}

/// A local polygon compiled into a gnomonic plane. Great-circle edges are
/// straight in this plane. The compiler bounds every component to a 20 km cap.
#[derive(Clone, Copy, Debug)]
pub struct PolygonArea {
    center: [f64; 3],
    east: [f64; 3],
    north: [f64; 3],
    cap_cos: f64,
    representative_center: (f64, f64),
    /// Closed metre-coordinate rings: exterior first, then its holes.
    rings: &'static [&'static [(f64, f64)]],
    ring_indices: &'static [RingIndex],
}

// Immutable edge buckets are compiled from the exact rings. Membership needs
// one row; boundary fading needs the rows intersecting its distance interval.
#[derive(Clone, Copy, Debug)]
struct RingIndex {
    min_y: f64,
    max_y: f64,
    rows: &'static [&'static [u32]],
}

impl RingIndex {
    fn row(self, y: f64) -> usize {
        (((y - self.min_y) / (self.max_y - self.min_y) * self.rows.len() as f64).floor() as usize)
            .min(self.rows.len() - 1)
    }

    fn query(self, point: (f64, f64), ring: &[(f64, f64)], limit_squared: f64) -> (bool, f64) {
        let (x, y) = point;
        let inside = y >= self.min_y
            && y <= self.max_y
            && self.rows[self.row(y)].iter().fold(false, |inside, &edge| {
                let ((ax, ay), (bx, by)) = (ring[edge as usize], ring[edge as usize + 1]);
                inside ^ ((ay > y) != (by > y) && x < ax + (y - ay) * (bx - ax) / (by - ay))
            });
        let reach = limit_squared.sqrt();
        let mut distance = limit_squared;
        if y + reach >= self.min_y && y - reach <= self.max_y {
            for row in &self.rows[self.row(y - reach)..=self.row(y + reach)] {
                for &edge in *row {
                    distance = distance.min(edge_distance_squared(
                        point,
                        ring[edge as usize],
                        ring[edge as usize + 1],
                    ));
                }
            }
        }
        (inside, distance)
    }
}

/// A geographic suggestion; scale and source compatibility are caller policy.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlaceInfluence {
    pub source_id: &'static str,
    pub source_name: &'static str,
    pub country_name: &'static str,
    pub style_key: &'static str,
    pub style: ArchitectureStyle,
    pub scope_tier: PlaceScope,
    /// Smooth geographic weight, independent of confidence and building use.
    pub strength: f32,
    pub palette_confidence: f32,
    pub character_confidence: f32,
    pub contemporary_infill: f32,
    pub max_height_m: f32,
    pub max_footprint_m2: f32,
}

include!("../assets/architecture/places-generated.rs");

/// Complete immutable catalog, including source identities and authored extents.
pub fn places() -> &'static [ArchitecturalPlace] {
    PLACES
}

/// Most specific available influence at geodetic latitude/longitude in radians.
pub fn influence_at(lat_rad: f64, lon_rad: f64) -> Option<PlaceInfluence> {
    influences_at(lat_rad, lon_rad).into_iter().flatten().next()
}

/// Bounded inspection view, not the production selection path.
/// Two district candidates followed by one city fallback, with empty slots omitted
/// by callers. Districts always precede cities, even at a weaker falloff strength.
///
/// Each tier ranks by geographic strength, then stable source ID and style key.
/// City capacity is reserved so overlapping districts cannot discard the fallback.
/// More than two overlapping districts or one overlapping city are truncated;
/// recipe compatibility and probabilistic admission remain caller policy. Invalid
/// coordinates return no influence; finite longitudes wrap the antimeridian.
/// Queries allocate nothing and inspect only a sparse two-degree spatial bucket.
pub fn influences_at(lat_rad: f64, lon_rad: f64) -> [Option<PlaceInfluence>; 3] {
    let mut result = [None, None, None];
    let Some((lon_rad, candidates)) = candidates_at(lat_rad, lon_rad) else {
        return result;
    };
    for &place_id in candidates {
        if let Some(influence) = PLACES[usize::from(place_id)].influence(lat_rad, lon_rad) {
            insert(&mut result, influence);
        }
    }
    result
}

fn candidates_at(lat_rad: f64, lon_rad: f64) -> Option<(f64, &'static [u16])> {
    if !lat_rad.is_finite() || !lon_rad.is_finite() || lat_rad.abs() > std::f64::consts::FRAC_PI_2 {
        return None;
    }
    let lon_rad =
        (lon_rad + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI;
    let row = (((lat_rad.to_degrees() + 90.0) / 2.0).floor() as u16).min(89);
    let col = (((lon_rad.to_degrees() + 180.0) / 2.0).floor() as u16).min(179);
    let cell = row * 180 + col;
    let Ok(bucket) = BUCKETS.binary_search_by_key(&cell, |&(key, _, _)| key) else {
        return None;
    };
    let (_, start, count) = BUCKETS[bucket];
    Some((
        lon_rad,
        &CANDIDATES[start as usize..(start + u32::from(count)) as usize],
    ))
}

// The bounded inspection list is not a selection API: an incompatible or
// rejected candidate must never evict a compatible lower-ranked provider.
// Check use/scale before traversing boundary edges and retain only the winner.
pub(crate) fn select_at<T>(
    lat_rad: f64,
    lon_rad: f64,
    eligible: impl FnMut(&ArchitecturalPlace) -> Option<T>,
    admitted: impl Fn(PlaceInfluence) -> bool,
) -> Option<(PlaceInfluence, T)> {
    let (lon_rad, candidates) = candidates_at(lat_rad, lon_rad)?;
    select_from(
        candidates.iter().map(|&id| &PLACES[usize::from(id)]),
        lat_rad,
        lon_rad,
        eligible,
        admitted,
    )
}

fn select_from<'a, T>(
    candidates: impl Iterator<Item = &'a ArchitecturalPlace>,
    lat_rad: f64,
    lon_rad: f64,
    mut eligible: impl FnMut(&ArchitecturalPlace) -> Option<T>,
    admitted: impl Fn(PlaceInfluence) -> bool,
) -> Option<(PlaceInfluence, T)> {
    let mut selected: Option<(PlaceInfluence, T)> = None;
    for place in candidates {
        let Some(recipe) = eligible(place) else {
            continue;
        };
        let Some(influence) = place.influence(lat_rad, lon_rad) else {
            continue;
        };
        if admitted(influence)
            && selected
                .as_ref()
                .is_none_or(|(other, _)| precedes(influence, *other))
        {
            selected = Some((influence, recipe));
        }
    }
    selected
}

fn precedes(candidate: PlaceInfluence, other: PlaceInfluence) -> bool {
    if candidate.scope_tier != other.scope_tier {
        return candidate.scope_tier == PlaceScope::District;
    }
    candidate.strength > other.strength
        || (candidate.strength == other.strength
            && (candidate.source_id, candidate.style_key) < (other.source_id, other.style_key))
}

fn insert(result: &mut [Option<PlaceInfluence>; 3], candidate: PlaceInfluence) {
    let slots = match candidate.scope_tier {
        PlaceScope::District => &mut result[..2],
        PlaceScope::City => &mut result[2..],
    };
    let before =
        |other: Option<PlaceInfluence>| other.is_none_or(|other| precedes(candidate, other));
    if let Some(index) = slots.iter().position(|&other| before(other)) {
        for i in (index + 1..slots.len()).rev() {
            slots[i] = slots[i - 1];
        }
        slots[index] = Some(candidate);
    }
}

fn unit_point(latitude_deg: f64, longitude_deg: f64) -> [f64; 3] {
    let (lat, lon) = (latitude_deg.to_radians(), longitude_deg.to_radians());
    [lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin()]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn angular_distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    let c = cross(a, b);
    dot(c, c).sqrt().atan2(dot(a, b))
}

fn segment_distance(q: [f64; 3], a: [f64; 3], b: [f64; 3]) -> f64 {
    let normal = cross(a, b);
    let length = dot(normal, normal).sqrt();
    // The compiler rejects coincident and antipodal control points. Retain a
    // finite endpoint fallback for a manually constructed degenerate segment.
    if length < 1e-12 {
        return angular_distance(q, a).min(angular_distance(q, b));
    }
    let normal = normal.map(|value| value / length);
    let tangent = cross(normal, a);
    let along = dot(q, tangent).atan2(dot(q, a));
    if along >= 0.0 && along <= length.atan2(dot(a, b)) {
        dot(q, normal).abs().clamp(0.0, 1.0).asin()
    } else {
        angular_distance(q, a).min(angular_distance(q, b))
    }
}

fn ring_contains_and_distance(
    point: (f64, f64),
    ring: &[(f64, f64)],
    limit_squared: f64,
) -> (bool, f64) {
    let (x, y) = point;
    let mut inside = false;
    // Distances beyond the inward fade all produce strength one.
    let mut distance_squared = limit_squared;
    for pair in ring.windows(2) {
        let ((ax, ay), (bx, by)) = (pair[0], pair[1]);
        if (ay > y) != (by > y) && x < ax + (y - ay) * (bx - ax) / (by - ay) {
            inside = !inside;
        }
        distance_squared = distance_squared.min(edge_distance_squared(point, pair[0], pair[1]));
    }
    (inside, distance_squared)
}

fn edge_distance_squared((x, y): (f64, f64), (ax, ay): (f64, f64), (bx, by): (f64, f64)) -> f64 {
    let (dx, dy) = (bx - ax, by - ay);
    let length_squared = dx * dx + dy * dy;
    let t = if length_squared > 0.0 {
        (((x - ax) * dx + (y - ay) * dy) / length_squared).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (x - ax - t * dx).powi(2) + (y - ay - t * dy).powi(2)
}

impl PolygonArea {
    fn ring_query(self, point: (f64, f64), ring: usize, limit_squared: f64) -> (bool, f64) {
        if let Some(index) = self.ring_indices.get(ring) {
            index.query(point, self.rings[ring], limit_squared)
        } else {
            ring_contains_and_distance(point, self.rings[ring], limit_squared)
        }
    }

    fn strength(self, q: [f64; 3], falloff_m: f32) -> Option<f32> {
        let denominator = dot(q, self.center);
        // Most buildings in the broad spatial bucket are outside this cap.
        // Reject them before projection or any ring-edge iteration.
        if denominator < self.cap_cos {
            return None;
        }
        let point = (
            EARTH_RADIUS_M * dot(q, self.east) / denominator,
            EARTH_RADIUS_M * dot(q, self.north) / denominator,
        );
        self.rings.first()?;
        let (inside, mut distance_squared) =
            self.ring_query(point, 0, f64::from(falloff_m).powi(2));
        if !inside {
            return None;
        }
        for hole in 1..self.rings.len() {
            let (inside_hole, distance) = self.ring_query(point, hole, distance_squared);
            if inside_hole {
                return None;
            }
            distance_squared = distance_squared.min(distance);
        }
        // Boundary points are excluded regardless of ray-crossing orientation.
        if distance_squared <= 1e-14 {
            return None;
        }
        let t = (distance_squared.sqrt() / f64::from(falloff_m)).min(1.0);
        Some((t * t * (3.0 - 2.0 * t)) as f32)
    }
}

impl InfluenceArea {
    /// A point on the authored center or centerline, in geodetic degrees.
    /// Corridors use a middle-segment midpoint; polygons use a compiled interior
    /// point outside their holes. These are staging focuses, not source identities.
    pub fn representative_center(self) -> (f64, f64) {
        match self {
            Self::Circle {
                latitude_deg,
                longitude_deg,
                ..
            } => (latitude_deg, longitude_deg),
            Self::Polygon { polygons, .. } => polygons
                .first()
                .map_or((0.0, 0.0), |polygon| polygon.representative_center),
            Self::Corridor { points, .. } => {
                let index = points.len().saturating_sub(1) / 2;
                let Some(&(lat, lon)) = points.get(index) else {
                    return (0.0, 0.0);
                };
                let Some(&(end_lat, end_lon)) = points.get(index + 1) else {
                    return (lat, lon);
                };
                let a = unit_point(lat, lon);
                let b = unit_point(end_lat, end_lon);
                let sum = [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
                (
                    sum[2].atan2(sum[0].hypot(sum[1])).to_degrees(),
                    sum[1].atan2(sum[0]).to_degrees(),
                )
            }
        }
    }

    fn widths(self) -> (f32, f32) {
        match self {
            Self::Circle {
                core_radius_m,
                outer_radius_m,
                ..
            } => (core_radius_m, outer_radius_m),
            Self::Corridor {
                core_half_width_m,
                outer_half_width_m,
                ..
            } => (core_half_width_m, outer_half_width_m),
            Self::Polygon {
                boundary_falloff_m, ..
            } => (0.0, boundary_falloff_m),
        }
    }

    fn strength(self, lat_rad: f64, lon_rad: f64) -> Option<f32> {
        let distance_m = match self {
            Self::Polygon {
                polygons,
                boundary_falloff_m,
            } => {
                let q = [
                    lat_rad.cos() * lon_rad.cos(),
                    lat_rad.cos() * lon_rad.sin(),
                    lat_rad.sin(),
                ];
                return polygons
                    .iter()
                    .filter_map(|polygon| polygon.strength(q, boundary_falloff_m))
                    .reduce(f32::max);
            }
            Self::Circle {
                latitude_deg,
                longitude_deg,
                ..
            } => {
                let place_lat = latitude_deg.to_radians();
                let sin_lat = ((lat_rad - place_lat) * 0.5).sin();
                let sin_lon = ((lon_rad - longitude_deg.to_radians()) * 0.5).sin();
                let a = sin_lat * sin_lat + lat_rad.cos() * place_lat.cos() * sin_lon * sin_lon;
                2.0 * EARTH_RADIUS_M * a.clamp(0.0, 1.0).sqrt().asin()
            }
            Self::Corridor { points, .. } => {
                let q = unit_point(lat_rad.to_degrees(), lon_rad.to_degrees());
                points
                    .windows(2)
                    .map(|pair| {
                        segment_distance(
                            q,
                            unit_point(pair[0].0, pair[0].1),
                            unit_point(pair[1].0, pair[1].1),
                        )
                    })
                    .reduce(f64::min)?
                    * EARTH_RADIUS_M
            }
        };
        let (core, outer) = self.widths();
        if distance_m >= f64::from(outer) {
            return None;
        }
        let t = ((distance_m - f64::from(core)) / f64::from(outer - core)).clamp(0.0, 1.0);
        Some((1.0 - t * t * (3.0 - 2.0 * t)) as f32)
    }
}

impl ArchitecturalPlace {
    fn influence(self, lat_rad: f64, lon_rad: f64) -> Option<PlaceInfluence> {
        let strength = self
            .areas
            .iter()
            .filter_map(|area| area.strength(lat_rad, lon_rad))
            .reduce(f32::max)?;
        Some(PlaceInfluence {
            source_id: self.source_id,
            source_name: self.source_name,
            country_name: self.country_name,
            style_key: self.style_key,
            style: self.style,
            scope_tier: self.scope_tier,
            strength,
            palette_confidence: self.palette_confidence,
            character_confidence: self.character_confidence,
            contemporary_infill: self.contemporary_infill,
            max_height_m: self.max_height_m,
            max_footprint_m2: self.max_footprint_m2,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::architecture_styles::ArchitectureStyle;
    use std::collections::BTreeSet;

    #[test]
    fn selection_does_not_truncate_before_compatibility_or_admission() {
        let mut places = [PLACES[0]; 6];
        for (index, place) in places.iter_mut().enumerate() {
            place.source_id = ["a", "b", "c", "d", "e", "f"][index];
            place.scope_tier = if index < 3 {
                PlaceScope::District
            } else {
                PlaceScope::City
            };
            place.areas = &[InfluenceArea::Circle {
                latitude_deg: 0.0,
                longitude_deg: 0.0,
                core_radius_m: 100.0,
                outer_radius_m: 200.0,
            }];
        }
        for reverse in [false, true] {
            if reverse {
                places.reverse();
            }
            let select = |districts: bool| {
                select_from(
                    places.iter(),
                    0.0,
                    0.0,
                    |p| {
                        (p.source_id != "b" && (districts || p.scope_tier == PlaceScope::City))
                            .then_some(p.source_id)
                    },
                    |p| !matches!(p.source_id, "a" | "d"),
                )
                .unwrap()
            };
            assert_eq!(select(true).1, "c");
            assert_eq!(select(false).1, "e");
        }
    }

    #[test]
    fn bounded_boundary_distance_matches_exhaustive_edges() {
        for place in PLACES {
            for area in place.areas {
                let InfluenceArea::Polygon {
                    polygons,
                    boundary_falloff_m,
                } = area
                else {
                    continue;
                };
                for polygon in *polygons {
                    for (ring_id, ring) in polygon.rings.iter().enumerate() {
                        // Probe either side of real vertices, including holes,
                        // with both saturated and partially faded distances.
                        for &(x, y) in ring.iter().step_by((ring.len() / 20).max(1)) {
                            for offset in [-100.0, -0.01, 0.0, 0.01, 100.0] {
                                let point = (x + offset, y - offset);
                                let mut inside = false;
                                let mut distance = f64::INFINITY;
                                for edge in ring.windows(2) {
                                    let ((ax, ay), (bx, by)) = (edge[0], edge[1]);
                                    if (ay > point.1) != (by > point.1)
                                        && point.0 < ax + (point.1 - ay) * (bx - ax) / (by - ay)
                                    {
                                        inside = !inside;
                                    }
                                    let a = glam::DVec2::new(ax, ay);
                                    let d = glam::DVec2::new(bx, by) - a;
                                    let q = glam::DVec2::new(point.0, point.1) - a;
                                    let t = if d.length_squared() == 0.0 {
                                        0.0
                                    } else {
                                        (q.dot(d) / d.length_squared()).clamp(0.0, 1.0)
                                    };
                                    distance = distance.min((q - d * t).length_squared());
                                }
                                let limit = f64::from(*boundary_falloff_m).powi(2);
                                assert_eq!(polygon.ring_indices.len(), polygon.rings.len());
                                let actual = polygon.ring_query(point, ring_id, limit);
                                assert_eq!(actual.0, inside);
                                assert!((actual.1 - distance.min(limit)).abs() < 1e-6);
                            }
                        }
                    }
                }
            }
        }
    }

    fn exhaustive(lat: f64, lon: f64) -> [Option<PlaceInfluence>; 3] {
        let mut result = [None, None, None];
        for place in PLACES {
            if let Some(influence) = place.influence(lat, lon) {
                insert(&mut result, influence);
            }
        }
        result
    }

    #[test]
    fn catalog_is_broad_source_identified_and_uses_known_styles() {
        assert!(PLACES.len() >= 100);
        assert!(
            PLACES
                .iter()
                .map(|p| p.country_name)
                .collect::<BTreeSet<_>>()
                .len()
                >= 40
        );
        let mut identities = BTreeSet::new();
        for p in PLACES {
            assert!(identities.insert((p.source_id, p.style_key)));
            assert!(
                p.source_id.starts_with("geonames:")
                    || p.source_id.starts_with("wikidata:")
                    || p.source_id.starts_with("nyclpc:")
            );
            assert!(
                ArchitectureStyle::from_key(p.style_key).is_some(),
                "{}",
                p.style_key
            );
            assert_eq!(p.style.key(), p.style_key);
            assert!(!p.country_name.is_empty() && !p.source_name.is_empty());
            assert!(!p.areas.is_empty());
            for area in p.areas {
                let (core, outer) = area.widths();
                assert!((0.0..outer).contains(&core));
            }
            for value in [
                p.palette_confidence,
                p.character_confidence,
                p.contemporary_infill,
            ] {
                assert!((0.0..=1.0).contains(&value));
            }
            assert!(p.max_height_m > 0.0 && p.max_footprint_m2 > 0.0);
        }
    }

    #[test]
    fn copenhagen_city_preserves_frederiksberg_hole_and_nyhavn_priority() {
        let city = PLACES
            .iter()
            .find(|p| p.source_id == "geonames:2618425")
            .unwrap();
        for (lat, lon) in [(55.6919_f64, 12.5573_f64), (55.7069, 12.5995)] {
            assert!(city.influence(lat.to_radians(), lon.to_radians()).is_some());
        }
        for (lat, lon) in [(55.679_f64, 12.532_f64), (55.593, 12.672)] {
            assert!(city.influence(lat.to_radians(), lon.to_radians()).is_none());
        }
        let choices = influences_at(55.6798_f64.to_radians(), 12.5900_f64.to_radians());
        assert_eq!(choices[0].unwrap().source_id, "wikidata:Q943946");
        assert_eq!(choices[0].unwrap().scope_tier, PlaceScope::District);
        assert_eq!(choices[2].unwrap().source_id, city.source_id);
        assert_eq!(choices[2].unwrap().scope_tier, PlaceScope::City);
    }

    #[test]
    fn athens_city_prior_is_bounded_to_the_municipality() {
        let city = PLACES
            .iter()
            .find(|p| p.source_id == "geonames:264371")
            .unwrap();
        for (lat, lon) in [
            (38.0018_f64, 23.7375_f64),
            (37.9658, 23.7440),
            (37.9726, 23.7278),
        ] {
            assert!(city.influence(lat.to_radians(), lon.to_radians()).is_some());
        }
        assert!(
            city.influence(37.9420_f64.to_radians(), 23.647_f64.to_radians())
                .is_none()
        );
    }

    #[test]
    fn sparse_index_matches_exhaustive_queries_on_envelope_edges() {
        for p in PLACES {
            for area in p.areas {
                let mut centers = vec![area.representative_center()];
                if let InfluenceArea::Corridor { points, .. } = area {
                    centers.extend_from_slice(points);
                    for pair in points.windows(2) {
                        let a = unit_point(pair[0].0, pair[0].1);
                        let b = unit_point(pair[1].0, pair[1].1);
                        let sum = [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
                        centers.push((
                            sum[2].atan2(sum[0].hypot(sum[1])).to_degrees(),
                            sum[1].atan2(sum[0]).to_degrees(),
                        ));
                    }
                }
                if let InfluenceArea::Polygon { polygons, .. } = area {
                    for polygon in *polygons {
                        for ring in polygon.rings {
                            for &(x, y) in *ring {
                                let q = std::array::from_fn::<_, 3, _>(|i| {
                                    polygon.center[i]
                                        + polygon.east[i] * x / EARTH_RADIUS_M
                                        + polygon.north[i] * y / EARTH_RADIUS_M
                                });
                                centers.push((
                                    q[2].atan2(q[0].hypot(q[1])).to_degrees(),
                                    q[1].atan2(q[0]).to_degrees(),
                                ));
                            }
                        }
                    }
                }
                for (latitude, longitude) in centers {
                    let lat = latitude.to_radians();
                    let lon = longitude.to_radians();
                    for factor in [0.0, 0.5, 0.99999, 1.00001] {
                        let distance = f64::from(area.widths().1) / EARTH_RADIUS_M * factor;
                        for heading in 0..24 {
                            let heading = f64::from(heading) * std::f64::consts::TAU / 24.0;
                            let target_lat = (lat.sin() * distance.cos()
                                + lat.cos() * distance.sin() * heading.cos())
                            .asin();
                            let target_lon = lon
                                + (heading.sin() * distance.sin() * lat.cos())
                                    .atan2(distance.cos() - lat.sin() * target_lat.sin());
                            let indexed = influences_at(target_lat, target_lon);
                            let expected = exhaustive(target_lat, target_lon);
                            for (got, want) in indexed.into_iter().zip(expected) {
                                assert_eq!(
                                    got.map(|x| x.source_id),
                                    want.map(|x| x.source_id),
                                    "{}",
                                    p.source_id
                                );
                                if let (Some(got), Some(want)) = (got, want) {
                                    assert!((got.strength - want.strength).abs() < 0.00001);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn separate_trulli_quarters_do_not_fill_the_gap_or_the_search_point() {
        let place = PLACES
            .iter()
            .find(|p| p.source_id == "wikidata:Q51812")
            .unwrap();
        assert_eq!(place.areas.len(), 2);
        for area in place.areas {
            assert_eq!(
                place
                    .influence(
                        area.representative_center().0.to_radians(),
                        area.representative_center().1.to_radians()
                    )
                    .unwrap()
                    .strength,
                1.0
            );
        }
        // Between the conservative circles, and at the northern search point.
        for (lat, lon) in [
            (40.78214_f64, 17.23856_f64),
            (40.784061111111, 17.237461111111),
        ] {
            assert!(
                place
                    .influence(lat.to_radians(), lon.to_radians())
                    .is_none()
            );
        }
    }

    #[test]
    fn requested_places_have_distinct_geographically_grounded_character() {
        for (id, key) in [
            ("wikidata:Q3634368", "jaisalmer-haveli"),
            ("wikidata:Q1483099", "jodhpur-haveli"),
            ("wikidata:Q131081437", "jaipur-bazaar"),
            ("wikidata:Q3106527", "tunisian-medina"),
            ("wikidata:Q1928594", "bahia-sobrado"),
            ("geonames:3183130", "amalfi-terrace"),
        ] {
            let p = PLACES.iter().find(|p| p.source_id == id).expect(id);
            assert_eq!(p.style_key, key);
            let result = influences_at(
                p.areas[0].representative_center().0.to_radians(),
                p.areas[0].representative_center().1.to_radians(),
            );
            assert!(
                result
                    .into_iter()
                    .flatten()
                    .any(|r| r.style_key == key && r.strength == 1.0)
            );
        }
    }

    #[test]
    fn smooth_falloff_wrapping_and_invalid_coordinates() {
        let p = *PLACES
            .iter()
            .find(|p| p.areas.len() == 1 && matches!(p.areas[0], InfluenceArea::Circle { .. }))
            .unwrap();
        let lat = p.areas[0].representative_center().0.to_radians();
        let lon = p.areas[0].representative_center().1.to_radians();
        let mut previous = 1.0;
        for step in 0..100 {
            let distance = f64::from(p.areas[0].widths().1) * f64::from(step) / 100.0;
            let strength = p
                .influence(lat + distance / EARTH_RADIUS_M, lon)
                .unwrap()
                .strength;
            assert!(strength <= previous + 1e-6);
            previous = strength;
        }
        assert!(
            p.influence(
                lat + f64::from(p.areas[0].widths().1 + 1.0) / EARTH_RADIUS_M,
                lon
            )
            .is_none()
        );
        let wrapped = influences_at(lat, lon + std::f64::consts::TAU);
        assert_eq!(
            wrapped.map(|p| p.map(|p| p.source_id)),
            influences_at(lat, lon).map(|p| p.map(|p| p.source_id))
        );
        for (lat, lon) in [
            (f64::NAN, 0.0),
            (0.0, f64::INFINITY),
            (2.0, 0.0),
            (-2.0, 0.0),
        ] {
            assert_eq!(influences_at(lat, lon), [None, None, None]);
        }
        assert_eq!(influences_at(0.0, 0.0), [None, None, None]);
    }

    #[test]
    fn fading_districts_precede_city_and_cannot_discard_its_fallback() {
        let mut district = *PLACES
            .iter()
            .find(|p| p.style_key == "copenhagen-nyhavn")
            .unwrap();
        // Co-locate real catalog identities to isolate geographic precedence
        // from catalog coverage changes and the lexical order of source IDs.
        district.scope_tier = PlaceScope::District;
        district.areas = &[InfluenceArea::Circle {
            latitude_deg: 0.0,
            longitude_deg: 0.0,
            core_radius_m: 100.0,
            outer_radius_m: 1000.0,
        }];
        let mut other = *PLACES
            .iter()
            .find(|p| p.style_key == "stockholm-gamla-stan")
            .unwrap();
        other.scope_tier = PlaceScope::District;
        other.areas = district.areas;
        let mut city = *PLACES
            .iter()
            .find(|p| p.source_id.starts_with("geonames:"))
            .unwrap();
        city.scope_tier = PlaceScope::City;
        city.areas = &[InfluenceArea::Circle {
            latitude_deg: 0.0,
            longitude_deg: 0.0,
            core_radius_m: 2000.0,
            outer_radius_m: 3000.0,
        }];
        let lat = 750.0 / EARTH_RADIUS_M;
        let d = district.influence(lat, 0.0).unwrap();
        let o = other.influence(lat, 0.0).unwrap();
        let c = city.influence(lat, 0.0).unwrap();
        assert!(d.strength > 0.0 && d.strength < c.strength);
        assert!(c.source_id < d.source_id);
        let mut expected = [None, None, None];
        for candidate in [d, o, c] {
            insert(&mut expected, candidate);
        }
        for candidates in [[c, d, o], [o, c, d], [o, d, c]] {
            let mut result = [None, None, None];
            for candidate in candidates {
                insert(&mut result, candidate);
            }
            assert_eq!(result, expected);
        }
        assert_eq!(expected[0].unwrap().scope_tier, PlaceScope::District);
        assert_eq!(expected[1].unwrap().scope_tier, PlaceScope::District);
        assert_eq!(expected[2], Some(c));
        let mut equal_district = d;
        equal_district.strength = c.strength;
        let mut tied = [None, None, None];
        insert(&mut tied, c);
        insert(&mut tied, equal_district);
        assert_eq!(tied[0], Some(equal_district));
        assert_eq!(tied[2], Some(c));
        // The resolver can reject both faded district admissions and still
        // encounter the city, whose independent geographic plateau survives.
        let admission = 0.5;
        assert!(d.strength < admission && o.strength < admission);
        assert_eq!(
            expected
                .into_iter()
                .flatten()
                .find(|p| p.strength > admission),
            Some(c)
        );
        assert!(district.influence(1100.0 / EARTH_RADIUS_M, 0.0).is_none());
        assert!(city.influence(1100.0 / EARTH_RADIUS_M, 0.0).is_some());
    }

    #[test]
    fn candidate_capacity_is_reserved_per_scope_and_city_only_is_reachable() {
        let mut candidate = PLACES[0]
            .influence(
                PLACES[0].areas[0].representative_center().0.to_radians(),
                PLACES[0].areas[0].representative_center().1.to_radians(),
            )
            .unwrap();
        candidate.scope_tier = PlaceScope::City;
        let mut result = [None, None, None];
        insert(&mut result, candidate);
        assert_eq!(result.into_iter().flatten().next(), Some(candidate));
        let city = candidate;
        candidate.scope_tier = PlaceScope::District;
        for (id, strength) in [("third", 0.1), ("second", 0.2), ("first", 0.3)] {
            candidate.source_id = id;
            candidate.strength = strength;
            insert(&mut result, candidate);
        }
        assert_eq!(
            result.map(|p| p.map(|p| p.source_id)),
            [Some("first"), Some("second"), Some(city.source_id)]
        );
        let mut weaker_city = city;
        weaker_city.strength = 0.5;
        insert(&mut result, weaker_city);
        assert_eq!(result[2], Some(city));
    }

    #[test]
    fn corridors_follow_finite_segments_with_round_endcaps_and_bends() {
        let area = InfluenceArea::Corridor {
            points: &[(0.0, 0.0), (0.0, 0.02), (0.02, 0.02)],
            core_half_width_m: 50.0,
            outer_half_width_m: 100.0,
        };
        let sample = |lat: f64, lon: f64| area.strength(lat.to_radians(), lon.to_radians());
        assert_eq!(sample(0.0, 0.01), Some(1.0));
        assert_eq!(sample(0.01, 0.02), Some(1.0));
        assert_eq!(sample(0.0, 0.02), Some(1.0));
        assert!(sample(0.01, 0.01).is_none()); // The bend does not fill the triangle.
        let offset = |meters: f64| (meters / EARTH_RADIUS_M).to_degrees();
        for (lat, lon) in [
            (0.0, -offset(75.0)),
            (offset(75.0), 0.01),
            (0.02 + offset(75.0), 0.02),
        ] {
            assert!((sample(lat, lon).unwrap() - 0.5).abs() < 1e-5);
        }
        assert!(sample(0.0, -offset(101.0)).is_none());
        assert!(sample(0.02 + offset(101.0), 0.02).is_none());
        assert!(sample(offset(101.0), 0.01).is_none());
        let center = area.representative_center();
        assert_eq!(sample(center.0, center.1), Some(1.0));
    }

    #[test]
    fn pimlico_council_polygon_excludes_churchill_gardens_independent_of_height() {
        let place = PLACES
            .iter()
            .find(|p| p.style_key == "london-pimlico")
            .unwrap();
        let InfluenceArea::Polygon { polygons, .. } = place.areas[0] else {
            panic!("Pimlico must use its pinned council boundary");
        };
        assert_eq!(polygons.len(), 2);
        assert_eq!(
            polygons
                .iter()
                .flat_map(|p| p.rings)
                .map(|ring| ring.len())
                .sum::<usize>(),
            197
        );
        assert!(
            place
                .influence(51.4887_f64.to_radians(), (-0.1395_f64).to_radians())
                .is_some()
        );
        for (lat, lon) in [
            (51.48683588655_f64, -0.1449438002_f64), // Gilbert House.
            (51.4857_f64, -0.1445_f64),              // Low housing inside Churchill Gardens.
        ] {
            assert!(
                place
                    .influence(lat.to_radians(), lon.to_radians())
                    .is_none()
            );
            assert!(
                !influences_at(lat.to_radians(), lon.to_radians())
                    .into_iter()
                    .flatten()
                    .any(|p| p.style_key == place.style_key)
            );
        }
    }

    const SQUARE_WITH_HOLE: PolygonArea = PolygonArea {
        center: [1.0, 0.0, 0.0],
        east: [0.0, 1.0, 0.0],
        north: [0.0, 0.0, 1.0],
        cap_cos: 0.99999998,
        representative_center: (0.0, 0.0005),
        ring_indices: &[],
        rings: &[
            &[
                (-100.0, -100.0),
                (100.0, -100.0),
                (100.0, 100.0),
                (-100.0, 100.0),
                (-100.0, -100.0),
            ],
            &[
                (-20.0, -20.0),
                (-20.0, 20.0),
                (20.0, 20.0),
                (20.0, -20.0),
                (-20.0, -20.0),
            ],
        ],
    };

    #[test]
    fn polygon_excludes_holes_outside_and_boundaries_with_only_inward_falloff() {
        let sample = |x: f64, y: f64| {
            let norm = (1.0 + (x / EARTH_RADIUS_M).powi(2) + (y / EARTH_RADIUS_M).powi(2)).sqrt();
            SQUARE_WITH_HOLE.strength(
                [
                    1.0 / norm,
                    x / EARTH_RADIUS_M / norm,
                    y / EARTH_RADIUS_M / norm,
                ],
                40.0,
            )
        };
        assert_eq!(sample(60.0, 0.0), Some(1.0));
        assert!((sample(90.0, 0.0).unwrap() - 0.15625).abs() < 1e-6);
        assert!((sample(30.0, 0.0).unwrap() - 0.15625).abs() < 1e-6);
        for (x, y) in [
            (0.0, 0.0),
            (20.0, 0.0),
            (100.0, 0.0),
            (100.0, 100.0),
            (101.0, 0.0),
            (0.0, 1000.0),
        ] {
            assert!(sample(x, y).is_none(), "({x},{y})");
        }
    }

    #[test]
    fn polygon_components_stay_disconnected_and_antimeridian_projection_is_local() {
        const AREA: InfluenceArea = InfluenceArea::Polygon {
            polygons: &[
                SQUARE_WITH_HOLE,
                PolygonArea {
                    ring_indices: &[],
                    center: [-1.0, 0.0, 0.0],
                    east: [0.0, -1.0, 0.0],
                    north: [0.0, 0.0, 1.0],
                    cap_cos: 0.99999998,
                    representative_center: (0.0, 180.0),
                    rings: &[&[
                        (-100.0, -100.0),
                        (100.0, -100.0),
                        (100.0, 100.0),
                        (-100.0, 100.0),
                        (-100.0, -100.0),
                    ]],
                },
            ],
            boundary_falloff_m: 40.0,
        };
        assert!(AREA.strength(0.0, 0.0).is_none()); // First component's hole.
        assert!(AREA.strength(0.0, 90.0_f64.to_radians()).is_none());
        for lon in [179.9999_f64, 180.0, -179.9999] {
            assert_eq!(AREA.strength(0.0, lon.to_radians()), Some(1.0));
        }
    }

    #[test]
    fn corridor_crosses_dateline_without_extending_around_the_globe() {
        let area = InfluenceArea::Corridor {
            points: &[(0.0, 179.99), (0.0, -179.99)],
            core_half_width_m: 50.0,
            outer_half_width_m: 100.0,
        };
        for lon in [179.995_f64, -180.0, -179.995] {
            assert_eq!(area.strength(0.0, lon.to_radians()), Some(1.0));
        }
        assert!(area.strength(0.0, 0.0).is_none());
        assert!(area.strength(0.0, 179.98_f64.to_radians()).is_none());
        assert!(area.strength(0.0, (-179.98_f64).to_radians()).is_none());
        let (lat, lon) = area.representative_center();
        assert!(lat.abs() < 1e-12 && (lon.abs() - 180.0).abs() < 1e-12);
    }

    #[test]
    fn dateline_distance_and_overlap_ties_are_deterministic() {
        let mut p = PLACES[0];
        p.scope_tier = PlaceScope::District;
        p.areas = &[InfluenceArea::Circle {
            latitude_deg: 0.0,
            longitude_deg: 179.999,
            core_radius_m: 500.0,
            outer_radius_m: 1000.0,
        }];
        assert_eq!(
            p.influence(0.0, (-179.999_f64).to_radians())
                .unwrap()
                .strength,
            1.0
        );
        let mut a = p
            .influence(0.0, p.areas[0].representative_center().1.to_radians())
            .unwrap();
        let mut b = a;
        a.source_id = "source:a";
        b.source_id = "source:b";
        let mut forward = [None, None, None];
        let mut backward = [None, None, None];
        insert(&mut forward, a);
        insert(&mut forward, b);
        insert(&mut backward, b);
        insert(&mut backward, a);
        assert_eq!(forward, backward);
        assert_eq!(forward[0].unwrap().source_id, "source:a");
    }
}
