//! Plain source-way bridge decks share one geographic height lattice. Tile
//! fragments never choose independent deck datums or voxel-sized clearances.

use crate::{N, road_surface::offset_polyline, tier0::GridProj};
use glam::{DVec2, DVec3};
use std::collections::{BTreeMap, HashSet};
use vearth_core::{ellipsoid::MEAN_RADIUS, frame::ChunkFrame};
use vearth_data::vectors::{FeaturePack, RoadClass};

pub(super) const DATUM_MARGIN_M: f64 = 128.0;
const GRID_M: f64 = 64.0;
const CLEARANCE_M: f64 = 5.0;
pub(super) const THICKNESS_M: f32 = 0.65;
const EPS: f64 = 1.0e-7;

pub(super) struct Patch {
    pub points: [DVec3; 3], // east, geodetic height, north, filled side upward
    pub class: RoadClass,
    pub appearance: crate::SurfaceAppearance,
}

pub(super) fn has_source(pack: &FeaturePack) -> bool {
    pack.roads
        .iter()
        .any(|road| road.is_bridge && road.pts.len() >= 2)
}

fn point(p: [f32; 2]) -> DVec2 {
    DVec2::from_array(p.map(f64::from))
}

fn clip(poly: &[DVec2], a: DVec2, b: DVec2) -> Vec<DVec2> {
    let edge = b - a;
    let mut out = Vec::new();
    for (&p, &q) in poly
        .iter()
        .zip(poly.iter().cycle().skip(1))
        .take(poly.len())
    {
        let dp = edge.perp_dot(p - a);
        let dq = edge.perp_dot(q - a);
        if dp >= -EPS {
            out.push(p);
        }
        if (dp >= -EPS) != (dq >= -EPS) {
            out.push(p.lerp(q, dp / (dp - dq)));
        }
    }
    out
}

pub(super) fn patches(
    frame: &ChunkFrame,
    pack: &FeaturePack,
    elev: &dyn Fn(f64, f64) -> f32,
) -> (Vec<Patch>, bool) {
    patches_with_cliff_cap::<true>(frame, pack, elev)
}

fn patches_with_cliff_cap<const CLIFF_CAP: bool>(
    frame: &ChunkFrame,
    pack: &FeaturePack,
    elev: &dyn Fn(f64, f64) -> f32,
) -> (Vec<Patch>, bool) {
    if !pack.roads.iter().any(|road| road.is_bridge) {
        return (Vec::new(), true);
    }
    let projection = GridProj::of(
        frame,
        frame.column_latlon(0.5, 0.5).0,
        frame.column_latlon(0.5, 0.5).1,
    );
    let (lat, lon) = frame.column_latlon(0.5, 0.5);
    let count = (std::f64::consts::TAU * MEAN_RADIUS / GRID_M).ceil();
    let angle = std::f64::consts::TAU / count;
    let to_grid = |p: DVec2| {
        DVec2::new(
            (lon + p.x / (MEAN_RADIUS * lat.cos())) / angle,
            (lat + p.y / MEAN_RADIUS) / angle,
        )
    };
    let from_grid = |p: DVec2| {
        DVec2::new(
            (p.x * angle - lon) * MEAN_RADIUS * lat.cos(),
            (p.y * angle - lat) * MEAN_RADIUS,
        )
    };
    let mut cell_lo = DVec2::splat(f64::INFINITY);
    let mut cell_hi = DVec2::splat(f64::NEG_INFINITY);
    for (x, z) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
        let (a, b) = frame.column_latlon(x, z);
        let mut dl = b - lon;
        if dl > std::f64::consts::PI {
            dl -= std::f64::consts::TAU;
        }
        if dl < -std::f64::consts::PI {
            dl += std::f64::consts::TAU;
        }
        let q = DVec2::new((lon + dl) / angle, a / angle);
        cell_lo = cell_lo.min(q);
        cell_hi = cell_hi.max(q);
    }
    let source = &pack.roads;
    let ordinary_segments: Vec<_> = source
        .iter()
        .filter(|r| !r.is_bridge)
        .flat_map(|r| r.pts.windows(2))
        .map(|pair| [point(pair[0]), point(pair[1])])
        .collect();
    let ramp_ends: Vec<_> = source
        .iter()
        .filter(|r| r.is_bridge)
        .flat_map(|r| r.pts.first().into_iter().chain(r.pts.last()))
        .copied()
        .map(point)
        .filter(|p| {
            ordinary_segments.iter().any(|[a, b]| {
                let edge = *b - *a;
                let t = if edge.length_squared() > EPS {
                    ((*p - *a).dot(edge) / edge.length_squared()).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                p.distance_squared(*a + edge * t) < 4.0
            })
        })
        .collect();
    let mut ground = BTreeMap::<(i64, i64), f64>::new();
    let mut owned = BTreeMap::<(i64, i64, usize), Vec<Vec<DVec2>>>::new();
    let mut output = Vec::new();
    let mut complete = true;
    let mut seen = HashSet::new();
    let mut bridges: Vec<_> = source
        .iter()
        .filter(|r| {
            r.is_bridge && !(pack.bridge_authority && super::catalog_claims_road(r, &pack.bridges))
        })
        .collect();
    let materials = crate::road_materials::Resolver::new(frame, pack);
    bridges.sort_by(|a, b| {
        b.width_m()
            .total_cmp(&a.width_m())
            .then(a.profile.seed.cmp(&b.profile.seed))
    });
    for road in bridges {
        if road.pts.iter().any(|&p| !point(p).is_finite()) {
            complete = false;
            continue;
        }
        let mut points: Vec<_> = road.pts.iter().copied().map(point).collect();
        points.dedup_by(|a, b| a.distance_squared(*b) < EPS);
        if points.len() < 2 {
            complete = false;
            continue;
        }
        // Millimetre keys absorb reprojection roundoff without merging roads.
        let key: Vec<_> = points
            .iter()
            .map(|p| {
                let q = to_grid(*p);
                (
                    (q.x * GRID_M * 1000.0).round() as i64,
                    (q.y * GRID_M * 1000.0).round() as i64,
                )
            })
            .collect();
        let reverse: Vec<_> = key.iter().copied().rev().collect();
        if !seen.insert((road.class as u8, road.profile.detail, key.min(reverse))) {
            continue;
        }
        let appearance = materials.appearance(road);
        let half = f64::from(road.width_m()) * 0.5;
        let mut polygon = offset_polyline(&points, -half);
        polygon.extend(offset_polyline(&points, half).into_iter().rev());
        let mut area = 0.0;
        for (a, b) in polygon
            .iter()
            .zip(polygon.iter().cycle().skip(1))
            .take(polygon.len())
        {
            area += a.perp_dot(*b);
        }
        if area < 0.0 {
            polygon.reverse();
        }
        let rings = vec![polygon.iter().map(|p| p.as_vec2().to_array()).collect()];
        let pieces =
            crate::building_surface::source_polygon_cells(&rings, 1.0e9).unwrap_or_else(|| {
                complete = false;
                points
                    .windows(2)
                    .map(|p| {
                        let d = (p[1] - p[0]).normalize();
                        let n = DVec2::new(-d.y, d.x) * half;
                        [p[0] - n, p[1] - n, p[1] + n, p[0] + n]
                    })
                    .collect()
            });
        let qpolys: Vec<Vec<_>> = pieces
            .into_iter()
            .map(|p| p.into_iter().map(to_grid).collect())
            .collect();
        let qpoly: Vec<_> = polygon.into_iter().map(to_grid).collect();
        let lo = qpoly
            .iter()
            .copied()
            .fold(DVec2::splat(f64::INFINITY), DVec2::min)
            .max(cell_lo - DVec2::ONE);
        let hi = qpoly
            .iter()
            .copied()
            .fold(DVec2::splat(f64::NEG_INFINITY), DVec2::max)
            .min(cell_hi + DVec2::ONE);
        if !lo.is_finite() || !hi.is_finite() || qpoly.iter().any(|p| !p.is_finite()) {
            complete = false;
            continue;
        }
        if lo.x > hi.x || lo.y > hi.y {
            continue;
        }
        for y in (lo.y.floor() as i64)..=(hi.y.floor() as i64) {
            for x in (lo.x.floor() as i64)..=(hi.x.floor() as i64) {
                let q = [
                    DVec2::new(x as f64, y as f64),
                    DVec2::new(x as f64 + 1.0, y as f64),
                    DVec2::new(x as f64 + 1.0, y as f64 + 1.0),
                    DVec2::new(x as f64, y as f64 + 1.0),
                ];
                let h = q.map(|p| {
                    *ground
                        .entry(((p.x as i64).rem_euclid(count as i64), p.y as i64))
                        .or_insert_with(|| {
                            let en = from_grid(p);
                            f64::from(elev(en.x, en.y))
                        })
                });
                if h.iter().any(|v| !v.is_finite()) {
                    complete = false;
                    continue;
                }
                // Subdivide only the canonical 64m tiles around the authored
                // cliff. Fine vertices interpolate the original coarse planes
                // before correction, so their unmodified outer edges still
                // coincide with neighboring unrefined tiles and cell borders.
                let refined = CLIFF_CAP
                    && crate::landmarks::bridge_ground_refines_tile(
                        (y as f64 + 0.5) * angle,
                        (x as f64 + 0.5) * angle,
                        GRID_M * 0.5,
                    );
                let divisions = if refined { 8_i64 } else { 1_i64 };
                for sy in 0..divisions {
                    for sx in 0..divisions {
                        let origin = DVec2::new(x as f64, y as f64);
                        let sub_q =
                            [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)].map(|(dx, dy)| {
                                origin
                                    + DVec2::new(
                                        (sx as f64 + dx) / divisions as f64,
                                        (sy as f64 + dy) / divisions as f64,
                                    )
                            });
                        let sub_h = if refined {
                            sub_q.map(|p| {
                                let dx = p.x - x as f64;
                                let dy = p.y - y as f64;
                                let raw = if dy <= dx {
                                    h[0] + (h[1] - h[0]) * dx + (h[2] - h[1]) * dy
                                } else {
                                    h[0] + (h[2] - h[3]) * dx + (h[3] - h[0]) * dy
                                };
                                let corrected = crate::landmarks::corrected_bridge_ground(
                                    p.y * angle,
                                    p.x * angle,
                                    raw as f32,
                                );
                                if corrected == raw as f32 {
                                    raw
                                } else {
                                    f64::from(corrected)
                                }
                            })
                        } else {
                            h
                        };
                        for (half, ids) in [[0, 1, 2], [0, 2, 3]].into_iter().enumerate() {
                            let t = ids.map(|i| sub_q[i]);
                            for qpoly in &qpolys {
                                let mut poly = qpoly.clone();
                                for i in 0..3 {
                                    poly = clip(&poly, t[i], t[(i + 1) % 3]);
                                }
                                if poly.len() < 3 {
                                    continue;
                                }
                                let origin = sub_q[0];
                                let poly = crate::road_surface::canonical(
                                    poly.into_iter().map(|p| p - origin).collect(),
                                );
                                if poly.len() < 3 {
                                    continue;
                                }
                                let cover = owned
                                    .entry((
                                        x * 8 + sx * 8 / divisions,
                                        y * 8 + sy * 8 / divisions,
                                        half,
                                    ))
                                    .or_default();
                                let mut remaining = vec![poly];
                                for previous in cover.iter() {
                                    remaining = remaining
                                        .into_iter()
                                        .flat_map(|p| crate::road_surface::difference(&p, previous))
                                        .collect();
                                }
                                for poly in remaining {
                                    cover.push(poly.clone());
                                    let poly: Vec<_> =
                                        poly.into_iter().map(|p| p + origin).collect();
                                    let vertices: Vec<_> = poly
                                        .iter()
                                        .map(|&p| {
                                            let a = t[1] - t[0];
                                            let b = t[2] - t[0];
                                            let v = p - t[0];
                                            let det = a.perp_dot(b);
                                            let u = v.perp_dot(b) / det;
                                            let w = a.perp_dot(v) / det;
                                            let base = sub_h[ids[0]]
                                                + (sub_h[ids[1]] - sub_h[ids[0]]) * u
                                                + (sub_h[ids[2]] - sub_h[ids[0]]) * w;
                                            let en = from_grid(p);
                                            let distance = ramp_ends
                                                .iter()
                                                .map(|e| en.distance(*e))
                                                .fold(f64::INFINITY, f64::min);
                                            let weight = (distance / 48.0).clamp(0.0, 1.0);
                                            DVec3::new(en.x, base + CLEARANCE_M * weight, en.y)
                                        })
                                        .collect();
                                    for i in 1..vertices.len().saturating_sub(1) {
                                        let points = [vertices[0], vertices[i + 1], vertices[i]];
                                        if (points[1] - points[0])
                                            .cross(points[2] - points[0])
                                            .length_squared()
                                            > EPS
                                        {
                                            output.push(Patch {
                                                points,
                                                class: road.class,
                                                appearance,
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    // The same unclipped triangles drive every neighboring cell's collision.
    output.retain(|patch| {
        let p = patch
            .points
            .map(|p| projection.to_cols([p.x as f32, p.z as f32]));
        (0..2).all(|axis| p.iter().any(|p| p[axis] >= 0.0) && p.iter().any(|p| p[axis] <= N as f32))
    });
    (output, complete)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vearth_core::{
        cell::{CellId, ChunkAddr},
        ellipsoid,
    };
    use vearth_data::{
        bridge_pack::{BridgeMaterial, BridgeStructure, CanonicalBridge},
        vectors::Road,
    };
    fn frame(level: u8) -> ChunkFrame {
        ChunkFrame::of(ChunkAddr {
            cell: CellId::from_dir(
                ellipsoid::geodetic_to_ecef(
                    18.5207_f64.to_radians(),
                    73.8524_f64.to_radians(),
                    0.0,
                ),
                level,
            ),
            k: 0,
        })
    }
    fn road(y: f32) -> Road {
        Road {
            profile: Default::default(),
            pts: vec![[-1000.0, y], [1000.0, y]],
            class: RoadClass::Major,
            is_bridge: true,
        }
    }

    #[test]
    fn cliff_cap_keeps_distant_bridge_patches_bit_identical() {
        let mut pack = FeaturePack::empty();
        pack.roads = vec![road(0.0), road(40.0)];
        for level in [13, 17, 19] {
            let frame = frame(level);
            let terrain = |e: f64, n: f64| (22.75 + e * 0.017 + n * 0.023) as f32;
            let (plain, plain_complete) = patches_with_cliff_cap::<false>(&frame, &pack, &terrain);
            let (actual, complete) = patches(&frame, &pack, &terrain);
            assert!(plain_complete && complete && !plain.is_empty());
            assert_eq!(actual.len(), plain.len());
            for (a, b) in actual.iter().zip(&plain) {
                assert_eq!(a.class, b.class);
                assert_eq!(
                    a.points.map(|p| p.to_array().map(f64::to_bits)),
                    b.points.map(|p| p.to_array().map(f64::to_bits)),
                );
            }
        }
    }

    #[test]
    fn cliff_refinement_preserves_shared_cell_and_coarse_lattice_edges() {
        let anchor_lat = 29.564827_f64.to_radians();
        let anchor_lon = 106.575395_f64.to_radians();
        let count = (std::f64::consts::TAU * MEAN_RADIUS / GRID_M).ceil();
        let angle = std::f64::consts::TAU / count;
        let y = (anchor_lat / angle).floor();
        let mut boundary = None;
        for k in 0..10 {
            let x = (anchor_lon / angle).floor() + k as f64;
            if crate::landmarks::bridge_ground_refines_tile(
                (y + 0.5) * angle,
                (x + 0.5) * angle,
                GRID_M / 2.0,
            ) && !crate::landmarks::bridge_ground_refines_tile(
                (y + 0.5) * angle,
                (x + 1.5) * angle,
                GRID_M / 2.0,
            ) {
                boundary = Some(((y + 0.5) * angle, (x + 1.0) * angle));
                break;
            }
        }
        let boundary = boundary.expect("local refinement must end on the shared coarse lattice");
        for (lat, lon) in [
            (
                anchor_lat + 47.0 / MEAN_RADIUS,
                anchor_lon - 17.0 / (MEAN_RADIUS * anchor_lat.cos()),
            ),
            boundary,
        ] {
            let base_cell =
                CellId::from_dir(ellipsoid::geodetic_to_ecef(lat, lon, 0.0).normalize(), 19);
            let a = ChunkFrame::of(ChunkAddr {
                cell: base_cell,
                k: 0,
            });
            let (edge_lat, edge_lon) = a.column_latlon(1.0, 0.5);
            let (next_lat, next_lon) = a.column_latlon(1.001, 0.5);
            let b = ChunkFrame::of(ChunkAddr {
                cell: CellId::from_dir(
                    ellipsoid::geodetic_to_ecef(next_lat, next_lon, 0.0).normalize(),
                    19,
                ),
                k: 0,
            });
            assert_ne!(a.addr.cell, b.addr.cell);
            let sample = |frame: &ChunkFrame, sample_lat: f64, sample_lon: f64| {
                let (center_lat, center_lon) = frame.column_latlon(0.5, 0.5);
                let en = |lat: f64, lon: f64| {
                    DVec2::new(
                        (lon - center_lon) * MEAN_RADIUS * center_lat.cos(),
                        (lat - center_lat) * MEAN_RADIUS,
                    )
                };
                let mut pack = FeaturePack::empty();
                pack.roads.push(Road {
                    profile: Default::default(),
                    pts: [-300.0, 300.0]
                        .map(|e| {
                            en(
                                sample_lat,
                                sample_lon + e / (MEAN_RADIUS * sample_lat.cos()),
                            )
                            .as_vec2()
                            .to_array()
                        })
                        .into(),
                    class: RoadClass::Major,
                    is_bridge: true,
                });
                let terrain = |e: f64, n: f64| {
                    let lat = center_lat + n / MEAN_RADIUS;
                    let lon = center_lon + e / (MEAN_RADIUS * center_lat.cos());
                    (231.0
                        + (lat - anchor_lat) * MEAN_RADIUS * 0.013
                        + (lon - anchor_lon) * MEAN_RADIUS * anchor_lat.cos() * 0.027)
                        as f32
                };
                let (patches, complete) = patches(frame, &pack, &terrain);
                assert!(complete && !patches.is_empty());
                let point = en(sample_lat, sample_lon);
                patches
                    .iter()
                    .find_map(|patch| {
                        let p = patch.points.map(|p| DVec2::new(p.x, p.z));
                        let a = p[1] - p[0];
                        let b = p[2] - p[0];
                        let v = point - p[0];
                        let determinant = a.perp_dot(b);
                        let u = v.perp_dot(b) / determinant;
                        let w = a.perp_dot(v) / determinant;
                        (u >= -1e-5 && w >= -1e-5 && u + w <= 1.0 + 1e-5).then(|| {
                            patch.points[0].y
                                + (patch.points[1].y - patch.points[0].y) * u
                                + (patch.points[2].y - patch.points[0].y) * w
                        })
                    })
                    .expect("both neighboring cells must own the shared road sample")
            };
            let ha = sample(&a, edge_lat, edge_lon);
            let hb = sample(&b, edge_lat, edge_lon);
            assert!((ha - hb).abs() < 0.0001, "bridge height seam: {ha} vs {hb}");
            // At the refined/unrefined lattice edge, the cap is a no-op and
            // both tessellations retain the original coarse plane exactly.
            if (lat, lon) == boundary {
                let raw = (231.0
                    + (lat - anchor_lat) * MEAN_RADIUS * 0.013
                    + (lon - anchor_lon) * MEAN_RADIUS * anchor_lat.cos() * 0.027)
                    as f32;
                assert_eq!(
                    crate::landmarks::corrected_bridge_ground(lat, lon, raw),
                    raw
                );
                for distance in [-0.02, 0.0, 0.02] {
                    let height = sample(&a, lat, lon + distance / (MEAN_RADIUS * lat.cos()));
                    assert!(
                        (height - f64::from(raw) - 5.0 - distance * 0.027).abs() < 0.0001,
                        "refined/coarse lattice join left the original plane: {height}"
                    );
                }
            }
        }
    }
    #[test]
    fn uncatalogued_source_crossings_have_metric_decks_at_every_visible_level() {
        for level in [11, 13, 14, 15, 17, 19] {
            let mut pack = FeaturePack::empty();
            pack.roads.push(road(0.0));
            let (patches, complete) = patches(&frame(level), &pack, &|_, _| 22.75);
            assert!(complete);
            assert!(!patches.is_empty(), "level {level}");
            for patch in patches {
                assert!(patch.points.iter().all(|p| (p.y - 27.75).abs() < 1e-9));
                assert!(
                    (patch.points[1] - patch.points[0])
                        .cross(patch.points[2] - patch.points[0])
                        .y
                        >= -1e-8
                );
                assert!(
                    patch
                        .points
                        .iter()
                        .all(|p| p.z.abs() <= f64::from(RoadClass::Major.width_m()) * 0.5 + 1e-5)
                );
            }
        }
    }
    #[test]
    fn partial_catalog_enriches_only_matching_parallel_source_crossings() {
        let mut pack = FeaturePack::empty();
        pack.bridge_authority = true;
        pack.roads = vec![road(0.0), road(40.0)];
        pack.bridges.push(CanonicalBridge {
            id: 1,
            class: RoadClass::Major,
            alignment: vec![[-1100.0, 0.0], [1100.0, 0.0]],
            deck_outline: Vec::new(),
            bridge_start_m: 0.0,
            bridge_end_m: 2200.0,
            water_start_m: 100.0,
            water_end_m: 2100.0,
            deck_width_m: 14.0,
            road_width_m: 12.0,
            water_is_ocean: false,
            structure: BridgeStructure::Beam,
            material: BridgeMaterial::Unknown,
        });
        let (patches, complete) = patches(&frame(15), &pack, &|_, _| 0.0);
        assert!(complete);
        assert!(!patches.is_empty());
        assert!(patches.iter().flat_map(|p| p.points).all(|p| p.z > 30.0));
        let crossing = Road {
            profile: Default::default(),
            pts: vec![[0.0, -200.0], [0.0, 200.0]],
            class: RoadClass::Major,
            is_bridge: true,
        };
        assert!(!super::super::catalog_claims_road(&crossing, &pack.bridges));
    }
    #[test]
    fn bridge_approach_at_ordinary_way_interior_matches_split_way() {
        let mut pack = FeaturePack::empty();
        pack.roads = vec![
            Road {
                profile: Default::default(),
                pts: vec![[-10.0, 0.0], [10.0, 0.0]],
                class: RoadClass::Path,
                is_bridge: true,
            },
            Road {
                profile: Default::default(),
                pts: vec![[10.0, -5.0], [10.0, 5.0]],
                class: RoadClass::Path,
                is_bridge: false,
            },
        ];
        for level in [15, 17, 19] {
            let frame = frame(level);
            let (interior, complete) = patches(&frame, &pack, &|_, _| 22.75);
            assert!(complete && !interior.is_empty());
            let mut split = FeaturePack::empty();
            split.roads = pack
                .roads
                .iter()
                .map(|r| Road {
                    profile: r.profile,
                    pts: r.pts.clone(),
                    class: r.class,
                    is_bridge: r.is_bridge,
                })
                .collect();
            split.roads[1].pts.insert(1, [10.0, 0.0]);
            let (explicit, complete) = patches(&frame, &split, &|_, _| 22.75);
            assert!(complete && interior.len() == explicit.len());
            for (a, b) in interior.iter().zip(&explicit) {
                assert_eq!(a.points, b.points);
            }
            assert!(
                interior.iter().flat_map(|p| p.points).any(|p| {
                    DVec2::new(p.x, p.z).distance(DVec2::new(10.0, 0.0)) < 3.0 && p.y < 23.1
                }),
                "ordinary-way junction needs a grounded ramp"
            );
        }
    }

    #[test]
    fn adaptive_road_endpoints_supply_plain_crossing_ramps() {
        let mut pack = FeaturePack::empty();
        pack.roads.push(Road {
            profile: Default::default(),
            pts: vec![[-120.0, 0.0], [120.0, 0.0]],
            class: RoadClass::Major,
            is_bridge: true,
        });
        let frame = frame(15);
        let (without_approaches, complete) = patches(&frame, &pack, &|_, _| 22.75);
        assert!(complete && !without_approaches.is_empty());
        pack.roads.extend([
            Road {
                profile: Default::default(),
                pts: vec![[-200.0, 0.0], [-120.0, 0.0]],
                class: RoadClass::Major,
                is_bridge: false,
            },
            Road {
                profile: Default::default(),
                pts: vec![[120.0, 0.0], [200.0, 0.0]],
                class: RoadClass::Major,
                is_bridge: false,
            },
        ]);
        let (with_approaches, complete) = patches(&frame, &pack, &|_, _| 22.75);
        assert!(complete);
        assert_eq!(with_approaches.len(), without_approaches.len());
        let mut lowered = false;
        let mut full_clearance = false;
        for (old, new) in without_approaches.iter().zip(&with_approaches) {
            assert_eq!(old.class, new.class);
            for (a, b) in old.points.into_iter().zip(new.points) {
                assert_eq!((a.x, a.z), (b.x, b.z));
                let distance = DVec2::new(b.x, b.z)
                    .distance(DVec2::new(-120.0, 0.0))
                    .min(DVec2::new(b.x, b.z).distance(DVec2::new(120.0, 0.0)));
                let expected = 22.75 + CLEARANCE_M * (distance / 48.0).clamp(0.0, 1.0);
                assert!((b.y - expected).abs() < 1e-9);
                assert!((a.y - (22.75 + CLEARANCE_M)).abs() < 1e-9);
                lowered |= b.y < a.y - 1.0;
                full_clearance |= (b.y - a.y).abs() < 1e-9;
            }
        }
        assert!(
            lowered,
            "matching adaptive approach ends must lower the deck"
        );
        assert!(
            full_clearance,
            "the middle of the crossing retains clearance"
        );
    }

    #[test]
    fn adaptive_duplicate_and_reversed_bridge_ways_have_one_deck() {
        let mut pack = FeaturePack::empty();
        pack.roads.push(road(0.0));
        let (baseline, complete) = patches(&frame(15), &pack, &|_, _| 12.0);
        assert!(complete && !baseline.is_empty());
        let mut reversed = road(0.0);
        reversed.pts.reverse();
        pack.roads.extend([road(0.0), reversed]);
        let (duplicated, complete) = patches(&frame(15), &pack, &|_, _| 12.0);
        assert!(complete);
        assert_eq!(baseline.len(), duplicated.len());
        for (a, b) in baseline.iter().zip(&duplicated) {
            assert_eq!(a.points, b.points);
            assert_eq!(a.class, b.class);
        }
    }
    #[test]
    fn incomplete_source_or_datum_cannot_publish_a_complete_bridge_result() {
        let mut pack = FeaturePack::empty();
        pack.roads.push(road(0.0));
        let (mesh, complete) = patches(&frame(17), &pack, &|_, _| f32::NAN);
        assert!(!complete);
        assert!(mesh.is_empty());
        pack.roads.push(Road {
            profile: Default::default(),
            pts: vec![[-100.0, 0.0], [f32::NAN, 20.0], [100.0, 0.0]],
            class: RoadClass::Major,
            is_bridge: true,
        });
        let (mesh, complete) = patches(&frame(17), &pack, &|_, _| 22.0);
        assert!(!complete);
        assert!(
            !mesh.is_empty(),
            "valid neighboring crossings remain visible"
        );
        pack.roads.pop();
        pack.roads.push(road(1.0e6));
        let (_, complete) = patches(&frame(17), &pack, &|_, _| 22.0);
        assert!(
            complete,
            "a valid source outside the cell is not a missing surface"
        );
    }
}
