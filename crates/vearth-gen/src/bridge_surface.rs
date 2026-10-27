//! Continuous visible structures derived from the canonical collision profile.

use super::{BridgeKind, Profile, build_profile, closest_path_hit, interpolate};
use crate::{
    N, SurfaceAppearance,
    building_surface::{source_polygon_boundary, source_polygon_cells},
    road_surface::offset_polyline,
    tier0::GridProj,
};
use glam::{DVec2, DVec3};
use vearth_core::{
    frame::ChunkFrame,
    mesh::{MeshBuffers, SurfaceVertex, encode_surface_normal},
};
use vearth_data::{
    bridge_pack::{BridgeMaterial, BridgeStructure, CanonicalBridge},
    vectors::{FeaturePack, Rings, RoadClass},
};

fn road_appearance(class: RoadClass, jitter: f32, scale: f32) -> SurfaceAppearance {
    let color = crate::road_appearance(class, jitter, scale);
    SurfaceAppearance {
        material: color.material,
        tint: color.tint,
    }
}
fn support_appearance(jitter: f32, scale: f32) -> SurfaceAppearance {
    let color = crate::bridge_support_appearance(jitter, scale);
    SurfaceAppearance {
        material: color.material,
        tint: color.tint,
    }
}
fn structure_appearance(kind: BridgeKind, jitter: f32, scale: f32) -> SurfaceAppearance {
    let color = crate::bridge_structure_appearance(kind, jitter, scale);
    SurfaceAppearance {
        material: color.material,
        tint: color.tint,
    }
}

const MAX_TRIANGLES: usize = 196_608;
const EPS: f64 = 1.0e-8;

pub(crate) struct BridgeGeometry {
    pub mesh: MeshBuffers,
    pub collision: Vec<crate::collision::CollisionTriangle>,
    pub complete: bool,
}

/// Source-shaped closed decks and structural members. Collision is derived
/// from these exact final triangles in the owning cell's metric frame.
pub(crate) fn generate(
    frame: &ChunkFrame,
    pack: &FeaturePack,
    exclusions: &[Vec<DVec2>],
    elev_en: &dyn Fn(f64, f64) -> f32,
) -> Option<BridgeGeometry> {
    generate_with_detail(frame, pack, exclusions, elev_en, 8.0, true)
}

fn generate_with_detail(
    frame: &ChunkFrame,
    pack: &FeaturePack,
    exclusions: &[Vec<DVec2>],
    elev_en: &dyn Fn(f64, f64) -> f32,
    step_m: f64,
    ornament: bool,
) -> Option<BridgeGeometry> {
    if frame.addr.cell.level < super::BRIDGE_MIN_LEVEL
        || ((!pack.bridge_authority || pack.bridges.is_empty())
            && !super::fallback::has_source(pack))
    {
        return None;
    }
    let (lat, lon) = frame.column_latlon(0.5, 0.5);
    let projection = GridProj::of(frame, lat, lon);
    let canonical_ground = |east: f64, north: f64| {
        crate::landmarks::corrected_bridge_ground(
            lat + north / vearth_core::ellipsoid::MEAN_RADIUS,
            lon + east / (vearth_core::ellipsoid::MEAN_RADIUS * lat.cos()),
            elev_en(east, north),
        )
    };
    let mut emitter = Emitter::new(frame, &projection, exclusions);
    emitter.step_m = step_m;
    emitter.ornament = ornament;
    let mut bridges = pack
        .bridges
        .iter()
        .filter(|_| pack.bridge_authority)
        .collect::<Vec<_>>();
    bridges.sort_by_key(|bridge| bridge.id);
    bridges.dedup_by_key(|bridge| bridge.id);
    let mut complete = true;
    let mut models = Vec::new();
    for bridge in bridges {
        if !valid_source(bridge) {
            complete = false;
            continue;
        }
        let Some(profile) = build_profile(bridge, &projection, &canonical_ground) else {
            complete = false;
            continue;
        };
        if profile
            .station_deck
            .iter()
            .chain(&profile.station_terrain)
            .any(|h| !h.is_finite())
        {
            complete = false;
            continue;
        }
        models.push((bridge, profile));
    }
    // Decks have priority over optional structural detail throughout a cell.
    for (bridge, profile) in &models {
        let model = Model {
            source: bridge,
            profile,
            projection: &projection,
        };
        let (v, i) = (emitter.mesh.vertices.len(), emitter.mesh.indices.len());
        if deck(&model, &mut emitter).is_none() {
            emitter.mesh.vertices.truncate(v);
            emitter.mesh.indices.truncate(i);
            emitter.failed = false;
            let mut plain = (*bridge).clone();
            plain.deck_outline.clear();
            let plain_model = Model {
                source: &plain,
                profile,
                projection: &projection,
            };
            if deck(&plain_model, &mut emitter).is_none() {
                emitter.mesh.vertices.truncate(v);
                emitter.mesh.indices.truncate(i);
                emitter.failed = false;
                complete = false;
            }
        }
    }
    let (fallback, fallback_complete) = super::fallback::patches(frame, pack, elev_en);
    complete &= fallback_complete;
    let mut edges = std::collections::BTreeMap::new();
    for patch in &fallback {
        let surface = if patch.class == RoadClass::Rail {
            support_appearance(0.5, 1.0)
        } else {
            patch.appearance
        };
        emitter.face(&patch.points, surface);
        let bottom = patch
            .points
            .map(|p| p - DVec3::Y * f64::from(super::fallback::THICKNESS_M));
        emitter.face(
            &[bottom[2], bottom[1], bottom[0]],
            support_appearance(0.5, 1.0),
        );
        for i in 0..3 {
            let a = patch.points[i];
            let b = patch.points[(i + 1) % 3];
            let key = |p: DVec3| {
                (
                    (p.x * 1000.0).round() as i64,
                    (p.y * 1000.0).round() as i64,
                    (p.z * 1000.0).round() as i64,
                )
            };
            let ka = key(a);
            let kb = key(b);
            let k = if ka < kb { (ka, kb) } else { (kb, ka) };
            let entry = edges.entry(k).or_insert((0, a, b));
            entry.0 += 1;
        }
    }
    for (_, (count, a, b)) in edges {
        if count != 1 {
            continue;
        }
        let down = DVec3::Y * f64::from(super::fallback::THICKNESS_M);
        emitter.face(&[a, b, b - down, a - down], support_appearance(0.5, 1.0));
    }
    if emitter.failed {
        complete = false;
        emitter.failed = false;
    }
    crate::built_surface::compact_mesh(&mut emitter.mesh);
    let budget = crate::built_surface::BRIDGE_BASE_BUDGET_BYTES;
    if ornament && crate::built_surface::mesh_bytes(&emitter.mesh) > budget {
        return generate_with_detail(frame, pack, exclusions, elev_en, 128.0, false);
    }
    // Travel contact needs the continuous closed decks. Optional arches,
    // suspension cables and towers remain visual detail instead of adding
    // thousands of obstacle triangles to the streamed collider.
    let collision = crate::collision::triangles_from_mesh(&emitter.mesh, frame);
    for (bridge, profile) in &models {
        let model = Model {
            source: bridge,
            profile,
            projection: &projection,
        };
        let mut detail = Emitter::new(frame, &projection, exclusions);
        if structure(&model, &mut detail, elev_en).is_none() {
            continue;
        }
        crate::built_surface::compact_mesh(&mut detail.mesh);
        if crate::built_surface::mesh_bytes(&emitter.mesh)
            + crate::built_surface::mesh_bytes(&detail.mesh)
            > budget
        {
            continue;
        }
        let offset = emitter.mesh.vertices.len() as u32;
        emitter.mesh.vertices.extend(detail.mesh.vertices);
        emitter
            .mesh
            .indices
            .extend(detail.mesh.indices.into_iter().map(|i| i + offset));
    }
    if emitter.mesh.indices.is_empty() && complete {
        return None;
    }
    // The shared cell safety ceiling bounds mandatory geometry. Optional
    // detail cannot force a legitimate mapped crossing out of the scene.
    complete &= crate::built_surface::mesh_bytes(&emitter.mesh) <= 128 * 1024 * 1024;
    Some(BridgeGeometry {
        mesh: emitter.mesh,
        collision,
        complete,
    })
}

pub(super) fn valid_source(bridge: &CanonicalBridge) -> bool {
    let length: f64 = bridge
        .alignment
        .windows(2)
        .map(|pair| {
            DVec2::from_array(pair[0].map(f64::from))
                .distance(DVec2::from_array(pair[1].map(f64::from)))
        })
        .sum();
    (2..=4097).contains(&bridge.alignment.len())
        && bridge.deck_outline.len() <= 512
        && bridge
            .alignment
            .iter()
            .chain(&bridge.deck_outline)
            .flatten()
            .all(|v| v.is_finite())
        && bridge.alignment.windows(2).all(|p| {
            DVec2::from_array(p[0].map(f64::from))
                .distance_squared(DVec2::from_array(p[1].map(f64::from)))
                > EPS
        })
        && (0.1..=2000.0).contains(&bridge.deck_width_m)
        && (0.1..=2000.0).contains(&bridge.road_width_m)
        && (8.0..=100_000.0).contains(&length)
}

struct Model<'a> {
    source: &'a CanonicalBridge,
    profile: &'a Profile,
    projection: &'a GridProj,
}

impl Model<'_> {
    fn point(&self, s: f64, lateral: f64, height: f64) -> DVec3 {
        let total = *self.profile.cumulative.last().unwrap();
        let s = s.clamp(0.0, total);
        let p = self.center(s);
        let a = self.center((s - 0.05).max(0.0));
        let b = self.center((s + 0.05).min(total));
        let direction = (b - a).normalize();
        let p = p + DVec2::new(-direction.y, direction.x) * lateral;
        DVec3::new(p.x, height, p.y)
    }

    fn center(&self, s: f64) -> DVec2 {
        let index = self
            .profile
            .cumulative
            .partition_point(|&v| v <= s)
            .saturating_sub(1)
            .min(self.source.alignment.len() - 2);
        let a = DVec2::from_array(self.source.alignment[index].map(f64::from));
        let b = DVec2::from_array(self.source.alignment[index + 1].map(f64::from));
        let lo = self.profile.cumulative[index];
        let hi = self.profile.cumulative[index + 1];
        a.lerp(b, ((s - lo) / (hi - lo)).clamp(0.0, 1.0))
    }

    fn station(&self, en: DVec2) -> f64 {
        let p = self.projection.to_cols(en.as_vec2().to_array());
        closest_path_hit(self.profile, p, self.projection.step_m)
            .unwrap()
            .s
    }

    fn height(&self, s: f64) -> f64 {
        f64::from(interpolate(
            &self.profile.station_s,
            &self.profile.station_deck,
            s,
        ))
    }

    fn thickness(&self) -> f64 {
        if self.profile.kind == BridgeKind::Beam {
            ((self.profile.water_end - self.profile.water_start) / 25.0).clamp(1.0, 3.5)
        } else {
            1.2
        }
    }

    fn structure_appearance(&self, support: bool) -> SurfaceAppearance {
        let fallback = if support {
            support_appearance(0.5, 1.0)
        } else {
            structure_appearance(self.profile.kind, 0.5, 1.0)
        };
        match self.source.material {
            BridgeMaterial::Unknown => fallback,
            BridgeMaterial::Stone => SurfaceAppearance {
                material: crate::material::PAVED,
                tint: [169, 158, 137],
            },
            BridgeMaterial::Brick => SurfaceAppearance {
                material: crate::material::PAVED,
                tint: [151, 104, 81],
            },
            BridgeMaterial::Concrete => support_appearance(0.5, 1.0),
            BridgeMaterial::Metal => SurfaceAppearance {
                material: crate::material::STEEL,
                tint: [120, 129, 135],
            },
            BridgeMaterial::Wood => SurfaceAppearance {
                material: crate::material::TIMBER_WALL,
                tint: [117, 88, 61],
            },
        }
    }

    fn deck_edges(&self, s: f64) -> (f64, f64) {
        let half = self.profile.deck_width * 0.5;
        if self.source.deck_outline.len() < 3 {
            return (-half, half);
        }
        let center = self.center(s);
        let side_point = self.point(s, 1.0, 0.0);
        let side = DVec2::new(side_point.x, side_point.z) - center;
        let mut crossings = Vec::new();
        for pair in self
            .source
            .deck_outline
            .iter()
            .zip(self.source.deck_outline.iter().cycle().skip(1))
            .take(self.source.deck_outline.len())
        {
            let a = DVec2::from_array(pair.0.map(f64::from));
            let b = DVec2::from_array(pair.1.map(f64::from));
            let delta = b - a;
            let denominator = side.perp_dot(delta);
            if denominator.abs() < EPS {
                continue;
            }
            let t = (a - center).perp_dot(side) / denominator;
            if (-EPS..=1.0 + EPS).contains(&t) {
                crossings.push((a - center).perp_dot(delta) / denominator);
            }
        }
        crossings.sort_by(f64::total_cmp);
        let left = crossings.iter().copied().filter(|x| *x <= 0.0).next_back();
        let right = crossings.iter().copied().find(|x| *x >= 0.0);
        match (left, right) {
            (Some(a), Some(b)) if b - a > 0.5 => (a, b),
            _ => (-half, half),
        }
    }

    fn stations(&self, lo: f64, hi: f64, step_m: f64) -> Vec<f64> {
        let mut stations = vec![lo, hi];
        stations.extend(
            self.profile
                .station_s
                .iter()
                .copied()
                .filter(|&s| s > lo && s < hi),
        );
        stations.extend(
            self.profile
                .cumulative
                .iter()
                .copied()
                .filter(|&s| s > lo && s < hi),
        );
        stations.sort_by(f64::total_cmp);
        stations.dedup_by(|a, b| (*a - *b).abs() < EPS);
        let mut result = Vec::new();
        for pair in stations.windows(2) {
            let n = ((pair[1] - pair[0]) / step_m).ceil().max(1.0) as usize;
            for i in 0..n {
                result.push(pair[0] + (pair[1] - pair[0]) * i as f64 / n as f64);
            }
        }
        result.push(hi);
        result
    }

    fn strip(&self, lo: f64, hi: f64, width: f64) -> Rings {
        let stations = std::iter::once(lo)
            .chain(
                self.profile
                    .cumulative
                    .iter()
                    .copied()
                    .filter(|&s| s > lo && s < hi),
            )
            .chain(std::iter::once(hi));
        let points: Vec<_> = stations.map(|s| self.center(s)).collect();
        let mut ring = offset_polyline(&points, -width * 0.5);
        ring.extend(offset_polyline(&points, width * 0.5).into_iter().rev());
        vec![ring.into_iter().map(|p| p.as_vec2().to_array()).collect()]
    }

    fn profile_cuts(&self, lo: f64, hi: f64) -> Vec<f64> {
        let start = self.profile.station_s.partition_point(|&s| s <= lo + EPS);
        let end = self.profile.station_s.partition_point(|&s| s < hi - EPS);
        self.profile.station_s[start.min(end)..end].to_vec()
    }

    fn deck_patches(&self, polygon: &[DVec2]) -> Vec<Vec<DVec2>> {
        let stations: Vec<_> = polygon.iter().map(|&p| self.station(p)).collect();
        let lo = stations.iter().copied().fold(f64::INFINITY, f64::min);
        let hi = stations.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let mut rest = polygon.to_vec();
        let mut patches = Vec::new();
        for cut in self.profile_cuts(lo, hi) {
            let before = self.clip_station(&rest, cut, -1.0);
            if before.len() >= 3 {
                patches.push(before);
            }
            rest = self.clip_station(&rest, cut, 1.0);
            if rest.len() < 3 {
                break;
            }
        }
        if rest.len() >= 3 {
            patches.push(rest);
        }
        patches
    }

    fn clip_station(&self, polygon: &[DVec2], cut: f64, sign: f64) -> Vec<DVec2> {
        let mut output = Vec::new();
        for (&a, &b) in polygon
            .iter()
            .zip(polygon.iter().cycle().skip(1))
            .take(polygon.len())
        {
            let da = (self.station(a) - cut) * sign;
            let db = (self.station(b) - cut) * sign;
            if da >= 0.0 {
                output.push(a);
            }
            if (da >= 0.0) != (db >= 0.0) {
                output.push(a.lerp(b, da / (da - db)));
            }
        }
        output
    }

    fn edge_points(&self, a: DVec2, b: DVec2, step_m: f64) -> Vec<DVec2> {
        let count = ((b - a).length() / step_m).ceil().max(1.0) as usize;
        let mut points = Vec::new();
        for i in 0..count {
            let p = a.lerp(b, i as f64 / count as f64);
            let q = a.lerp(b, (i + 1) as f64 / count as f64);
            let (sp, sq) = (self.station(p), self.station(q));
            let mut cuts = self.profile_cuts(sp.min(sq), sp.max(sq));
            if sp > sq {
                cuts.reverse();
            }
            points.push(p);
            for cut in cuts {
                points.push(p.lerp(q, (cut - sp) / (sq - sp)));
            }
        }
        points.push(b);
        points
    }
}

fn deck(model: &Model<'_>, emitter: &mut Emitter<'_>) -> Option<()> {
    let profile = model.profile;
    let total = *profile.cumulative.last()?;
    let mut domains = Vec::new();
    if profile.bridge_start > EPS {
        domains.push(model.strip(0.0, profile.bridge_start, profile.road_width));
    }
    if model.source.deck_outline.len() >= 3 {
        domains.push(vec![model.source.deck_outline.clone()]);
    } else {
        domains.push(model.strip(
            profile.bridge_start,
            profile.bridge_end.min(total),
            profile.deck_width,
        ));
    }
    if profile.bridge_end < total - EPS {
        domains.push(model.strip(profile.bridge_end, total, profile.road_width));
    }
    let concrete = model.structure_appearance(true);
    let surface = if model.source.class == RoadClass::Rail {
        concrete
    } else {
        road_appearance(model.source.class, 0.5, 1.0)
    };
    let steel = model.structure_appearance(
        profile.kind == BridgeKind::Arch && profile.tagged_structure != BridgeStructure::TiedArch,
    );
    let thickness = model.thickness();
    for rings in &domains {
        // Profile stations split the surface's linear elevation intervals;
        // source concavity stays intact and cell cuts never acquire fascia.
        for cell in source_polygon_cells(rings, emitter.step_m)? {
            for patch in model.deck_patches(&cell) {
                let top: Vec<_> = patch
                    .into_iter()
                    .map(|p| DVec3::new(p.x, model.height(model.station(p)), p.y))
                    .collect();
                emitter.face(&top.iter().copied().rev().collect::<Vec<_>>(), surface);
                let bottom: Vec<_> = top.into_iter().map(|p| p - DVec3::Y * thickness).collect();
                emitter.face(&bottom, concrete);
            }
        }
        for [a, b] in source_polygon_boundary(rings)? {
            for pair in model.edge_points(a, b, emitter.step_m).windows(2) {
                let (p, q) = (pair[0], pair[1]);
                let h0 = model.height(model.station(p));
                let h1 = model.height(model.station(q));
                emitter.face(
                    &[
                        DVec3::new(p.x, h0, p.y),
                        DVec3::new(q.x, h1, q.y),
                        DVec3::new(q.x, h1 - thickness, q.y),
                        DVec3::new(p.x, h0 - thickness, p.y),
                    ],
                    concrete,
                );
                let middle = (p + q) * 0.5;
                let s = model.station(middle);
                let direction =
                    model.center((s + 0.1).min(total)) - model.center((s - 0.1).max(0.0));
                if emitter.ornament
                    && model.source.class != RoadClass::Rail
                    && (profile.bridge_start + 0.25..profile.bridge_end - 0.25).contains(&s)
                    && (q - p).normalize().dot(direction.normalize()).abs() > 0.45
                {
                    emitter.beam(
                        DVec3::new(p.x, h0 + 0.58, p.y),
                        DVec3::new(q.x, h1 + 0.58, q.y),
                        0.24,
                        1.16,
                        steel,
                    );
                }
            }
        }
    }
    if emitter.ornament && model.source.class == RoadClass::Rail {
        let rail = road_appearance(RoadClass::Rail, 0.5, 1.0);
        for pair in model.stations(0.0, total, 4.0).windows(2) {
            for side in [-0.7175, 0.7175] {
                emitter.beam(
                    model.point(pair[0], side, model.height(pair[0]) + 0.09),
                    model.point(pair[1], side, model.height(pair[1]) + 0.09),
                    0.15,
                    0.16,
                    rail,
                );
            }
        }
    }
    (!emitter.failed).then_some(())
}

fn structure(
    model: &Model<'_>,
    emitter: &mut Emitter<'_>,
    elev_en: &dyn Fn(f64, f64) -> f32,
) -> Option<()> {
    let profile = model.profile;
    let span = profile.water_end - profile.water_start;
    let side = (profile.deck_width * 0.5 - 0.25).max(0.25);
    let concrete = model.structure_appearance(true);
    let steel = model.structure_appearance(false);
    if profile.kind == BridgeKind::Arch && profile.tagged_structure != BridgeStructure::TiedArch {
        return lower_arch(model, emitter, elev_en);
    }
    if matches!(profile.kind, BridgeKind::Beam | BridgeKind::Truss) {
        let spans = (span / 42.0).ceil().max(1.0) as usize;
        for index in 1..spans {
            let s = profile.water_start + span * index as f64 / spans as f64;
            let top = model.height(s) - model.thickness();
            let diameter = (span / spans as f64 / 22.0).clamp(0.85, 2.5);
            for offset in [-profile.deck_width * 0.18, profile.deck_width * 0.18] {
                let p = model.point(s, offset, top);
                let ground = f64::from(elev_en(p.x, p.z)) - 0.5;
                emitter.beam(
                    DVec3::new(p.x, ground, p.z),
                    p,
                    diameter,
                    diameter,
                    concrete,
                );
            }
            emitter.beam(
                model.point(s, -side * 0.72, top - 0.35),
                model.point(s, side * 0.72, top - 0.35),
                1.4,
                0.7,
                concrete,
            );
        }
    }
    if profile.kind == BridgeKind::Beam {
        return (!emitter.failed).then_some(());
    }
    let cable_stayed = profile.tagged_structure == BridgeStructure::CableStayed;
    let tower_fractions = if cable_stayed {
        [0.32, 0.68]
    } else {
        [0.2, 0.8]
    };
    if profile.kind == BridgeKind::Suspension {
        for fraction in tower_fractions {
            let s = profile.water_start + span * fraction;
            let h = model.height(s);
            let top = f64::from(super::suspension_top(profile, h as f32));
            let diameter = (span / 250.0).clamp(1.1, 3.5);
            for offset in [-side, side] {
                let p = model.point(s, offset, top);
                let ground = f64::from(elev_en(p.x, p.z)) - 0.5;
                emitter.beam(
                    DVec3::new(p.x, ground, p.z),
                    p,
                    diameter,
                    diameter,
                    concrete,
                );
            }
            for cross_height in [top - 0.5, h + (top - h) * 0.6] {
                emitter.beam(
                    model.point(s, -side, cross_height),
                    model.point(s, side, cross_height),
                    diameter,
                    diameter * 0.7,
                    concrete,
                );
            }
        }
    }
    let spacing = match profile.kind {
        BridgeKind::Beam => unreachable!(),
        BridgeKind::Arch => (span / 14.0).clamp(8.0, 25.0),
        BridgeKind::Truss => (span / 16.0).clamp(6.0, 18.0),
        BridgeKind::Suspension => (span / 24.0).clamp(8.0, 30.0),
    };
    if !cable_stayed {
        let mut stations = model.stations(
            profile.water_start,
            profile.water_end,
            (span / 64.0).clamp(2.0, 8.0),
        );
        if profile.kind == BridgeKind::Suspension {
            stations.extend(tower_fractions.map(|f| profile.water_start + span * f));
            stations.sort_by(f64::total_cmp);
            stations.dedup_by(|a, b| (*a - *b).abs() < EPS);
        }
        for pair in stations.windows(2) {
            let bands =
                [pair[0], pair[1]].map(|s| super::upper_band(profile, s, model.height(s) as f32));
            if bands
                .iter()
                .any(|(lo, hi)| !lo.is_finite() || !hi.is_finite())
            {
                continue;
            }
            let radius = match profile.kind {
                BridgeKind::Arch => f64::from(bands[0].1 - bands[0].0),
                BridgeKind::Truss => 0.4,
                BridgeKind::Suspension => (span / 2000.0).clamp(0.16, 0.48),
                BridgeKind::Beam => unreachable!(),
            };
            for lateral in [-side, side] {
                emitter.beam(
                    model.point(pair[0], lateral, f64::from(bands[0].0 + bands[0].1) * 0.5),
                    model.point(pair[1], lateral, f64::from(bands[1].0 + bands[1].1) * 0.5),
                    radius,
                    radius,
                    steel,
                );
            }
        }
    }
    let count = (span / spacing).ceil() as usize;
    for index in 0..=count {
        let s = (profile.water_start + index as f64 * spacing).min(profile.water_end);
        let h = model.height(s);
        let (lo, hi) = super::upper_band(profile, s, h as f32);
        for lateral in [-side, side] {
            if cable_stayed {
                let fraction = if s < profile.water_start + span * 0.5 {
                    tower_fractions[0]
                } else {
                    tower_fractions[1]
                };
                let tower = profile.water_start + span * fraction;
                let top = f64::from(super::suspension_top(profile, model.height(tower) as f32));
                emitter.beam(
                    model.point(s, lateral, h + 0.5),
                    model.point(tower, lateral, top - 0.5),
                    0.12,
                    0.12,
                    steel,
                );
            } else if lo.is_finite() && hi.is_finite() {
                let width = if profile.kind == BridgeKind::Suspension {
                    0.11
                } else {
                    0.28
                };
                emitter.beam(
                    model.point(s, lateral, h + 0.5),
                    model.point(s, lateral, f64::from(lo + hi) * 0.5),
                    width,
                    width,
                    steel,
                );
                if profile.kind == BridgeKind::Truss && index < count {
                    let next = (s + spacing).min(profile.water_end);
                    let next_h = model.height(next);
                    let (_, next_top) = super::upper_band(profile, next, next_h as f32);
                    emitter.beam(
                        model.point(s, lateral, h + 0.45),
                        model.point(next, lateral, f64::from(next_top) - 0.6),
                        0.32,
                        0.32,
                        steel,
                    );
                }
            }
        }
    }
    (!emitter.failed).then_some(())
}

fn lower_arch(
    model: &Model<'_>,
    emitter: &mut Emitter<'_>,
    elev_en: &dyn Fn(f64, f64) -> f32,
) -> Option<()> {
    let profile = model.profile;
    let count = super::arch_spans(profile);
    let bay = (profile.water_end - profile.water_start) / count as f64;
    let voxel = model.structure_appearance(true);
    for arch in 0..count {
        let start = profile.water_start + bay * arch as f64;
        for step in 0..16 {
            let stations = [
                start + bay * step as f64 / 16.0,
                start + bay * (step + 1) as f64 / 16.0,
            ];
            let rings = stations.map(|s| {
                let (low, high) = super::upper_band(profile, s, model.height(s) as f32);
                let (left, right) = model.deck_edges(s);
                [
                    model.point(s, left, f64::from(low)),
                    model.point(s, right, f64::from(low)),
                    model.point(s, right, f64::from(high)),
                    model.point(s, left, f64::from(high)),
                ]
            });
            emitter.prism(rings, voxel);
        }
    }
    for index in 0..=count {
        let s = profile.water_start + bay * index as f64;
        let point = model.center(s);
        let ground = f64::from(elev_en(point.x, point.y)) - 0.5;
        let top = model.height(s) - model.thickness();
        let (left, right) = model.deck_edges(s);
        let width = (bay * 0.075).clamp(1.2, 2.5);
        emitter.beam(
            model.point(s, left, (top + ground) * 0.5),
            model.point(s, right, (top + ground) * 0.5),
            width,
            (top - ground).max(0.5),
            voxel,
        );
    }
    (!emitter.failed).then_some(())
}

struct Emitter<'a> {
    frame: &'a ChunkFrame,
    projection: &'a GridProj,
    exclusions: Vec<Vec<DVec2>>,
    flip_winding: bool,
    mesh: MeshBuffers,
    failed: bool,
    step_m: f64,
    ornament: bool,
}

impl<'a> Emitter<'a> {
    fn new(frame: &'a ChunkFrame, projection: &'a GridProj, exclusions: &[Vec<DVec2>]) -> Self {
        let origin = DVec2::from_array(projection.to_cols([0.0, 0.0]).map(f64::from));
        let east = DVec2::from_array(projection.to_cols([100.0, 0.0]).map(f64::from)) - origin;
        let north = DVec2::from_array(projection.to_cols([0.0, 100.0]).map(f64::from)) - origin;
        Self {
            frame,
            projection,
            exclusions: exclusions.to_vec(),
            flip_winding: east.perp_dot(north) < 0.0,
            mesh: MeshBuffers::default(),
            failed: false,
            step_m: 8.0,
            ornament: true,
        }
    }

    fn local(&self, p: DVec3) -> DVec3 {
        let [x, z] = self
            .projection
            .to_cols([p.x as f32, p.z as f32])
            .map(f64::from);
        let (_, offset) = self.frame.column_surface(x / N as f64, z / N as f64);
        DVec3::new(x, (p.y + offset) / self.frame.grid_scale.y, z)
    }

    fn face(&mut self, points: &[DVec3], voxel: SurfaceAppearance) {
        if self.failed {
            return;
        }
        let mut points: Vec<_> = points.iter().map(|&p| self.local(p)).collect();
        if self.flip_winding {
            points.reverse();
        }
        if points.iter().any(|p| !p.is_finite()) {
            self.failed = true;
            return;
        }
        let lo = points
            .iter()
            .copied()
            .fold(DVec3::splat(f64::INFINITY), DVec3::min);
        let hi = points
            .iter()
            .copied()
            .fold(DVec3::splat(f64::NEG_INFINITY), DVec3::max);
        for rectangle in &[[0.0, 0.0, N as f64, N as f64]] {
            if hi.x < rectangle[0]
                || hi.z < rectangle[1]
                || lo.x > rectangle[2]
                || lo.z > rectangle[3]
            {
                continue;
            }
            let mut clipped = points.clone();
            for (axis, edge, sign) in [
                (0, rectangle[0], 1.0),
                (2, rectangle[1], 1.0),
                (0, rectangle[2], -1.0),
                (2, rectangle[3], -1.0),
            ] {
                clipped = clip_face(&clipped, axis, edge, sign);
                if clipped.len() < 3 {
                    break;
                }
            }
            let mut pieces = vec![clipped];
            for exclusion in &self.exclusions {
                pieces = pieces
                    .into_iter()
                    .flat_map(|polygon| outside_footprint(&polygon, exclusion))
                    .collect();
            }
            for clipped in pieces {
                for i in 1..clipped.len().saturating_sub(1) {
                    let triangle = [clipped[0], clipped[i], clipped[i + 1]]
                        .map(|p| DVec3::from_array(p.as_vec3().to_array().map(f64::from)));
                    let normal = (triangle[1] - triangle[0]).cross(triangle[2] - triangle[0]);
                    if normal.length_squared() < 1.0e-18 {
                        continue;
                    }
                    let payload = encode_surface_normal(
                        2 | (3 << 3) | (u32::from(voxel.material) << 5),
                        normal.normalize().as_vec3().to_array(),
                    );
                    let base = self.mesh.vertices.len() as u32;
                    self.mesh.vertices.extend(triangle.map(|p| SurfaceVertex {
                        pos: p.as_vec3().to_array(),
                        face_ao: payload,
                        rgba: [voxel.tint[0], voxel.tint[1], voxel.tint[2], 255],
                    }));
                    self.mesh.indices.extend([base, base + 1, base + 2]);
                }
            }
            if self.mesh.indices.len() / 3 > MAX_TRIANGLES {
                self.failed = true;
                return;
            }
        }
    }

    fn beam(&mut self, a: DVec3, b: DVec3, width: f64, depth: f64, voxel: SurfaceAppearance) {
        let delta = b - a;
        if delta.length_squared() < EPS {
            return;
        }
        let direction = delta.normalize();
        let side = if direction.y.abs() < 0.99 {
            direction.cross(DVec3::Y).normalize()
        } else {
            DVec3::X
        } * (width * 0.5);
        let up = side.normalize().cross(direction).normalize() * (depth * 0.5);
        let rings = [a, b].map(|p| [p - side - up, p + side - up, p + side + up, p - side + up]);
        self.prism(rings, voxel);
    }

    fn prism(&mut self, rings: [[DVec3; 4]; 2], voxel: SurfaceAppearance) {
        self.face(&rings[0], voxel);
        self.face(&rings[1].into_iter().rev().collect::<Vec<_>>(), voxel);
        for i in 0..4 {
            let j = (i + 1) % 4;
            self.face(&[rings[0][i], rings[1][i], rings[1][j], rings[0][j]], voxel);
        }
    }
}

fn clip_face(points: &[DVec3], axis: usize, edge: f64, sign: f64) -> Vec<DVec3> {
    let mut output = Vec::new();
    for (&a, &b) in points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
    {
        let da = (a[axis] - edge) * sign;
        let db = (b[axis] - edge) * sign;
        if da >= 0.0 {
            output.push(a);
        }
        if (da >= 0.0) != (db >= 0.0) {
            let mut point = a.lerp(b, da / (da - db));
            point[axis] = edge;
            output.push(point);
        }
    }
    output
}

/// Partition a face against a convex, counter-clockwise horizontal footprint.
/// Keeping each exterior half-space in sequence produces disjoint pieces and
/// preserves vertical bridge walls as well as horizontal decks.
fn outside_footprint(points: &[DVec3], footprint: &[DVec2]) -> Vec<Vec<DVec3>> {
    let mut remainder = points.to_vec();
    let mut outside = Vec::new();
    for (&a, &b) in footprint
        .iter()
        .zip(footprint.iter().cycle().skip(1))
        .take(footprint.len())
    {
        let edge = b - a;
        let distance = |p: DVec3| edge.perp_dot(DVec2::new(p.x, p.z) - a);
        let clip = |keep_inside: bool| {
            let mut result = Vec::new();
            for (&p, &q) in remainder
                .iter()
                .zip(remainder.iter().cycle().skip(1))
                .take(remainder.len())
            {
                let dp = distance(p);
                let dq = distance(q);
                let pin = if keep_inside { dp >= 0.0 } else { dp < 0.0 };
                let qin = if keep_inside { dq >= 0.0 } else { dq < 0.0 };
                if pin {
                    result.push(p);
                }
                if pin != qin {
                    result.push(p.lerp(q, dp / (dp - dq)));
                }
            }
            result
        };
        let piece = clip(false);
        if piece.len() >= 3 {
            outside.push(piece);
        }
        remainder = clip(true);
        if remainder.len() < 3 {
            break;
        }
    }
    outside
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::material;
    use vearth_core::{
        cell::{CellId, ChunkAddr},
        ellipsoid,
        mesh::decode_surface_normal,
    };

    fn frame(level: u8, lat: f64, lon: f64) -> ChunkFrame {
        let direction =
            ellipsoid::geodetic_to_ecef(lat.to_radians(), lon.to_radians(), 0.0).normalize();
        ChunkFrame::of(ChunkAddr {
            cell: CellId::from_dir(direction, level),
            k: 0,
        })
    }

    fn fixture(structure: BridgeStructure) -> CanonicalBridge {
        CanonicalBridge {
            id: 71,
            class: RoadClass::Minor,
            alignment: vec![[-50.0, 0.0], [50.0, 0.0]],
            deck_outline: Vec::new(),
            bridge_start_m: 10.0,
            bridge_end_m: 90.0,
            water_start_m: 20.0,
            water_end_m: 80.0,
            deck_width_m: 8.0,
            road_width_m: 6.5,
            water_is_ocean: false,
            structure,
            material: BridgeMaterial::Unknown,
        }
    }

    fn projection(frame: &ChunkFrame) -> GridProj {
        let (lat, lon) = frame.column_latlon(0.5, 0.5);
        GridProj::of(frame, lat, lon)
    }

    fn to_en(projection: &GridProj, point: DVec2) -> DVec2 {
        let a = DVec2::from_array(projection.to_cols([0.0, 0.0]).map(f64::from));
        let e = DVec2::from_array(projection.to_cols([100.0, 0.0]).map(f64::from)) - a;
        let n = DVec2::from_array(projection.to_cols([0.0, 100.0]).map(f64::from)) - a;
        let p = point - a;
        DVec2::new(p.perp_dot(n), e.perp_dot(p)) * (100.0 / e.perp_dot(n))
    }

    #[test]
    fn bridge_members_have_outward_winding_in_every_geographic_orientation() {
        for (lat, lon) in [(47.0, 8.0), (60.0, 25.0), (-35.0, 150.0), (0.0, -75.0)] {
            let frame = frame(17, lat, lon);
            let projection = projection(&frame);
            let mut emitter = Emitter::new(&frame, &projection, &[]);
            let (a, b) = (DVec3::new(-2.0, 20.0, -1.0), DVec3::new(3.0, 24.0, 2.0));
            let center = (emitter.local(a) + emitter.local(b)) * 0.5;
            emitter.beam(a, b, 1.0, 1.2, support_appearance(0.5, 1.0));
            assert_eq!(emitter.mesh.indices.len(), 36);
            for triangle in emitter.mesh.indices.chunks_exact(3) {
                let p = [triangle[0], triangle[1], triangle[2]].map(|i| {
                    DVec3::from_array(emitter.mesh.vertices[i as usize].pos.map(f64::from))
                });
                let normal = (p[1] - p[0]).cross(p[2] - p[0]);
                assert!(
                    normal.dot((p[0] + p[1] + p[2]) / 3.0 - center) > 0.0,
                    "{lat}/{lon}: inverted member"
                );
                let encoded =
                    decode_surface_normal(emitter.mesh.vertices[triangle[0] as usize].face_ao)
                        .unwrap();
                assert!(
                    normal
                        .normalize()
                        .dot(DVec3::from_array(encoded.map(f64::from)))
                        > 0.9
                );
            }
        }
    }

    #[test]
    fn source_deck_concavity_and_profile_heights_survive_meshing() {
        let frame = frame(16, 47.0, 8.0);
        let projection = projection(&frame);
        let mut source = fixture(BridgeStructure::Beam);
        source.bridge_start_m = 0.0;
        source.bridge_end_m = 100.0;
        source.deck_outline = vec![
            [-45.0, -6.0],
            [45.0, -6.0],
            [45.0, 6.0],
            [5.0, 6.0],
            [5.0, 1.0],
            [-45.0, 1.0],
        ];
        let profile = build_profile(&source, &projection, &|_, _| 0.0).unwrap();
        let model = Model {
            source: &source,
            profile: &profile,
            projection: &projection,
        };
        let mut emitter = Emitter::new(&frame, &projection, &[]);
        deck(&model, &mut emitter).unwrap();
        let mut area = 0.0;
        for triangle in emitter.mesh.indices.chunks_exact(3) {
            let vertices =
                [triangle[0], triangle[1], triangle[2]].map(|i| emitter.mesh.vertices[i as usize]);
            if (vertices[0].face_ao >> 5) & 255 != u32::from(material::ASPHALT) {
                continue;
            }
            let p = vertices.map(|v| DVec3::from_array(v.pos.map(f64::from)));
            assert!((p[1] - p[0]).cross(p[2] - p[0]).y > 0.0);
            let en = p.map(|v| to_en(&projection, DVec2::new(v.x, v.z)));
            area += (en[1] - en[0]).perp_dot(en[2] - en[0]).abs() * 0.5;
            let center = (en[0] + en[1] + en[2]) / 3.0;
            assert!(center.x >= 5.0 - 0.001 || center.y <= 1.0 + 0.001);
            let local_center = (p[0] + p[1] + p[2]) / 3.0;
            let (_, center_offset) =
                frame.column_surface(local_center.x / N as f64, local_center.z / N as f64);
            let interpolated_height = local_center.y * frame.grid_scale.y - center_offset;
            assert!(
                (interpolated_height - model.height(model.station(center))).abs() < 0.002,
                "deck triangle crossed an unsplit profile station"
            );
            for (v, en) in p.into_iter().zip(en) {
                let (_, offset) = frame.column_surface(v.x / N as f64, v.z / N as f64);
                let geo = v.y * frame.grid_scale.y - offset;
                assert!((geo - model.height(model.station(en))).abs() < 0.002);
            }
        }
        assert!((area - 830.0).abs() < 0.01, "source deck area {area}");
    }

    #[test]
    fn bridge_arch_vault_and_collision_stay_below_the_deck_unless_tied() {
        let frame = frame(16, 48.8578, 2.3419);
        let projection = projection(&frame);
        for kind in [BridgeStructure::Arch, BridgeStructure::TiedArch] {
            let mut source = fixture(kind);
            source.deck_outline = vec![[-40.0, -5.0], [40.0, -5.0], [40.0, 7.0], [-40.0, 7.0]];
            let profile = build_profile(&source, &projection, &|_, _| 0.0).unwrap();
            let model = Model {
                source: &source,
                profile: &profile,
                projection: &projection,
            };
            assert_eq!(model.deck_edges(50.0), (-5.0, 7.0));
            let mut emitter = Emitter::new(&frame, &projection, &[]);
            structure(&model, &mut emitter, &|_, _| 0.0).unwrap();
            assert!(!emitter.mesh.vertices.is_empty());
            let mut has_above = false;
            for vertex in &emitter.mesh.vertices {
                let p = DVec3::from_array(vertex.pos.map(f64::from));
                let en = to_en(&projection, DVec2::new(p.x, p.z));
                let (_, offset) = frame.column_surface(p.x / N as f64, p.z / N as f64);
                let geo = p.y * frame.grid_scale.y - offset;
                has_above |= geo > model.height(model.station(en)) + 0.1;
            }
            assert_eq!(has_above, kind == BridgeStructure::TiedArch);
            for step in 0..101 {
                let s = profile.water_start
                    + (profile.water_end - profile.water_start) * step as f64 / 100.0;
                let deck = model.height(s) as f32;
                let (low, high) = super::super::upper_band(&profile, s, deck);
                if !high.is_finite() {
                    assert_eq!(kind, BridgeStructure::TiedArch);
                    continue;
                }
                assert!(low <= high);
                assert_eq!(high > deck, kind == BridgeStructure::TiedArch);
            }
        }
    }

    #[test]
    fn bridge_families_keep_clearance_and_curated_column_ownership() {
        for structure in [
            BridgeStructure::Beam,
            BridgeStructure::Arch,
            BridgeStructure::TiedArch,
            BridgeStructure::Truss,
            BridgeStructure::Suspension,
            BridgeStructure::CableStayed,
        ] {
            let frame = frame(17, 47.0, 8.0);
            let mut pack = FeaturePack::empty();
            pack.bridge_authority = true;
            pack.bridges.push(fixture(structure));
            let excluded = vec![vec![
                DVec2::splat(12.0),
                DVec2::new(20.0, 12.0),
                DVec2::splat(20.0),
                DVec2::new(12.0, 20.0),
            ]];
            let surface = generate(&frame, &pack, &excluded, &|_, _| 0.0).unwrap();
            assert!(surface.complete);
            assert!(!surface.collision.is_empty());
            assert!(surface.mesh.indices.len() < MAX_TRIANGLES * 3);
            for triangle in surface.mesh.indices.chunks_exact(3) {
                let p = [triangle[0], triangle[1], triangle[2]].map(|i| {
                    DVec3::from_array(surface.mesh.vertices[i as usize].pos.map(f64::from))
                });
                let center = (p[0] + p[1] + p[2]) / 3.0;
                assert!(
                    !(center.x > 12.00001
                        && center.x < 19.99999
                        && center.z > 12.00001
                        && center.z < 19.99999)
                );
                assert!(p.iter().all(|p| p.is_finite()
                    && p.x >= 0.0
                    && p.x <= N as f64
                    && p.z >= 0.0
                    && p.z <= N as f64));
            }
            pack.bridge_authority = false;
            assert!(generate(&frame, &pack, &excluded, &|_, _| 0.0).is_none());
            pack.bridge_authority = true;
            pack.bridges[0].alignment[0][0] = f32::NAN;
            assert!(
                !generate(&frame, &pack, &excluded, &|_, _| 0.0)
                    .unwrap()
                    .complete
            );
        }
    }
}
