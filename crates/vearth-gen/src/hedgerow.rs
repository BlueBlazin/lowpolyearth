//! Bounded, decorative field hedges following detailed source polygon edges.
//!
//! Regional priors authorize occasional enclosing hedges, never new field
//! boundaries. Candidate spacing is anchored in global Mercator coordinates;
//! clipping the same source edge into another terrain cell cannot restart it.

use std::collections::BTreeSet;

use glam::{DVec2, DVec3};
use vearth_core::{
    ellipsoid,
    frame::ChunkFrame,
    mesh::{MeshBuffers, SurfaceVertex, encode_surface_normal},
};
use vearth_data::{
    admin_regions::{AdminRegionAtlas, AdminRegionContext},
    vectors::{FeaturePack, FloraRegionKind, GroundKind},
};

use crate::{
    N, TerrainSurfaceData, material,
    tier0::{CellOverlay, GridProj},
    vegetation::{FoliageMotionClass, FoliageSeasonClass, FoliageTraits},
};

const RADIUS: f64 = 6_371_008.8;
const MERCATOR_RADIUS: f64 = 6_378_137.0;
const SPACING_M: f64 = 4.0;
const MIN_LEVEL: u8 = 16;
const MAX_CROWNS: usize = 96;
const TRIANGLES_PER_CROWN: usize = 24;

#[derive(Clone, Copy, Debug)]
struct HedgeStyle {
    probability: u64,
    height_m: f64,
    width_m: f64,
    color: [u8; 3],
    season: FoliageSeasonClass,
}

fn style(context: AdminRegionContext) -> Option<HedgeStyle> {
    use FoliageSeasonClass::{DeciduousYellow, EvergreenBroadleaf, EvergreenNeedle};
    let (probability, height_m, width_m, color, season) = match context.country_name {
        "United Kingdom" | "Ireland" | "Guernsey" | "Jersey" | "Isle of Man" => {
            (68, 2.0, 1.6, [73, 105, 57], DeciduousYellow)
        }
        "France"
            if [
                "Côtes-d'Armor",
                "Finistère",
                "Ille-et-Vilaine",
                "Morbihan",
                "Calvados",
                "Manche",
                "Orne",
                "Mayenne",
                "Sarthe",
                "Vendée",
                "Deux-Sèvres",
                "Loire-Atlantique",
                "Maine-et-Loire",
                "Creuse",
                "Haute-Vienne",
            ]
            .contains(&context.name) =>
        {
            (72, 2.35, 1.9, [77, 109, 62], DeciduousYellow)
        }
        "France"
            if [
                "Var",
                "Vaucluse",
                "Bouches-du-Rhône",
                "Gard",
                "Hérault",
                "Aude",
            ]
            .contains(&context.name) =>
        {
            (16, 1.45, 1.15, [92, 110, 72], EvergreenBroadleaf)
        }
        "France" | "Belgium" | "Netherlands" | "Luxembourg" => {
            (24, 1.8, 1.45, [80, 112, 62], DeciduousYellow)
        }
        "Germany" | "Austria" | "Switzerland" | "Denmark" => {
            (18, 1.8, 1.45, [77, 108, 61], DeciduousYellow)
        }
        "Italy" | "Spain" | "Portugal" | "Greece" => {
            (10, 1.4, 1.1, [91, 111, 72], EvergreenBroadleaf)
        }
        "New Zealand" => (28, 2.8, 1.5, [66, 97, 64], EvergreenNeedle),
        "Japan" => (12, 1.5, 1.25, [63, 107, 65], EvergreenBroadleaf),
        _ => return None,
    };
    Some(HedgeStyle {
        probability,
        height_m,
        width_m,
        color,
        season,
    })
}

fn hash(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn latlon(en: DVec2, origin: (f64, f64)) -> (f64, f64) {
    (
        origin.0 + en.y / RADIUS,
        origin.1 + en.x / (RADIUS * origin.0.cos()),
    )
}

fn mercator(en: DVec2, origin: (f64, f64)) -> DVec2 {
    let (lat, lon) = latlon(en, origin);
    DVec2::new(
        lon * MERCATOR_RADIUS,
        (std::f64::consts::FRAC_PI_4 + lat * 0.5).tan().ln() * MERCATOR_RADIUS,
    )
}

fn from_mercator(point: DVec2, origin: (f64, f64)) -> DVec2 {
    let lat = 2.0 * (point.y / MERCATOR_RADIUS).exp().atan() - std::f64::consts::FRAC_PI_2;
    DVec2::new(
        (point.x / MERCATOR_RADIUS - origin.1) * RADIUS * origin.0.cos(),
        (lat - origin.0) * RADIUS,
    )
}

/// Ring fragments may close along source-tile borders or their MVT buffer.
/// Such straight closures are not evidence of a real agricultural boundary.
fn tile_closure(a: DVec2, b: DVec2) -> bool {
    let world = std::f64::consts::TAU * MERCATOR_RADIUS;
    for axis in 0..2 {
        if (a[axis] - b[axis]).abs() > 0.7 {
            continue;
        }
        for zoom in 7..=16 {
            let tile = world / f64::from(1_u32 << zoom);
            let offset = ((a[axis] + world * 0.5) / tile).rem_euclid(1.0);
            // Tile edges and conventional 8/16/64/80/128/256-unit buffers.
            for buffer in [0.0, 8.0, 16.0, 64.0, 80.0, 128.0, 256.0] {
                let fraction = buffer / 4096.0;
                if (offset - fraction)
                    .abs()
                    .min((offset - (1.0 - fraction)).abs())
                    * tile
                    < 1.0
                {
                    return true;
                }
            }
        }
    }
    false
}

fn inside(point: DVec2, ring: impl Iterator<Item = DVec2>) -> bool {
    let mut points = ring;
    let Some(first) = points.next() else {
        return false;
    };
    let mut result = false;
    let mut a = first;
    for b in points.chain(std::iter::once(first)) {
        if (a.y > point.y) != (b.y > point.y)
            && point.x < (b.x - a.x) * (point.y - a.y) / (b.y - a.y) + a.x
        {
            result = !result;
        }
        a = b;
    }
    result
}

fn segment_distance_squared(point: DVec2, a: DVec2, b: DVec2) -> f64 {
    let d = b - a;
    (point - (a + d * ((point - a).dot(d) / d.length_squared().max(1.0e-12)).clamp(0.0, 1.0)))
        .length_squared()
}

fn polygon_gap(point: DVec2, rings: &[Vec<[f32; 2]>], clearance: f64) -> bool {
    let mut within = false;
    for ring in rings {
        within ^= inside(
            point,
            ring.iter().map(|p| DVec2::from_array(p.map(f64::from))),
        );
        if ring
            .iter()
            .zip(ring.iter().cycle().skip(1))
            .take(ring.len())
            .any(|(a, b)| {
                segment_distance_squared(
                    point,
                    DVec2::from_array(a.map(f64::from)),
                    DVec2::from_array(b.map(f64::from)),
                ) < clearance * clearance
            })
        {
            return true;
        }
    }
    within
}

fn physical_gap(point: DVec2, pack: &FeaturePack, clearance: f64) -> bool {
    if pack
        .water
        .iter()
        .any(|water| polygon_gap(point, &water.rings, clearance))
        || pack
            .buildings
            .iter()
            .any(|building| polygon_gap(point, &building.rings, clearance))
    {
        return true;
    }
    pack.roads
        .iter()
        .filter(|road| !road.is_bridge)
        .any(|road| {
            let radius = f64::from(road.width_m()) * 0.5 + clearance;
            road.pts.windows(2).any(|edge| {
                segment_distance_squared(
                    point,
                    DVec2::from_array(edge[0].map(f64::from)),
                    DVec2::from_array(edge[1].map(f64::from)),
                ) < radius * radius
            })
        })
}

fn clipped_interval(a: DVec2, b: DVec2) -> Option<(f64, f64)> {
    let mut lo: f64 = 0.0;
    let mut hi: f64 = 1.0;
    for axis in 0..2 {
        let delta = b[axis] - a[axis];
        if delta.abs() < 1.0e-9 {
            if !(0.0..N as f64).contains(&a[axis]) {
                return None;
            }
        } else {
            let t0 = -a[axis] / delta;
            let t1 = (N as f64 - a[axis]) / delta;
            lo = lo.max(t0.min(t1));
            hi = hi.min(t0.max(t1));
        }
    }
    (lo < hi).then_some((lo, hi))
}

fn edge_slots(a: DVec2, b: DVec2, spacing: f64, interval: (f64, f64)) -> Vec<DVec2> {
    let mut direction = (b - a).normalize_or_zero();
    if direction.x < 0.0 || (direction.x == 0.0 && direction.y < 0.0) {
        direction = -direction;
    }
    if direction == DVec2::ZERO {
        return Vec::new();
    }
    let start = (a + (b - a) * interval.0).dot(direction);
    let end = (a + (b - a) * interval.1).dot(direction);
    let lo = (start.min(end) / spacing).ceil() as i64;
    let hi = (start.max(end) / spacing).floor() as i64;
    (lo..=hi)
        .take(MAX_CROWNS * 4)
        .map(|slot| a + direction * (slot as f64 * spacing - a.dot(direction)))
        .collect()
}

fn height(terrain: &TerrainSurfaceData, point: DVec2) -> f64 {
    let x = point.x.clamp(0.0, N as f64 - 1.0e-6);
    let z = point.y.clamp(0.0, N as f64 - 1.0e-6);
    let (ix, iz) = (x.floor() as usize, z.floor() as usize);
    let (fx, fz) = (x.fract(), z.fract());
    let sample = |dx, dz| f64::from(terrain.heights_m[(iz + dz) * (N + 1) + ix + dx]);
    let (h00, h10, h01, h11) = (sample(0, 0), sample(1, 0), sample(0, 1), sample(1, 1));
    if fz <= fx {
        h00 + (h10 - h00) * fx + (h11 - h10) * fz
    } else {
        h00 + (h11 - h01) * fx + (h01 - h00) * fz
    }
}

fn crown(
    mesh: &mut MeshBuffers,
    frame: &ChunkFrame,
    root: DVec3,
    along: DVec3,
    up: DVec3,
    style: HedgeStyle,
    seed: u64,
) {
    let across = up.cross(along).normalize();
    let variation = (hash(seed) & 255) as f64 / 255.0;
    let height = style.height_m * (0.82 + variation * 0.30);
    let width = style.width_m * (0.88 + variation * 0.22);
    let to_local = |a, y, c| frame.ecef_to_local(root + along * a + up * y + across * c);
    let rings: [[DVec3; 6]; 2] = std::array::from_fn(|layer| {
        std::array::from_fn(|side| {
            let angle = (side as f64 + 0.10 * variation) * std::f64::consts::TAU / 6.0;
            let scale = if layer == 0 { 0.78 } else { 1.0 };
            to_local(
                angle.cos() * SPACING_M * 0.64 * scale,
                height * if layer == 0 { 0.22 } else { 0.62 },
                angle.sin() * width * 0.5 * scale,
            )
        })
    });
    let bottom = to_local(0.0, 0.06, 0.0);
    let top = to_local(SPACING_M * (variation - 0.5) * 0.22, height, 0.0);
    let material = material::foliage(FoliageTraits {
        season: style.season,
        motion: FoliageMotionClass::Stiff,
    });
    let color = style
        .color
        .map(|channel| (f64::from(channel) * (0.94 + variation * 0.12)).round() as u8);
    let mut triangle = |a: DVec3, b: DVec3, c: DVec3| {
        let normal = (b - a).cross(c - a).normalize().as_vec3();
        let packed =
            encode_surface_normal(2 | (3 << 3) | (u32::from(material) << 5), normal.to_array());
        let first = mesh.vertices.len() as u32;
        mesh.vertices.extend([a, b, c].map(|point| SurfaceVertex {
            pos: point.to_array().map(|v| v as f32),
            face_ao: packed,
            rgba: [color[0], color[1], color[2], 255],
        }));
        mesh.indices.extend([first, first + 1, first + 2]);
    };
    for i in 0..6 {
        let j = (i + 1) % 6;
        triangle(bottom, rings[0][j], rings[0][i]);
        triangle(rings[0][i], rings[0][j], rings[1][j]);
        triangle(rings[0][i], rings[1][j], rings[1][i]);
        triangle(rings[1][i], rings[1][j], top);
    }
}

pub(crate) fn build(
    frame: &ChunkFrame,
    pack: &FeaturePack,
    overlay: &CellOverlay,
    terrain: &TerrainSurfaceData,
    exclusions: &[Vec<DVec2>],
    landmark_hits: &[Vec<&'static crate::landmarks::Landmark>],
) -> MeshBuffers {
    let mut mesh = MeshBuffers::default();
    if frame.addr.cell.level < MIN_LEVEL {
        return mesh;
    }
    let origin = frame.column_latlon(0.5, 0.5);
    let projection = GridProj::of(frame, origin.0, origin.1);
    let mut occupied = BTreeSet::new();
    let mut fields: Vec<_> = pack
        .flora
        .iter()
        .filter(|field| {
            matches!(
                field.kind,
                FloraRegionKind::Farmland | FloraRegionKind::Meadow
            )
        })
        .collect();
    fields.sort_by_key(|field| field.seed);
    'fields: for field in fields {
        let location = crate::flora::field_context_location(
            field.location_rad.unwrap_or([origin.0, origin.1]),
        );
        let Some(style) = AdminRegionAtlas::bundled()
            .context_at(location[0], location[1])
            .and_then(style)
        else {
            continue;
        };
        if hash(field.seed ^ 0x6865_6467_6573) % 100 >= style.probability {
            continue;
        }
        let metric_scale = location[0].cos().max(0.1);
        for ring in &field.rings {
            for (&start, &end) in ring
                .iter()
                .zip(ring.iter().cycle().skip(1))
                .take(ring.len())
            {
                let (a, b) = (
                    DVec2::from_array(start.map(f64::from)),
                    DVec2::from_array(end.map(f64::from)),
                );
                let (ma, mb) = (mercator(a, origin), mercator(b, origin));
                if tile_closure(ma, mb) || (a - b).length_squared() < 16.0 {
                    continue;
                }
                let ca = DVec2::from_array(projection.to_cols(start).map(f64::from));
                let cb = DVec2::from_array(projection.to_cols(end).map(f64::from));
                let Some(interval) = clipped_interval(ca, cb) else {
                    continue;
                };
                for slot in edge_slots(ma, mb, SPACING_M / metric_scale, interval) {
                    let point = from_mercator(slot, origin);
                    let columns = DVec2::from_array(
                        projection
                            .to_cols(point.to_array().map(|v| v as f32))
                            .map(f64::from),
                    );
                    if !(0.0..N as f64).contains(&columns.x)
                        || !(0.0..N as f64).contains(&columns.y)
                    {
                        continue;
                    }
                    let (x, z) = (columns.x as usize, columns.y as usize);
                    // Tree suppression is independent of whether a landmark
                    // retains source roads through its replacement footprint.
                    if !landmark_hits.is_empty()
                        && landmark_hits[z * N + x]
                            .iter()
                            .any(|landmark| landmark.suppress.trees)
                    {
                        continue;
                    }
                    let col = &overlay.cols[z][x];
                    if col.water
                        || col.ocean
                        || col.building.is_some()
                        || col.road.is_some()
                        || matches!(
                            col.ground,
                            Some(
                                GroundKind::Urban
                                    | GroundKind::Paved
                                    | GroundKind::Farmyard
                                    | GroundKind::RegionalUrban
                                    | GroundKind::AirportArea
                                    | GroundKind::RunwayArea
                            )
                        )
                        || physical_gap(point, pack, style.width_m * 0.5 + 1.2)
                        || exclusions
                            .iter()
                            .any(|ring| inside(columns, ring.iter().copied()))
                    {
                        continue;
                    }
                    // Fragment overlap must not create an internal hedge in one field.
                    if pack
                        .flora
                        .iter()
                        .filter(|other| other.seed == field.seed && !std::ptr::eq(*other, field))
                        .any(|other| {
                            other.rings.iter().fold(false, |within, ring| {
                                within
                                    ^ inside(
                                        point,
                                        ring.iter().map(|p| DVec2::from_array(p.map(f64::from))),
                                    )
                            })
                        })
                    {
                        continue;
                    }
                    let address = (
                        (slot.x * metric_scale / 3.0).round() as i64,
                        (slot.y * metric_scale / 3.0).round() as i64,
                    );
                    if !occupied.insert(address) {
                        continue;
                    }
                    let seed =
                        hash(field.seed ^ address.0 as u64 ^ (address.1 as u64).rotate_left(29));
                    // Small persistent access gaps interrupt occasional hedge runs.
                    if hash(seed ^ 0x6761_7465) % 31 == 0 {
                        continue;
                    }
                    let (lat, lon) = latlon(point, origin);
                    let root =
                        ellipsoid::geodetic_to_ecef(lat, lon, height(terrain, columns) - 0.10);
                    let up = ellipsoid::geodetic_normal(root);
                    let direction = (b - a).normalize();
                    let east = DVec3::new(-lon.sin(), lon.cos(), 0.0);
                    let north = up.cross(east).normalize();
                    crown(
                        &mut mesh,
                        frame,
                        root,
                        (east * direction.x + north * direction.y).normalize(),
                        up,
                        style,
                        seed,
                    );
                    if mesh.indices.len() / (TRIANGLES_PER_CROWN * 3) >= MAX_CROWNS {
                        break 'fields;
                    }
                }
            }
        }
    }
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regional_hedges_are_strongest_in_bocage_and_absent_in_open_prairies() {
        let context = |country_name, name| AdminRegionContext {
            id: 0,
            country_name,
            name,
        };
        let bocage = style(context("France", "Manche")).unwrap();
        assert!(bocage.probability > style(context("France", "Marne")).unwrap().probability);
        assert!(bocage.width_m > style(context("Spain", "Madrid")).unwrap().width_m);
        assert!(style(context("United States of America", "Iowa")).is_none());
    }

    #[test]
    fn source_edge_slots_survive_clipping_and_reversal_without_border_rows() {
        let a = DVec2::new(130.0, 45.0);
        let b = DVec2::new(210.0, 75.0);
        let whole = edge_slots(a, b, 4.0, (0.0, 1.0));
        let left = edge_slots(a, b, 4.0, (0.0, 0.5));
        let right = edge_slots(a, b, 4.0, (0.5, 1.0));
        assert_eq!(whole.len(), left.len() + right.len());
        assert!(
            left.into_iter()
                .chain(right)
                .all(|p| whole.iter().any(|q| p.distance(*q) < 1.0e-8))
        );
        let reversed = edge_slots(b, a, 4.0, (0.0, 1.0));
        assert_eq!(whole.len(), reversed.len());
        assert!(
            whole
                .iter()
                .zip(reversed)
                .all(|(a, b)| a.distance(b) < 1.0e-8)
        );
        assert!(clipped_interval(DVec2::new(-2.0, -1.0), DVec2::new(-2.0, 33.0)).is_none());
        assert!(tile_closure(DVec2::new(0.0, 13.0), DVec2::new(0.0, 180.0)));
    }

    #[test]
    fn road_crossings_leave_hedge_gaps_and_geometry_is_bounded() {
        use vearth_data::{
            road_profile::RoadProfile,
            vectors::{Road, RoadClass},
        };
        let mut pack = FeaturePack::empty();
        pack.roads.push(Road {
            profile: RoadProfile::default(),
            pts: vec![[-10.0, 0.0], [10.0, 0.0]],
            class: RoadClass::Minor,
            is_bridge: false,
        });
        assert!(physical_gap(DVec2::new(0.0, 1.0), &pack, 2.0));
        assert!(!physical_gap(DVec2::new(0.0, 20.0), &pack, 2.0));
        assert!(MAX_CROWNS * TRIANGLES_PER_CROWN <= 2304);
        assert!(
            edge_slots(DVec2::ZERO, DVec2::new(1.0e8, 0.0), 4.0, (0.0, 1.0)).len()
                <= MAX_CROWNS * 4
        );
    }
    #[test]
    fn rendered_hedges_follow_source_edges_and_respect_cell_budget_and_water() {
        use vearth_core::cell::{CellId, ChunkAddr};
        use vearth_data::vectors::{FloraRegion, Water};
        let center = ellipsoid::geodetic_to_ecef(48.4_f64.to_radians(), -1.6_f64.to_radians(), 0.0);
        let cell = CellId::from_dir(center, 17);
        let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
        let mut pack = FeaturePack::empty();
        let seed = (0..1000)
            .find(|seed| hash(seed ^ 0x6865_6467_6573) % 100 < 5)
            .unwrap();
        pack.flora.push(FloraRegion {
            seed,
            kind: FloraRegionKind::Farmland,
            location_rad: Some([48.4_f64.to_radians(), -1.6_f64.to_radians()]),
            rings: vec![vec![
                [-25.0, -20.0],
                [25.0, -15.0],
                [23.0, 22.0],
                [-26.0, 18.0],
            ]],
        });
        let overlay = CellOverlay::empty();
        let terrain = TerrainSurfaceData {
            heights_m: Box::new([50.0; (N + 1) * (N + 1)]),
            shading: None,
            source_columns: [u32::MAX; N],
        };
        let mesh = build(&frame, &pack, &overlay, &terrain, &[], &[]);
        assert!(!mesh.indices.is_empty());
        assert!(mesh.indices.len() <= MAX_CROWNS * TRIANGLES_PER_CROWN * 3);
        assert!(
            mesh.vertices
                .iter()
                .all(|vertex| vertex.pos.iter().all(|p| p.is_finite())
                    && material::is_foliage(((vertex.face_ao >> 5) & 255) as u8))
        );
        assert_eq!(
            mesh.vertices,
            build(&frame, &pack, &overlay, &terrain, &[], &[]).vertices
        );
        let landmark = crate::landmarks::LANDMARKS
            .iter()
            .find(|landmark| landmark.suppress.trees && !landmark.suppress.roads)
            .expect("catalog contains landmarks preserving source roads");
        let hits = vec![vec![landmark]; N * N];
        assert!(
            build(&frame, &pack, &overlay, &terrain, &[], &hits)
                .indices
                .is_empty(),
            "tree suppression must remain effective without road exclusions"
        );
        pack.water.push(Water::ocean(vec![vec![
            [-1000.0, -1000.0],
            [1000.0, -1000.0],
            [1000.0, 1000.0],
            [-1000.0, 1000.0],
        ]]));
        assert!(
            build(&frame, &pack, &overlay, &terrain, &[], &[])
                .indices
                .is_empty()
        );
        pack.water.clear();
        // A field enclosing the entire cell has no owned boundary inside it.
        pack.flora[0].rings = vec![vec![
            [-1000.0, -1000.0],
            [1000.0, -1000.0],
            [1000.0, 1000.0],
            [-1000.0, 1000.0],
        ]];
        assert!(
            build(&frame, &pack, &overlay, &terrain, &[], &[])
                .indices
                .is_empty()
        );
    }
}
