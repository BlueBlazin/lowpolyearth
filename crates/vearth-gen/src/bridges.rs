//! Continuous bridge geometry and its source ownership metadata.
//!
//! Prepared OSM topology supplies enriched physical structures. Other mapped
//! bridge ways use plain metric slabs on a shared geographic height lattice;
//! tile fragments never establish separate elevations or voxel-sized lifts.

use crate::tier0::GridProj;
use vearth_core::frame::ChunkFrame;
use vearth_data::bridge_pack::{BridgeStructure, CanonicalBridge};
use vearth_data::vectors::{FeaturePack, RoadClass};

#[path = "bridge_fallback.rs"]
mod fallback;
#[path = "bridge_surface.rs"]
mod surface;
pub(crate) use surface::generate as source_surface;

/// Start canonical bridge synthesis at the same level as the road network.
pub const BRIDGE_MIN_LEVEL: u8 = 11;

pub fn source_footprint(frame: &ChunkFrame, pack: &FeaturePack) -> Option<crate::SourceFootprint> {
    if frame.addr.cell.level < BRIDGE_MIN_LEVEL {
        return None;
    }
    let (center_lat, center_lon) = frame.column_latlon(0.5, 0.5);
    let mut radius_rad = 0.0f64;
    let mut found = false;
    for bridge in pack.bridges.iter().filter(|_| pack.bridge_authority) {
        let pad = f64::from(bridge.deck_width_m.max(bridge.road_width_m)) + 8.0;
        for point in bridge.alignment.iter().chain(&bridge.deck_outline) {
            if !point.iter().all(|v| v.is_finite()) || !pad.is_finite() {
                continue;
            }
            found = true;
            radius_rad = radius_rad.max(
                (f64::from(point[0]).abs() + f64::from(point[1]).abs() + pad * 2.0)
                    / vearth_core::ellipsoid::MEAN_RADIUS
                    * 1.01,
            );
        }
    }
    if fallback::has_source(pack) {
        for (x, z) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
            let (lat, lon) = frame.column_latlon(x, z);
            let mut delta = (lon - center_lon).abs();
            if delta > std::f64::consts::PI {
                delta = std::f64::consts::TAU - delta;
            }
            radius_rad = radius_rad.max(
                (lat - center_lat).abs()
                    + delta * center_lat.cos().abs()
                    + fallback::DATUM_MARGIN_M / vearth_core::ellipsoid::MEAN_RADIUS,
            );
        }
        found = true;
    }
    found.then_some(crate::SourceFootprint {
        center_lat,
        center_lon,
        radius_rad: radius_rad + 1.0e-9,
        sample_hint_m: crate::datum_pitch_m(center_lat),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BridgeKind {
    Beam,
    Arch,
    Truss,
    Suspension,
}

pub(crate) fn catalog_claims_road(
    road: &vearth_data::vectors::Road,
    bridges: &[CanonicalBridge],
) -> bool {
    if road.pts.len() < 2 {
        return false;
    }
    bridges
        .iter()
        .filter(|b| surface::valid_source(b))
        .any(|bridge| {
            if (road.class == RoadClass::Rail) != (bridge.class == RoadClass::Rail) {
                return false;
            }
            let radius = f64::from(bridge.deck_width_m) * 0.5 + 2.0;
            let mut covered = 0.0;
            let mut total = 0.0;
            for pair in road.pts.windows(2) {
                let a = glam::DVec2::from_array(pair[0].map(f64::from));
                let b = glam::DVec2::from_array(pair[1].map(f64::from));
                let length = a.distance(b);
                total += length;
                let mid = (a + b) * 0.5;
                if bridge.alignment.windows(2).any(|segment| {
                    let p = glam::DVec2::from_array(segment[0].map(f64::from));
                    let q = glam::DVec2::from_array(segment[1].map(f64::from));
                    let d = q - p;
                    if d.length_squared() < 1.0e-6 || length < 1.0e-6 {
                        return false;
                    }
                    let t = ((mid - p).dot(d) / d.length_squared()).clamp(0.0, 1.0);
                    mid.distance(p + d * t) <= radius
                        && d.normalize().dot((b - a) / length).abs() > 0.8
                }) {
                    covered += length;
                }
            }
            total > 0.0 && covered / total > 0.9
        })
}

struct Profile {
    points_cols: Vec<[f32; 2]>,
    cumulative: Vec<f64>,
    station_s: Vec<f64>,
    station_deck: Vec<f32>,
    station_terrain: Vec<f32>,
    bridge_start: f64,
    bridge_end: f64,
    water_start: f64,
    water_end: f64,
    deck_width: f64,
    road_width: f64,
    kind: BridgeKind,
    tagged_structure: BridgeStructure,
}

struct StripHit {
    s: f64,
    lateral_m: f64,
}

fn closest_path_hit(profile: &Profile, column: [f32; 2], step_m: f64) -> Option<StripHit> {
    let mut best: Option<StripHit> = None;
    for index in 0..profile.points_cols.len() - 1 {
        let a = profile.points_cols[index];
        let b = profile.points_cols[index + 1];
        let direction = [b[0] - a[0], b[1] - a[1]];
        let length2 = direction[0] * direction[0] + direction[1] * direction[1];
        if length2 < 1e-8 {
            continue;
        }
        let offset = [column[0] - a[0], column[1] - a[1]];
        let t = ((offset[0] * direction[0] + offset[1] * direction[1]) / length2).clamp(0.0, 1.0);
        let closest = [a[0] + direction[0] * t, a[1] + direction[1] * t];
        let lateral_m = ((column[0] - closest[0]).powi(2) + (column[1] - closest[1]).powi(2)).sqrt()
            as f64
            * step_m;
        let s = profile.cumulative[index]
            + (profile.cumulative[index + 1] - profile.cumulative[index]) * t as f64;
        if best
            .as_ref()
            .is_none_or(|current| lateral_m < current.lateral_m)
        {
            best = Some(StripHit { s, lateral_m });
        }
    }
    best
}

fn build_profile(
    bridge: &CanonicalBridge,
    projection: &GridProj,
    elev_en: &dyn Fn(f64, f64) -> f32,
) -> Option<Profile> {
    if bridge.alignment.len() < 2 {
        return None;
    }
    let points_en: Vec<[f64; 2]> = bridge
        .alignment
        .iter()
        .map(|point| [point[0] as f64, point[1] as f64])
        .collect();
    let points_cols = bridge
        .alignment
        .iter()
        .map(|&point| projection.to_cols(point))
        .collect();
    let mut cumulative = Vec::with_capacity(points_en.len());
    cumulative.push(0.0);
    for segment in points_en.windows(2) {
        let length = (segment[1][0] - segment[0][0]).hypot(segment[1][1] - segment[0][1]);
        cumulative.push(cumulative.last().copied().unwrap() + length);
    }
    let total = *cumulative.last()?;
    if total < 8.0 {
        return None;
    }
    let bridge_start = bridge.bridge_start_m as f64;
    let bridge_end = bridge.bridge_end_m as f64;
    let water_start = bridge.water_start_m as f64;
    let water_end = bridge.water_end_m as f64;
    if !(0.0 <= bridge_start
        && bridge_start <= water_start
        && water_start < water_end
        && water_end <= bridge_end
        && bridge_end <= total + 2.0)
    {
        return None;
    }

    let span = water_end - water_start;
    let water_level = if bridge.water_is_ocean {
        0.0
    } else {
        water_level(&points_en, &cumulative, water_start, water_end, elev_en)
    };
    let target = water_level + clearance_m(bridge.class, span) as f32;
    let station_count = ((total / 8.0).ceil() as usize).clamp(2, 4096);
    let station_s: Vec<_> = (0..=station_count)
        .map(|index| total * index as f64 / station_count as f64)
        .collect();
    let raw_terrain: Vec<_> = station_s
        .iter()
        .map(|&s| {
            let point = point_at_s(&points_en, &cumulative, s);
            elev_en(point[0], point[1])
        })
        .collect();
    let station_terrain: Vec<_> = (0..raw_terrain.len())
        .map(|index| {
            let radius = 2usize;
            let start = index.saturating_sub(radius);
            let end = (index + radius).min(raw_terrain.len() - 1);
            raw_terrain[start..=end].iter().sum::<f32>() / (end + 1 - start) as f32
        })
        .collect();
    let available_left = (water_start - 0.0).max(1.0);
    let available_right = (total - water_end).max(1.0);
    let desired_ramp = (span * 0.35).clamp(25.0, 180.0);
    let left_ramp = desired_ramp.min(available_left);
    let right_ramp = desired_ramp.min(available_right);
    let station_deck = station_s
        .iter()
        .zip(&station_terrain)
        .map(|(&s, &terrain)| {
            let weight = if s < water_start {
                smooth(1.0 - (water_start - s) / left_ramp)
            } else if s > water_end {
                smooth(1.0 - (s - water_end) / right_ramp)
            } else {
                1.0
            } as f32;
            let deck = terrain + (target - terrain) * weight;
            if (water_start..=water_end).contains(&s) {
                deck.max(water_level + 1.2)
            } else {
                deck
            }
        })
        .collect();
    let kind = match bridge.structure {
        BridgeStructure::Beam => BridgeKind::Beam,
        BridgeStructure::Arch | BridgeStructure::TiedArch => BridgeKind::Arch,
        BridgeStructure::Truss => BridgeKind::Truss,
        BridgeStructure::Suspension | BridgeStructure::CableStayed => BridgeKind::Suspension,
    };
    Some(Profile {
        points_cols,
        cumulative,
        station_s,
        station_deck,
        station_terrain,
        bridge_start,
        bridge_end,
        water_start,
        water_end,
        deck_width: bridge.deck_width_m as f64,
        road_width: bridge.road_width_m as f64,
        kind,
        tagged_structure: bridge.structure,
    })
}

fn upper_band(profile: &Profile, s: f64, deck: f32) -> (f32, f32) {
    let span = profile.water_end - profile.water_start;
    let t = ((s - profile.water_start) / span).clamp(0.0, 1.0);
    match profile.kind {
        BridgeKind::Beam => (f32::NAN, f32::NAN),
        BridgeKind::Arch => {
            if profile.tagged_structure != BridgeStructure::TiedArch {
                let count = arch_spans(profile);
                let bay = span / count as f64;
                let fraction = ((s - profile.water_start) / bay).clamp(0.0, count as f64);
                let t = if fraction >= count as f64 {
                    1.0
                } else {
                    fraction.fract()
                };
                let rise = (bay * 0.22).clamp(2.0, 6.0);
                let top = f64::from(deck) - 1.2;
                let intrados = top - 0.65 - rise * (1.0 - 4.0 * t * (1.0 - t));
                return (intrados as f32, top as f32);
            }
            let rise = (span / 5.5).clamp(6.0, 75.0) as f32;
            let rib = (span / 45.0).clamp(1.0, 2.5) as f32;
            let height = deck + rise * (4.0 * t * (1.0 - t)) as f32;
            if height > deck + 1.5 {
                (height - rib, height)
            } else {
                (f32::NAN, f32::NAN)
            }
        }
        BridgeKind::Truss => {
            let depth = (span / 11.0).clamp(4.0, 10.0) as f32;
            (deck + depth - 1.2, deck + depth)
        }
        BridgeKind::Suspension => {
            let top = suspension_top(profile, deck);
            let low = deck + 3.0;
            let height = if profile.tagged_structure == BridgeStructure::CableStayed {
                let tower = if t < 0.5 { 0.32 } else { 0.68 };
                let reach = 0.32;
                let weight = (1.0 - (t - tower).abs() / reach).clamp(0.0, 1.0) as f32;
                deck + (top - deck) * weight
            } else if (0.2..=0.8).contains(&t) {
                let u = ((t - 0.2) / 0.6) as f32;
                top + (low - top) * (4.0 * u * (1.0 - u))
            } else {
                let u = if t < 0.2 { t / 0.2 } else { (1.0 - t) / 0.2 } as f32;
                deck + (top - deck) * u
            };
            if height > deck + 1.5 {
                (height - 1.2, height)
            } else {
                (f32::NAN, f32::NAN)
            }
        }
    }
}

fn arch_spans(profile: &Profile) -> usize {
    ((profile.water_end - profile.water_start) / 22.0)
        .ceil()
        .max(1.0) as usize
}

fn suspension_top(profile: &Profile, deck: f32) -> f32 {
    let span = profile.water_end - profile.water_start;
    deck + ((span * 0.6) / 9.0).max(12.0) as f32
}

fn water_level(
    points: &[[f64; 2]],
    cumulative: &[f64],
    start: f64,
    end: f64,
    elev_en: &dyn Fn(f64, f64) -> f32,
) -> f32 {
    let mut level = f32::INFINITY;
    for index in 0..8 {
        let s = start + (end - start) * index as f64 / 7.0;
        let point = point_at_s(points, cumulative, s);
        level = level.min(elev_en(point[0], point[1]));
    }
    level
}

fn point_at_s(points: &[[f64; 2]], cumulative: &[f64], s: f64) -> [f64; 2] {
    let s = s.clamp(0.0, *cumulative.last().unwrap_or(&0.0));
    let upper = cumulative
        .partition_point(|value| *value < s)
        .min(points.len() - 1);
    if upper == 0 {
        return points[0];
    }
    let lower = upper - 1;
    let length = cumulative[upper] - cumulative[lower];
    let t = if length > 1e-9 {
        (s - cumulative[lower]) / length
    } else {
        0.0
    };
    [
        points[lower][0] + (points[upper][0] - points[lower][0]) * t,
        points[lower][1] + (points[upper][1] - points[lower][1]) * t,
    ]
}

fn interpolate(stations: &[f64], values: &[f32], s: f64) -> f32 {
    let upper = stations
        .partition_point(|value| *value < s)
        .min(stations.len() - 1);
    if upper == 0 {
        return values[0];
    }
    let lower = upper - 1;
    let length = stations[upper] - stations[lower];
    let t = if length > 1e-9 {
        ((s - stations[lower]) / length).clamp(0.0, 1.0) as f32
    } else {
        0.0
    };
    values[lower] + (values[upper] - values[lower]) * t
}

fn clearance_m(class: RoadClass, span_m: f64) -> f64 {
    match class {
        RoadClass::Path => (2.5 + span_m * 0.03).min(9.0),
        RoadClass::Rail => (3.0 + span_m * 0.03).min(15.0),
        _ => (2.5 + span_m * 0.05).min(42.0),
    }
}

fn smooth(value: f64) -> f64 {
    let value = value.clamp(0.0, 1.0);
    value * value * (3.0 - 2.0 * value)
}
