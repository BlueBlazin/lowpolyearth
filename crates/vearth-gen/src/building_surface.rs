//! Continuous mapped building envelopes with metric architectural datums.

mod city_fixtures;

use glam::DVec2;
use vearth_data::vectors::Rings;

use crate::{
    N, SurfaceAppearance, architecture_facade,
    architecture_styles::{
        Banding, GroundStorey, OpeningShape, ProjectionKind, Roofline, ShutterKind, StyleProfile,
    },
    tier0::{self, BuildingStamp, CellOverlay, FacadeStamp},
};
use glam::DVec3;
use std::collections::HashMap;
use std::rc::Rc;
use vearth_core::{
    ellipsoid::{self, MEAN_RADIUS},
    frame::ChunkFrame,
    mesh::{MeshBuffers, SurfaceVertex, encode_surface_normal},
};
use vearth_data::vectors::{Building, BuildingKind, FeaturePack, RoofShape};

pub(crate) const ROOF_SAMPLE_HINT_M: f32 = 1.0;

pub(crate) fn city_attachment_radius_m(seed: u64, level: u8) -> f64 {
    if level >= 16 {
        city_fixtures::attachment_radius_m(seed)
    } else {
        0.0
    }
}

pub(crate) struct BuildingSurfaceData {
    pub mesh: MeshBuffers,
    pub shadow_omissions: Vec<crate::ShadowOmission>,
    pub collision: Vec<crate::collision::CollisionTriangle>,
    pub details: Vec<BuildingDetail>,
    /// Whether all required source geometry succeeded, including empty output.
    pub complete: bool,
}

pub(crate) struct BuildingDetail {
    pub mesh: MeshBuffers,
    pub bound_center: [f32; 3],
    pub radius_m: f32,
    pub min_screen_px: f32,
}

/// Interior samples defining the floor datum, independent of the cell grid.
pub(crate) fn datum_samples(source: &Building) -> Option<Vec<[f32; 2]>> {
    let domain = Footprint::with_limits(
        vec![&source.rings],
        Vec::new(),
        MAX_BUILDING_FOOTPRINT_EDGES,
        MAX_ROOF_CELLS,
    )?;
    let (lo, hi) = bounds(&source.rings)?;
    if (hi - lo).max_element() > 4_096.0 {
        return None;
    }
    let mut result = Vec::new();
    for y in 0..3 {
        for x in 0..3 {
            let point = lo + (hi - lo) * DVec2::new((x as f64 + 0.5) / 3.0, (y as f64 + 0.5) / 3.0);
            if domain.contains(point) {
                result.push(point.as_vec2().to_array());
            }
        }
    }
    if result.is_empty() {
        let edge = domain.boundary().into_iter().next()?;
        let direction = (edge[1] - edge[0]).normalize();
        let point = (edge[0] + edge[1]) * 0.5 + DVec2::new(-direction.y, direction.x) * 0.05;
        if domain.contains(point) {
            result.push(point.as_vec2().to_array());
        }
    }
    (!result.is_empty()).then_some(result)
}

pub(crate) fn sample_latlon(frame: &ChunkFrame, en: [f32; 2]) -> (f64, f64) {
    let (lat, lon) = frame.column_latlon(0.5, 0.5);
    (
        (lat + f64::from(en[1]) / MEAN_RADIUS)
            .clamp(-std::f64::consts::FRAC_PI_2, std::f64::consts::FRAC_PI_2),
        (lon + f64::from(en[0]) / (MEAN_RADIUS * lat.cos().abs().max(1.0e-6))
            + std::f64::consts::PI)
            .rem_euclid(std::f64::consts::TAU)
            - std::f64::consts::PI,
    )
}

/// Fixed-detail datum coverage for every building this cell can anchor: each
/// source's bounds plus one datum pitch, so interior datum samples and their
/// bilinear taps stay inside the prepared tiles.
pub(crate) fn add_datum_cover(
    frame: &ChunkFrame,
    pack: &FeaturePack,
    cover: &mut crate::DatumCover,
) {
    if frame.addr.cell.level < tier0::BUILDINGS_MIN_LEVEL {
        return;
    }
    let (center_lat, center_lon) = frame.column_latlon(0.5, 0.5);
    let projection = tier0::GridProj::of(frame, center_lat, center_lon);
    for source in &pack.buildings {
        let Some((lo, hi)) = bounds(&source.rings) else {
            continue;
        };
        let points = [lo, DVec2::new(hi.x, lo.y), hi, DVec2::new(lo.x, hi.y)];
        let columns = points.map(|p| projection.to_cols(p.as_vec2().to_array()));
        let padding =
            projection.radius_cols(city_attachment_radius_m(source.seed, frame.addr.cell.level));
        if columns.iter().all(|p| p[0] < -padding[0])
            || columns.iter().all(|p| p[0] > N as f32 + padding[0])
            || columns.iter().all(|p| p[1] < -padding[1])
            || columns.iter().all(|p| p[1] > N as f32 + padding[1])
        {
            continue;
        }
        if datum_samples(source).is_none() {
            continue;
        }
        let corners = points.map(|p| sample_latlon(frame, p.as_vec2().to_array()));
        cover.add(&corners, crate::datum_pitch_m(corners[0].0));
    }
}

/// Coverage that building datums may read; see [`add_datum_cover`].
pub(crate) fn datum_cover(frame: &ChunkFrame, pack: &FeaturePack) -> crate::DatumCover {
    let mut cover =
        crate::DatumCover::new(frame, crate::datum_pitch_m(frame.column_latlon(0.5, 0.5).0));
    add_datum_cover(frame, pack, &mut cover);
    cover
}

pub(crate) fn roof_source_footprint(
    frame: &ChunkFrame,
    pack: &FeaturePack,
) -> Option<crate::SourceFootprint> {
    if frame.addr.cell.level < crate::SURFACE_MIN_LEVEL {
        return None;
    }
    let mut footprint = whole_source_cap(frame, pack)?;
    footprint.sample_hint_m = f64::from(ROOF_SAMPLE_HINT_M);
    Some(footprint)
}

/// One cap around every building the cell can anchor. Measured-roof surface
/// providers are regional and already sparse, so they keep this single cap.
fn whole_source_cap(frame: &ChunkFrame, pack: &FeaturePack) -> Option<crate::SourceFootprint> {
    if frame.addr.cell.level < tier0::BUILDINGS_MIN_LEVEL {
        return None;
    }
    let (center_lat, center_lon) = frame.column_latlon(0.5, 0.5);
    let projection = tier0::GridProj::of(frame, center_lat, center_lon);
    let mut radius_rad = 0.0f64;
    let mut found = false;
    for source in &pack.buildings {
        let Some((lo, hi)) = bounds(&source.rings) else {
            continue;
        };
        let points = [lo, DVec2::new(hi.x, lo.y), hi, DVec2::new(lo.x, hi.y)];
        let columns = points.map(|p| projection.to_cols(p.as_vec2().to_array()));
        let padding =
            projection.radius_cols(city_attachment_radius_m(source.seed, frame.addr.cell.level));
        if columns.iter().all(|p| p[0] < -padding[0])
            || columns.iter().all(|p| p[0] > N as f32 + padding[0])
            || columns.iter().all(|p| p[1] < -padding[1])
            || columns.iter().all(|p| p[1] > N as f32 + padding[1])
        {
            continue;
        }
        if datum_samples(source).is_none() {
            continue;
        }
        found = true;
        for p in points {
            let (lat, lon) = sample_latlon(frame, p.as_vec2().to_array());
            radius_rad = radius_rad
                .max(crate::angular_distance(center_lat, center_lon, lat, lon) + 8.0 / MEAN_RADIUS);
        }
    }
    found.then_some(crate::SourceFootprint {
        center_lat,
        center_lon,
        radius_rad: radius_rad + 1.0e-9,
        sample_hint_m: crate::datum_pitch_m(center_lat),
    })
}

fn bounds(rings: &Rings) -> Option<(DVec2, DVec2)> {
    let (mut lo, mut hi) = (DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY));
    for point in rings.iter().flatten() {
        let p = DVec2::from_array(point.map(f64::from));
        lo = lo.min(p);
        hi = hi.max(p);
    }
    (lo.is_finite() && hi.is_finite() && (hi - lo).min_element() > 0.01).then_some((lo, hi))
}

#[derive(Clone)]
struct Datum {
    floor: f64,
    bottom: f64,
    measured: Option<f64>,
}

struct Projection<'a> {
    frame: &'a ChunkFrame,
    lat: f64,
    lon: f64,
}
impl Projection<'_> {
    fn latlon(&self, p: DVec2) -> (f64, f64) {
        (
            self.lat + p.y / MEAN_RADIUS,
            (self.lon
                + p.x / (MEAN_RADIUS * self.lat.cos().abs().max(1.0e-6))
                + std::f64::consts::PI)
                .rem_euclid(std::f64::consts::TAU)
                - std::f64::consts::PI,
        )
    }
    fn point(&self, p: DVec2, h: f64) -> DVec3 {
        self.frame.ecef_to_local(self.world_point(p, h))
    }
    fn world_point(&self, p: DVec2, h: f64) -> DVec3 {
        let (lat, lon) = self.latlon(p);
        ellipsoid::geodetic_to_ecef(lat, lon, h)
    }
}

/// Generate visual envelopes and continuous collision from mapped source rings.
pub(crate) fn generate(
    frame: &ChunkFrame,
    pack: &FeaturePack,
    overlay: &CellOverlay,
    eligible_columns: &[u32; N],
    elevation: &dyn Fn(f64, f64, f32) -> Option<f32>,
    surface: &dyn Fn(f64, f64, f32) -> Option<f32>,
) -> Option<BuildingSurfaceData> {
    generate_bounded(
        frame,
        pack,
        overlay,
        eligible_columns,
        elevation,
        surface,
        crate::built_surface::BUILDING_BASE_BUDGET_BYTES,
    )
}

fn generate_bounded(
    frame: &ChunkFrame,
    pack: &FeaturePack,
    overlay: &CellOverlay,
    eligible_columns: &[u32; N],
    elevation: &dyn Fn(f64, f64, f32) -> Option<f32>,
    surface: &dyn Fn(f64, f64, f32) -> Option<f32>,
    budget: usize,
) -> Option<BuildingSurfaceData> {
    generate_bounded_with_coarse_reuse::<true>(
        frame,
        pack,
        overlay,
        eligible_columns,
        elevation,
        surface,
        budget,
    )
}

fn generate_bounded_with_coarse_reuse<const REUSE: bool>(
    frame: &ChunkFrame,
    pack: &FeaturePack,
    overlay: &CellOverlay,
    eligible_columns: &[u32; N],
    elevation: &dyn Fn(f64, f64, f32) -> Option<f32>,
    surface: &dyn Fn(f64, f64, f32) -> Option<f32>,
    budget: usize,
) -> Option<BuildingSurfaceData> {
    // Geometry budgets change tessellation, never source ownership or ground
    // datums. Keep their immutable preparation alive across a whole-cell retry.
    let plan = prepare_envelopes_with_coarse_reuse::<REUSE>(
        frame,
        pack,
        overlay,
        eligible_columns,
        elevation,
        surface,
    )?;
    let full_started =
        crate::contracts::generation_timing_enabled().then(vearth_platform::time::Instant::now);
    let mut result = generate_prepared_envelopes::<true>(
        frame,
        pack,
        overlay,
        eligible_columns,
        elevation,
        surface,
        false,
        budget,
        &plan,
    )?;
    // The detailed pass can stop between features at this limit. Compaction
    // must not hide that stop and publish a cell missing its remaining owners.
    let retry_simple = crate::built_surface::mesh_bytes(&result.mesh) > budget;
    if retry_simple {
        if let Some(started) = full_started {
            crate::contracts::record_building_retry(
                frame.addr.cell.level,
                started.elapsed().as_nanos() as u64,
                crate::built_surface::mesh_bytes(&result.mesh)
                    + result
                        .details
                        .iter()
                        .map(|detail| crate::built_surface::mesh_bytes(&detail.mesh))
                        .sum::<usize>(),
            );
        }
        result = generate_prepared_envelopes::<true>(
            frame,
            pack,
            overlay,
            eligible_columns,
            elevation,
            surface,
            true,
            budget,
            &plan,
        )?;
        crate::built_surface::compact_mesh(&mut result.mesh);
    }
    Some(result)
}

struct PlannedDomain<'a> {
    footprint: Footprint<'a>,
    boundary: std::cell::OnceCell<Vec<[DVec2; 2]>>,
}

struct EnvelopePlan<'a> {
    stamps: HashMap<usize, BuildingStamp>,
    source_bounds: Vec<Option<(DVec2, DVec2)>>,
    overlaps: Vec<Vec<usize>>,
    detail_neighbors: Vec<Vec<usize>>,
    domains: Vec<Option<PlannedDomain<'a>>>,
    datums: Vec<Option<Datum>>,
    complete: bool,
    facade_resolution: FacadeResolution,
    roof_resolution: RoofResolution,
}

#[cfg(test)]
fn generate_envelopes<const PRUNE_HIDDEN_WALLS: bool>(
    frame: &ChunkFrame,
    pack: &FeaturePack,
    overlay: &CellOverlay,
    eligible_columns: &[u32; N],
    elevation: &dyn Fn(f64, f64, f32) -> Option<f32>,
    surface: &dyn Fn(f64, f64, f32) -> Option<f32>,
    simple: bool,
    budget: usize,
) -> Option<BuildingSurfaceData> {
    let plan = prepare_envelopes(frame, pack, overlay, eligible_columns, elevation, surface)?;
    generate_prepared_envelopes::<PRUNE_HIDDEN_WALLS>(
        frame,
        pack,
        overlay,
        eligible_columns,
        elevation,
        surface,
        simple,
        budget,
        &plan,
    )
}

#[cfg(test)]
fn prepare_envelopes<'a>(
    frame: &ChunkFrame,
    pack: &'a FeaturePack,
    overlay: &CellOverlay,
    eligible_columns: &[u32; N],
    elevation: &dyn Fn(f64, f64, f32) -> Option<f32>,
    surface: &dyn Fn(f64, f64, f32) -> Option<f32>,
) -> Option<EnvelopePlan<'a>> {
    prepare_envelopes_with_coarse_reuse::<true>(
        frame,
        pack,
        overlay,
        eligible_columns,
        elevation,
        surface,
    )
}

fn prepare_envelopes_with_coarse_reuse<'a, const REUSE: bool>(
    frame: &ChunkFrame,
    pack: &'a FeaturePack,
    overlay: &CellOverlay,
    eligible_columns: &[u32; N],
    elevation: &dyn Fn(f64, f64, f32) -> Option<f32>,
    surface: &dyn Fn(f64, f64, f32) -> Option<f32>,
) -> Option<EnvelopePlan<'a>> {
    if frame.addr.cell.level < tier0::BUILDINGS_MIN_LEVEL
        || pack.buildings.is_empty()
        || eligible_columns.iter().all(|row| *row == 0)
    {
        return None;
    }
    let (lat, lon) = frame.column_latlon(0.5, 0.5);
    let projection = Projection { frame, lat, lon };
    let stamps: HashMap<_, _> = overlay
        .source_buildings
        .iter()
        .filter_map(|stamp| stamp.source_index.map(|index| (index, *stamp)))
        .collect();
    if stamps.is_empty() && overlay.source_towers.is_empty() {
        return None;
    }
    let source_bounds: Vec<_> = pack.buildings.iter().map(|b| bounds(&b.rings)).collect();
    let overlaps = overlapping_sources(&source_bounds);
    // Projections also encounter touching neighbours, which deliberately do
    // not participate in source-volume overlap ownership.
    let detail_bounds: Vec<_> = source_bounds
        .iter()
        .map(|bounds| bounds.map(|(lo, hi)| (lo - DVec2::splat(2.0), hi + DVec2::splat(2.0))))
        .collect();
    let detail_neighbors = overlapping_sources(&detail_bounds);
    // Source order resolves equal-height ownership and feeds polygon clipping.
    // Reuse the same ordered exclusions for admission and envelope emission.
    let exclusions: Vec<Vec<_>> = pack
        .buildings
        .iter()
        .enumerate()
        .map(|(i, source)| {
            overlaps[i]
                .iter()
                .copied()
                .filter(|&j| wins(j, &pack.buildings[j], i, source))
                .collect()
        })
        .collect();
    let coarse_budget = Rc::new(std::cell::Cell::new(COARSE_FOOTPRINT_CACHE_BYTES));
    let domains: Vec<_> = pack
        .buildings
        .iter()
        .enumerate()
        .map(|(i, source)| {
            if !stamps.contains_key(&i) {
                return None;
            }
            let exclusions = exclusions[i]
                .iter()
                .map(|&j| &pack.buildings[j].rings)
                .collect();
            let mut footprint =
                Footprint::for_building(vec![&source.rings], exclusions, frame.addr.cell.level)?;
            if REUSE {
                footprint.coarse_cache = Some(CoarseFootprintCache {
                    remaining_bytes: Rc::clone(&coarse_budget),
                    cells: std::cell::OnceCell::new(),
                });
            }
            Some(PlannedDomain {
                footprint,
                boundary: std::cell::OnceCell::new(),
            })
        })
        .collect();
    let mut complete = true;
    let mut needed: Vec<_> = pack
        .buildings
        .iter()
        .enumerate()
        .map(|(i, _)| {
            if !stamps.contains_key(&i) {
                return false;
            }
            let Some(domain) = &domains[i] else {
                crate::contracts::record_building_failure(1, frame.addr.cell.level);
                complete = false;
                return false;
            };
            match domain.footprint.coarse_roof_cells() {
                Some(cells) => !cells.is_empty(),
                None => {
                    crate::contracts::record_building_failure(9, frame.addr.cell.level);
                    complete = false;
                    false
                }
            }
        })
        .collect();
    let mut parents = vec![None; pack.buildings.len()];
    // Source subtraction determines visible envelopes before raster sampling.
    // Parent anchors remain necessary for elevated parts even if fully covered.
    for (i, source) in pack.buildings.iter().enumerate() {
        if !needed[i] || !source.kind.is_part() {
            continue;
        }
        let parent = stamps.get(&i).and_then(|stamp| stamp.parent_index);
        parents[i] = parent;
        if let Some(parent) = parent {
            needed[parent] = true;
        }
    }
    let cover = datum_cover(frame, pack);
    let mut datums = Vec::with_capacity(pack.buildings.len());
    for (i, source) in pack.buildings.iter().enumerate() {
        let datum = (|| {
            if !needed[i] {
                return None;
            }
            let (lo, hi) = source_bounds[i].or_else(|| {
                crate::contracts::record_building_failure(0, frame.addr.cell.level);
                None
            })?;
            if ![lo, DVec2::new(hi.x, lo.y), hi, DVec2::new(lo.x, hi.y)]
                .iter()
                .all(|point| {
                    let (lat, lon) = sample_latlon(frame, point.as_vec2().to_array());
                    cover.contains(lat, lon)
                })
            {
                crate::contracts::record_building_failure(8, frame.addr.cell.level);
                return None;
            }
            let samples = datum_samples(source).or_else(|| {
                crate::contracts::record_building_failure(0, frame.addr.cell.level);
                None
            })?;
            let height = |lat, lon| {
                elevation(lat, lon, crate::datum_pitch_m(lat) as f32)
                    .filter(|h| h.is_finite())
                    .or_else(|| {
                        crate::contracts::record_building_failure(2, frame.addr.cell.level);
                        None
                    })
            };
            let mut heights = Vec::new();
            let mut roofs = Vec::new();
            for point in samples {
                let (lat, lon) = projection.latlon(DVec2::from_array(point.map(f64::from)));
                heights.push(f64::from(height(lat, lon)?));
                if stamps.get(&i).is_some_and(|stamp| stamp.lidar)
                    && let Some(h) = surface(lat, lon, ROOF_SAMPLE_HINT_M)
                        .filter(|h| h.is_finite() && f64::from(*h) > *heights.last().unwrap() + 2.0)
                {
                    roofs.push(f64::from(h));
                }
            }
            heights.sort_by(f64::total_cmp);
            roofs.sort_by(f64::total_cmp);
            let mut support = heights[0];
            // Wall support reaches the source boundary, independently of the
            // interior floor datum, so downhill corners cannot float.
            for point in source.rings.iter().flatten() {
                let (lat, lon) = projection.latlon(DVec2::from_array(point.map(f64::from)));
                support = support.min(f64::from(height(lat, lon)?));
            }
            Some(Datum {
                floor: heights[heights.len() / 2],
                bottom: support - 0.4,
                measured: (!roofs.is_empty()).then(|| roofs[roofs.len() / 2]),
            })
        })();
        complete &= !needed[i] || datum.is_some();
        datums.push(datum);
    }
    // Relative heights on contained parts share their mapped whole-building
    // reference plane; adjoining unrelated footprints never share a datum.
    for (i, parent) in parents.into_iter().enumerate() {
        if let Some(parent) = parent.and_then(|j| datums[j].clone())
            && let Some(datum) = &mut datums[i]
        {
            datum.floor = parent.floor;
            datum.bottom = parent.bottom;
        }
    }
    Some(EnvelopePlan {
        stamps,
        source_bounds,
        overlaps,
        detail_neighbors,
        domains,
        datums,
        complete,
        facade_resolution: FacadeResolution::for_frame(frame),
        roof_resolution: RoofResolution::for_frame(frame),
    })
}

fn generate_prepared_envelopes<const PRUNE_HIDDEN_WALLS: bool>(
    frame: &ChunkFrame,
    pack: &FeaturePack,
    overlay: &CellOverlay,
    eligible_columns: &[u32; N],
    elevation: &dyn Fn(f64, f64, f32) -> Option<f32>,
    surface: &dyn Fn(f64, f64, f32) -> Option<f32>,
    simple: bool,
    budget: usize,
    plan: &EnvelopePlan<'_>,
) -> Option<BuildingSurfaceData> {
    let EnvelopePlan {
        stamps,
        source_bounds,
        overlaps,
        detail_neighbors,
        domains,
        datums,
        complete,
        facade_resolution,
        roof_resolution,
    } = plan;
    let (lat, lon) = frame.column_latlon(0.5, 0.5);
    let projection = Projection { frame, lat, lon };
    let coarse_pixel_m = coarse_building_pixel_m(frame);
    let mut result = BuildingSurfaceData {
        mesh: MeshBuffers::default(),
        shadow_omissions: Vec::new(),
        collision: Vec::new(),
        details: Vec::new(),
        complete: *complete,
    };
    let mut opaque_walls = OpaqueWallIndex::default();
    'features: for (i, source) in pack.buildings.iter().enumerate() {
        let Some(mut stamp) = stamps.get(&i).copied() else {
            continue;
        };
        let Some(datum) = &datums[i] else {
            continue;
        };
        let Some(planned) = &domains[i] else {
            crate::contracts::record_building_failure(1, frame.addr.cell.level);
            result.complete = false;
            continue;
        };
        let domain = &planned.footprint;
        let boundary = planned.boundary.get_or_init(|| domain.boundary());
        let (shape, rise) = (stamp.roof_shape, stamp.roof_height_m);
        stamp.roof_shape = if stamp.lidar {
            Some(RoofShape::Flat)
        } else {
            shape
        };
        let rise = if stamp.lidar {
            0.0
        } else {
            rise.unwrap_or(0.0) as f64
        };
        let top = if stamp.lidar {
            datum
                .measured
                .or_else(|| {
                    stamp
                        .top_geo
                        .is_finite()
                        .then_some(f64::from(stamp.top_geo))
                })
                .unwrap_or(datum.floor + f64::from(stamp.height))
        } else {
            datum.floor + f64::from(stamp.height)
        };
        let floor = datum.floor + f64::from(source.min_height_m.max(0.0));
        let flat_roof = FlatRoof::for_source(source, &stamp, floor, top).map(|roof| {
            if coarse_pixel_m.is_some_and(|pixel| roof.top - roof.deck <= pixel) {
                FlatRoof {
                    deck: roof.top,
                    thickness: 0.0,
                    ..roof
                }
            } else {
                roof.with_resolution(frame.addr.cell.sample_pitch_m() / 3.0)
            }
        });
        let measured_sample = |p: DVec2| -> Option<f64> {
            let (lat, lon) = projection.latlon(p);
            surface(lat, lon, ROOF_SAMPLE_HINT_M)
                .filter(|h| {
                    h.is_finite()
                        && f64::from(*h) > floor + 2.0
                        && (f64::from(*h) - top).abs()
                            <= (f64::from(source.height_m) * 0.15).clamp(12.0, 40.0)
                })
                .map(f64::from)
        };
        let eave = if stamp.lidar {
            let mut lowest = top;
            for ring in &source.rings {
                for i in 0..ring.len() {
                    let a = DVec2::from_array(ring[i].map(f64::from));
                    let b = DVec2::from_array(ring[(i + 1) % ring.len()].map(f64::from));
                    for n in 0..=8 {
                        if let Some(height) = measured_sample(a.lerp(b, n as f64 / 8.0)) {
                            lowest = lowest.min(height);
                        }
                    }
                }
            }
            lowest
        } else {
            flat_roof.map_or((top - rise).max(floor + 0.6), |roof| roof.deck)
        };
        let base = if source.min_height_m <= 0.0 {
            datum.bottom
        } else {
            floor
        };
        let mut mesh = MeshBuffers::default();
        let mut collision = MeshBuffers::default();
        let mut details = MeshBuffers::default();
        let mut roof = RoofField::new(
            &source.rings,
            stamp.roof_shape,
            eave,
            flat_roof.map_or(top, |roof| roof.deck),
            architecture_facade::production_hip_pitch(&stamp.design),
        );
        configure_roof_style(&mut roof, source, &stamp);
        let cap_top = if let Some(pixel_m) = coarse_pixel_m {
            // This decision belongs to the source's resolution, including
            // sources crossing owners. A subpixel roof retains its full
            // mapped outline and maximum height without interior tessellation.
            (!stamp.lidar && roof.top - roof.eave <= pixel_m).then_some(roof.top)
        } else {
            roof_resolution.contained_cap_top(&projection, source, &stamp, &roof)
        };
        let rotate_rings = |rings: &&Rings| -> Rings {
            rings
                .iter()
                .map(|ring| {
                    ring.iter()
                        .map(|p| {
                            roof.rotate(DVec2::from_array(p.map(f64::from)))
                                .as_vec2()
                                .to_array()
                        })
                        .collect()
                })
                .collect()
        };
        let required: Vec<_> = if cap_top.is_some() {
            Vec::new()
        } else {
            domain.required.iter().map(rotate_rings).collect()
        };
        let excluded: Vec<_> = if cap_top.is_some() {
            Vec::new()
        } else {
            domain.excluded.iter().map(rotate_rings).collect()
        };
        let rotated = if cap_top.is_some() {
            None
        } else {
            let Some(rotated) = Footprint::for_building(
                required.iter().collect(),
                excluded.iter().collect(),
                frame.addr.cell.level,
            ) else {
                crate::contracts::record_building_failure(10, frame.addr.cell.level);
                result.complete = false;
                continue;
            };
            Some(rotated)
        };
        let measured = |p: DVec2| -> f64 {
            if !stamp.lidar {
                return roof.height(p);
            }
            measured_sample(p).unwrap_or(top).max(eave)
        };
        let roof_appearance =
            crate::building_appearance(top + 1.0, floor, eave, &stamp, None, 0.01);
        let mut solid_wall_stamp = stamp;
        solid_wall_stamp.facade.boundary = false;
        let mut facade = FacadeContext::from_sources(
            &source.rings,
            &pack.buildings,
            if coarse_pixel_m.is_some() {
                &[]
            } else {
                &detail_neighbors[i]
            },
            datum.floor,
        );
        if PRUNE_HIDDEN_WALLS
            && source.kind == BuildingKind::Building
            && source.min_height_m <= 0.0
            && overlaps[i].is_empty()
            && !stamp.lidar
        {
            facade.party_walls = opaque_walls.facing(&boundary);
            if source.rings.len() == 1 && single_boundary_cycle(&boundary) {
                facade.opaque_core = Some(std::cell::RefCell::new(OpaqueCore::default()));
            }
        }
        if coarse_pixel_m.is_none()
            && source.min_height_m <= 0.0
            && source.kind != BuildingKind::RoofOnly
            && eave - floor >= 2.4
        {
            let front_axis = expressed_style(&stamp).and_then(|style| {
                (matches!(style.roofline, Roofline::FrontGable | Roofline::ShapedGable)
                    || (style.ground_storey == GroundStorey::Shopfront
                        && (roof.bounds[1] - roof.bounds[0]).x
                            > (roof.bounds[1] - roof.bounds[0]).y * 1.5))
                    .then_some(roof.direction)
            });
            facade.entrance =
                entrance_on_source_boundary(&source.rings, &boundary, floor, &facade, front_axis);
        }
        let mut roof_trace = RoofTrace::start(frame, source, i, &roof, simple, stamp.lidar, false);
        if let Some(trace) = &mut roof_trace {
            trace.record.work.analytic_attempts =
                u64::from(cap_top.is_none() && !simple && !stamp.lidar && rise > 0.0);
        }
        let analytic = (cap_top.is_none() && !simple && !stamp.lidar && rise > 0.0)
            .then(|| {
                analytic_roof(
                    rotated.as_ref().unwrap(),
                    &boundary,
                    &roof,
                    true,
                    coarse_pixel_m.map_or(3.0, |_| 4096.0),
                    &projection,
                )
            })
            .flatten();
        if let Some(trace) = &mut roof_trace {
            trace.record.work.analytic_accepted = u64::from(analytic.is_some());
        }
        let roof_mesh = if let Some(top) = cap_top {
            let cap = if coarse_pixel_m.is_some() {
                distant_roof_cap
            } else {
                coarse_roof_cap
            };
            cap(
                &domain,
                &boundary,
                top,
                roof_trace.as_mut().map(|trace| &mut trace.record.work),
            )
        } else {
            analytic.or_else(|| {
                let rotated = rotated.as_ref().unwrap();
                let cells = rotated
                    .roof_cells(coarse_pixel_m.unwrap_or_else(|| {
                        if !simple && (stamp.lidar || rise > 0.0) {
                            3.0
                        } else {
                            4096.0
                        }
                    }))
                    .or_else(|| {
                        if let Some(trace) = &mut roof_trace {
                            trace.record.work.coarse_grid_fallbacks += 1;
                        }
                        rotated.roof_cells(4096.0)
                    })?;
                roof_geometry_profiled(
                    cells.into_iter().map(|cell| cell.map(|p| roof.unrotate(p))),
                    &boundary,
                    &measured,
                    coarse_pixel_m.is_none() && !simple && !stamp.lidar && rise > 0.0,
                    roof_resolution.sampling_tolerance(top - eave, stamp.roof_shape),
                    if stamp.lidar {
                        None
                    } else {
                        roof.height_bounds()
                    },
                    roof_trace.as_mut().map(|trace| &mut trace.record.work),
                )
            })
        };
        let Some(roof_mesh) = roof_mesh else {
            crate::contracts::record_building_failure(3, frame.addr.cell.level);
            result.complete = false;
            continue;
        };
        if let Some(trace) = &mut roof_trace {
            trace.record.work.triangles = roof_mesh.triangles.len() as u64;
        }
        drop(roof_trace);
        let mut roof_triangles = roof_mesh.triangles.len();
        let first_roof_vertex = mesh.vertices.len();
        emit_roof(&projection, &mut mesh, &roof_mesh, roof_appearance);
        if coarse_pixel_m.is_some() && roof.top > roof.eave {
            seal_coarse_roof_cuts(frame, &mut mesh, 0, roof.eave, roof_appearance);
        }
        apply_finish_variant(
            &mut mesh.vertices[first_roof_vertex..],
            stamp.design.roof_variant,
        );
        if let Some(core) = &facade.opaque_core {
            core.borrow_mut().roof_y = mesh
                .vertices
                .iter()
                .map(|vertex| f64::from(vertex.pos[1]))
                .min_by(f64::total_cmp);
        }
        if emit_collision_prism(
            &projection,
            &mut collision,
            &domain,
            domain.coarse_cache.as_ref().map(|_| boundary.as_slice()),
            base,
            top,
        )
        .is_none()
        {
            crate::contracts::record_building_failure(4, frame.addr.cell.level);
            result.complete = false;
        }
        for edge in boundary {
            emit_wall_with_resolution(
                &projection,
                &mut mesh,
                &mut details,
                *edge,
                base,
                floor,
                eave,
                &stamp,
                &source.rings,
                &facade,
                simple,
                *facade_resolution,
            );
        }
        let wall = crate::building_appearance(
            (floor + eave) * 0.5,
            floor,
            eave,
            &solid_wall_stamp,
            None,
            0.01,
        );
        if let Some(flat_roof) = flat_roof {
            if !simple && facade_resolution.resolves(1.2) {
                emit_highrise_roof_plant(
                    &projection,
                    &mut details,
                    &domain,
                    source,
                    flat_roof,
                    &stamp,
                );
            }
            emit_flat_parapets(
                &projection,
                &mut mesh,
                &domain,
                &boundary,
                &source.rings,
                flat_roof,
                wall,
                facade_trim(&stamp),
            );
        }
        for edge in &roof_mesh.boundary {
            let first_closure = mesh.vertices.len();
            emit_roof_closure(&projection, &mut mesh, edge, eave, wall);
            if wall.material == stamp.design.wall_material {
                apply_finish_variant(
                    &mut mesh.vertices[first_closure..],
                    stamp.design.wall_variant,
                );
            }
            if coarse_pixel_m.is_none()
                && cap_top.is_none()
                && !stamp.lidar
                && source.roof_shape.is_none()
            {
                emit_gable_crown(
                    &projection,
                    &mut mesh,
                    edge,
                    &roof,
                    &source.rings,
                    &domain,
                    &stamp,
                    wall,
                    *facade_resolution,
                );
            }
            if coarse_pixel_m.is_none()
                && flat_roof.is_none()
                && (!simple || expressed_style(&stamp).is_some())
                && !stamp.lidar
                && source.kind == BuildingKind::Building
                && edge_facade(&source.rings, (edge.points[0] + edge.points[1]) * 0.5)
                    .edge_distance_m
                    <= 0.01
            {
                let direction = (edge.points[1] - edge.points[0]).normalize();
                let plane = FacadePlane {
                    shadow_units: None,
                    projection: &projection,
                    origin: edge.points[0],
                    direction,
                    outward: DVec2::new(direction.y, -direction.x),
                };
                let reach = expressed_style(&stamp)
                    .map_or(
                        architecture_facade::production_detail_profile(&stamp.design).eave_m,
                        |style| f64::from(style.eave_m),
                    )
                    .min(if roof.reentrant { 0.35 } else { 2.0 });
                if plane.clear(
                    &facade,
                    [0.0, edge.points[0].distance(edge.points[1])],
                    [eave - 0.2, top + 0.01],
                    reach,
                ) {
                    emit_eave(
                        &projection,
                        &mut mesh,
                        edge,
                        top - eave > 0.01,
                        roof_appearance,
                        &stamp.design,
                        reach,
                    );
                }
            }
        }
        if !simple && cap_top.is_none() && !stamp.lidar && source.roof_shape.is_none() {
            emit_eyebrow_dormers(
                &projection,
                &mut details,
                &roof,
                &domain,
                source,
                &stamp,
                roof_appearance,
                *facade_resolution,
            );
        }
        // Preserve a mapped parent's body beneath an elevated part, without
        // stretching the part itself down to terrain.
        if source.kind == BuildingKind::Building {
            for part in overlaps[i]
                .iter()
                .map(|&j| &pack.buildings[j])
                .filter(|p| p.kind.is_part() && p.min_height_m > source.min_height_m)
            {
                let Some(body) = Footprint::for_building(
                    vec![&source.rings, &part.rings],
                    Vec::new(),
                    frame.addr.cell.level,
                ) else {
                    crate::contracts::record_building_failure(11, frame.addr.cell.level);
                    result.complete = false;
                    continue;
                };
                let cap = (datum.floor + f64::from(part.min_height_m)).min(top);
                if cap <= floor {
                    continue;
                }
                let exposed_roof = datum.floor + f64::from(part.min_height_m) > top + 0.01;
                let body_eave = if exposed_roof { eave.min(cap) } else { cap };
                let body_boundary = body.boundary();
                for edge in &body_boundary {
                    emit_wall_with_resolution(
                        &projection,
                        &mut mesh,
                        &mut details,
                        *edge,
                        base,
                        floor,
                        body_eave,
                        &stamp,
                        &source.rings,
                        &facade,
                        simple,
                        *facade_resolution,
                    );
                }
                if exposed_roof {
                    let mut roof_trace =
                        RoofTrace::start(frame, source, i, &roof, simple, stamp.lidar, true);
                    if let Some(trace) = &mut roof_trace {
                        trace.record.work.analytic_attempts =
                            u64::from(cap_top.is_none() && !simple && !stamp.lidar && rise > 0.0);
                    }
                    let analytic = (cap_top.is_none() && !simple && !stamp.lidar && rise > 0.0)
                        .then(|| {
                            analytic_roof(
                                &body,
                                &body_boundary,
                                &roof,
                                false,
                                coarse_pixel_m.map_or(2.0, |_| 4096.0),
                                &projection,
                            )
                        })
                        .flatten();
                    if let Some(trace) = &mut roof_trace {
                        trace.record.work.analytic_accepted = u64::from(analytic.is_some());
                    }
                    let body_roof = if let Some(top) = cap_top {
                        let cap = if coarse_pixel_m.is_some() {
                            distant_roof_cap
                        } else {
                            coarse_roof_cap
                        };
                        cap(
                            &body,
                            &body_boundary,
                            top,
                            roof_trace.as_mut().map(|trace| &mut trace.record.work),
                        )
                    } else {
                        analytic.or_else(|| {
                            let cells = body
                                .roof_cells(coarse_pixel_m.unwrap_or(if simple {
                                    4096.0
                                } else {
                                    2.0
                                }))
                                .or_else(|| {
                                    if let Some(trace) = &mut roof_trace {
                                        trace.record.work.coarse_grid_fallbacks += 1;
                                    }
                                    body.roof_cells(4096.0)
                                })?;
                            roof_geometry_profiled(
                                cells,
                                &body_boundary,
                                &measured,
                                coarse_pixel_m.is_none() && !simple && !stamp.lidar && rise > 0.0,
                                roof_resolution.sampling_tolerance(top - eave, stamp.roof_shape),
                                if stamp.lidar {
                                    None
                                } else {
                                    roof.height_bounds()
                                },
                                roof_trace.as_mut().map(|trace| &mut trace.record.work),
                            )
                        })
                    };
                    let Some(body_roof) = body_roof else {
                        crate::contracts::record_building_failure(3, frame.addr.cell.level);
                        result.complete = false;
                        continue 'features;
                    };
                    if let Some(trace) = &mut roof_trace {
                        trace.record.work.triangles = body_roof.triangles.len() as u64;
                    }
                    drop(roof_trace);
                    roof_triangles += body_roof.triangles.len();
                    if roof_triangles > MAX_ROOF_TRIANGLES {
                        crate::contracts::record_building_failure(6, frame.addr.cell.level);
                        result.complete = false;
                        continue 'features;
                    }
                    let wall = crate::building_appearance(
                        (floor + body_eave) * 0.5,
                        floor,
                        body_eave,
                        &solid_wall_stamp,
                        None,
                        0.01,
                    );
                    let first_roof_index = mesh.indices.len();
                    let first_roof_vertex = mesh.vertices.len();
                    emit_roof(&projection, &mut mesh, &body_roof, roof_appearance);
                    if coarse_pixel_m.is_some() && roof.top > body_eave {
                        seal_coarse_roof_cuts(
                            frame,
                            &mut mesh,
                            first_roof_index,
                            body_eave,
                            roof_appearance,
                        );
                    }
                    apply_finish_variant(
                        &mut mesh.vertices[first_roof_vertex..],
                        stamp.design.roof_variant,
                    );
                    if emit_collision_prism(
                        &projection,
                        &mut collision,
                        &body,
                        domain
                            .coarse_cache
                            .as_ref()
                            .map(|_| body_boundary.as_slice()),
                        base,
                        cap,
                    )
                    .is_none()
                    {
                        crate::contracts::record_building_failure(4, frame.addr.cell.level);
                        result.complete = false;
                    }
                    if let Some(flat_roof) = flat_roof {
                        emit_flat_parapets(
                            &projection,
                            &mut mesh,
                            &body,
                            &body_boundary,
                            &source.rings,
                            flat_roof,
                            wall,
                            facade_trim(&stamp),
                        );
                    }
                    for edge in &body_roof.boundary {
                        let first_closure = mesh.vertices.len();
                        emit_roof_closure(&projection, &mut mesh, edge, body_eave, wall);
                        if wall.material == stamp.design.wall_material {
                            apply_finish_variant(
                                &mut mesh.vertices[first_closure..],
                                stamp.design.wall_variant,
                            );
                        }
                        if coarse_pixel_m.is_none()
                            && cap_top.is_none()
                            && !stamp.lidar
                            && source.roof_shape.is_none()
                        {
                            emit_gable_crown(
                                &projection,
                                &mut mesh,
                                edge,
                                &roof,
                                &source.rings,
                                &body,
                                &stamp,
                                wall,
                                *facade_resolution,
                            );
                        }
                    }
                } else if emit_collision_prism(
                    &projection,
                    &mut collision,
                    &body,
                    domain
                        .coarse_cache
                        .as_ref()
                        .map(|_| body_boundary.as_slice()),
                    base,
                    cap,
                )
                .is_none()
                {
                    crate::contracts::record_building_failure(4, frame.addr.cell.level);
                    result.complete = false;
                    continue 'features;
                }
            }
        }
        if city_fixtures::has_fixtures(source.seed) {
            city_fixtures::append(
                &projection,
                &mut mesh,
                source.seed,
                boundary,
                datum.floor,
                top,
            );
        }
        let opaque = PRUNE_HIDDEN_WALLS
            && result.complete
            && source.kind == BuildingKind::Building
            && source.min_height_m <= 0.0
            && source.rings.len() == 1
            && overlaps[i].is_empty()
            && !stamp.lidar
            && eave > base
            && mandatory_envelope_inside_cell(&mesh, eligible_columns);
        let foundation = opaque.then(|| foundation_from_collision(&collision, wall));
        let mut shadow_walls = facade.shadow_walls.take();
        mesh = clip_to_eligible_recorded(mesh, eligible_columns, Some(&mut shadow_walls));
        collision = clip_to_eligible(collision, eligible_columns);
        details = clip_to_eligible(details, eligible_columns);
        if mesh.vertices.is_empty() {
            continue;
        }
        let (lo, hi) = source_bounds[i].unwrap();
        let bound_center = projection
            .point((lo + hi) * 0.5, (base + top) * 0.5)
            .as_vec3()
            .to_array();
        let radius_m = (((hi - lo) * 0.5).length_squared() + ((top - base) * 0.5).powi(2)).sqrt()
            as f32
            + if stamp.lidar { 41.0 } else { 2.1 };
        if !details.vertices.is_empty() {
            result.details.push(BuildingDetail {
                mesh: details,
                bound_center,
                radius_m,
                // Admission uses the building's projected diameter. Convert the
                // generation's relief cutoff to that same diameter and require
                // two pixels at that scale, even when a saturated terrain cut
                // keeps a fine mesh far from the camera.
                min_screen_px: detail_screen_threshold(radius_m, *facade_resolution),
            });
        }
        result
            .collision
            .extend(crate::collision::triangles_from_mesh(&collision, frame));
        let shadow_omissions = prove_shadow_omissions(&mesh, frame, &shadow_walls);
        append_mesh_with_shadows(
            &mut result.mesh,
            &mut result.shadow_omissions,
            mesh,
            shadow_omissions,
        );
        // Only a successfully emitted, wholly resident mandatory envelope can
        // hide a later owner's shared wall. Source/budget order stays intact.
        if let (Some(core), Some(foundation)) = (facade.opaque_core.take(), foundation) {
            let mut core = core.into_inner();
            core.floor_y = foundation
                .vertices
                .iter()
                .map(|vertex| f64::from(vertex.pos[1]))
                .max_by(f64::total_cmp);
            if !core.invalid
                && core.floor_y.is_some()
                && core.roof_y.is_some()
                && mandatory_envelope_inside_cell(&foundation, eligible_columns)
            {
                opaque_walls.insert(&boundary, &source.rings, base, eave, core, foundation);
            }
        }
        // Stop the expensive pass as soon as a whole-cell simpler envelope
        // is required. The retry retains the same source rings and datums.
        if !simple && crate::built_surface::mesh_bytes(&result.mesh) > budget {
            break;
        }
    }
    opaque_walls.finish(&mut result.mesh);
    for tower in &overlay.source_towers {
        if let Some(mesh) = observation_tower_mesh(&projection, *tower, elevation, simple) {
            let mesh = clip_to_eligible(mesh, eligible_columns);
            result
                .collision
                .extend(crate::collision::triangles_from_mesh(&mesh, frame));
            append_mesh(&mut result.mesh, mesh);
        } else {
            crate::contracts::record_building_failure(5, frame.addr.cell.level);
            result.complete = false;
        }
    }
    (!result.mesh.vertices.is_empty() || !result.complete).then_some(result)
}

fn observation_tower_mesh(
    projection: &Projection<'_>,
    tower: tier0::ObservationTowerSpec,
    elevation: &dyn Fn(f64, f64, f32) -> Option<f32>,
    simple: bool,
) -> Option<MeshBuffers> {
    let center = DVec2::from_array(tower.centre_en.map(f64::from));
    let (lat, lon) = projection.latlon(center);
    let base =
        f64::from(elevation(lat, lon, crate::datum_pitch_m(lat) as f32).filter(|h| h.is_finite())?);
    let profile = crate::observation_towers::observation_tower_profile(tower.height_m);
    let total = f64::from(profile.height_m);
    let shaft_base = f64::from(profile.shaft_base_radius_m);
    let shaft_top = f64::from(profile.shaft_top_radius_m);
    let pod = f64::from(profile.pod_radius_m);
    let mut mesh = MeshBuffers::default();
    let cuts = [
        0.0,
        0.045,
        0.12,
        0.675,
        0.6912,
        0.6975,
        0.7434,
        0.7506,
        0.765,
        0.780,
        0.825,
        1.0 - 1.0 / total,
        1.0,
    ];
    for pair in cuts.windows(2) {
        let middle = (pair[0] + pair[1]) * 0.5;
        let radius = |fraction: f64| {
            if middle >= 1.0 - 1.0 / total {
                0.9
            } else if middle < 0.045 {
                shaft_base * 1.22
            } else if middle < 0.675 {
                let t = ((fraction - 0.045) / 0.630).clamp(0.0, 1.0);
                shaft_base + (shaft_top - shaft_base) * (1.0 - (1.0 - t).powf(1.55))
            } else if middle < 0.765 {
                pod * (0.88 + (((fraction - 0.675) / 0.09) * std::f64::consts::PI).sin() * 0.12)
            } else if middle < 0.780 {
                pod + 1.25
            } else if middle < 0.825 {
                shaft_top
            } else {
                (shaft_top * (1.0 - ((fraction - 0.825) / 0.175) * 0.72)).max(0.7)
            }
        };
        let (material, color) = if middle >= 1.0 - 1.0 / total {
            (crate::material::AVIATION_BEACON, [224, 24, 18])
        } else if middle < 0.12 {
            (crate::material::WALL, [151, 157, 158])
        } else if middle < 0.675 || (0.780..0.825).contains(&middle) {
            (crate::material::WALL, [176, 179, 176])
        } else if (0.6975..0.7434).contains(&middle) {
            (crate::material::WINDOW, [78, 122, 149])
        } else if (0.6912..0.7506).contains(&middle) {
            (crate::material::WALL, [151, 157, 158])
        } else {
            (crate::material::STEEL, [128, 139, 143])
        };
        let appearance = SurfaceAppearance {
            material,
            tint: crate::art_direction::architecture_surface(material, color),
        };
        let segments = if simple { 16 } else { 32 };
        let steps =
            (((pair[1] - pair[0]) * total / if simple { 8.0 } else { 3.0 }).ceil() as usize).max(1);
        let ring = |fraction: f64, sector: usize| {
            let angle = sector as f64 / segments as f64 * std::f64::consts::TAU;
            projection.point(
                center + DVec2::new(angle.cos(), angle.sin()) * radius(fraction),
                base + fraction * total,
            )
        };
        for step in 0..steps {
            let lo = pair[0] + (pair[1] - pair[0]) * step as f64 / steps as f64;
            let hi = pair[0] + (pair[1] - pair[0]) * (step + 1) as f64 / steps as f64;
            for sector in 0..segments {
                emit_quad(
                    &mut mesh,
                    [
                        ring(lo, sector),
                        ring(lo, sector + 1),
                        ring(hi, sector + 1),
                        ring(hi, sector),
                    ],
                    appearance,
                    false,
                );
            }
        }
        for (fraction, up) in [(pair[0], false), (pair[1], true)] {
            let start = mesh.indices.len();
            for sector in 0..segments {
                emit_triangle(
                    &mut mesh,
                    [
                        projection.point(center, base + fraction * total),
                        ring(fraction, sector),
                        ring(fraction, sector + 1),
                    ],
                    appearance,
                    true,
                );
            }
            if !up {
                for triangle in mesh.indices[start..].chunks_exact_mut(3) {
                    triangle.swap(1, 2);
                }
            }
        }
    }
    Some(mesh)
}

pub(crate) fn composition_parents(buildings: &[Building]) -> Vec<Option<usize>> {
    if !buildings.iter().any(|b| b.kind.is_part()) {
        return vec![None; buildings.len()];
    }
    let boxes: Vec<_> = buildings.iter().map(|b| bounds(&b.rings)).collect();
    let neighbors = overlapping_sources(&boxes);
    buildings
        .iter()
        .enumerate()
        .map(|(i, source)| {
            if !source.kind.is_part() {
                return None;
            }
            neighbors[i]
                .iter()
                .copied()
                .filter(|&j| {
                    let parent = &buildings[j];
                    if parent.kind != BuildingKind::Building {
                        return false;
                    }
                    let (Some((lo, hi)), Some((plo, phi))) = (boxes[i], boxes[j]) else {
                        return false;
                    };
                    if !(lo + DVec2::splat(0.001)).cmpge(plo).all()
                        || !(hi - DVec2::splat(0.001)).cmple(phi).all()
                    {
                        return false;
                    }
                    // Vertex containment alone admits bridges across concavities and
                    // parts that fill a parent's courtyard. Allow only the same millimetre
                    // boundary tolerance as the bounds test, including projection roundoff.
                    Footprint::new(vec![&source.rings], vec![&parent.rings])
                        .and_then(|outside| outside.roof_cells(4096.0))
                        .is_some_and(|cells| {
                            cells.iter().map(cell_area).sum::<f64>()
                                <= 0.001
                                    * source
                                        .rings
                                        .iter()
                                        .map(|ring| {
                                            (0..ring.len())
                                                .map(|k| {
                                                    let a =
                                                        DVec2::from_array(ring[k].map(f64::from));
                                                    let b = DVec2::from_array(
                                                        ring[(k + 1) % ring.len()].map(f64::from),
                                                    );
                                                    a.distance(b)
                                                })
                                                .sum::<f64>()
                                        })
                                        .sum::<f64>()
                        })
                })
                .min_by(|&a, &b| {
                    let area = |j: usize| {
                        boxes[j].map_or(f64::INFINITY, |(lo, hi)| (hi - lo).element_product())
                    };
                    area(a)
                        .total_cmp(&area(b))
                        .then(buildings[a].seed.cmp(&buildings[b].seed))
                })
        })
        .collect()
}

fn cell_area(cell: &[DVec2; 4]) -> f64 {
    ((cell[1] - cell[0]).perp_dot(cell[2] - cell[0]).abs()
        + (cell[2] - cell[0]).perp_dot(cell[3] - cell[0]).abs())
        * 0.5
}

pub(crate) fn outline_fallback_height(source: &Building, parts: &[&Building]) -> f32 {
    if source.height_m < 28.0 || source.min_height_m > 0.0 {
        return source.height_m;
    }
    let ground_parts: Vec<_> = parts
        .iter()
        .copied()
        .filter(|p| {
            p.kind == BuildingKind::Part
                && p.min_height_m == 0.0
                && p.height_m.is_finite()
                && p.height_m > 0.0
        })
        .collect();
    let Some(lowest) = ground_parts
        .iter()
        .map(|p| p.height_m)
        .min_by(f32::total_cmp)
    else {
        return source.height_m;
    };
    if source.height_m - lowest < 8.0 {
        return source.height_m;
    }
    let Some(residual) = Footprint::new(
        vec![&source.rings],
        ground_parts.iter().map(|p| &p.rings).collect(),
    ) else {
        return source.height_m;
    };
    let Some(cells) = residual.roof_cells(4096.0) else {
        return source.height_m;
    };
    let area: f64 = cells.iter().map(cell_area).sum();
    let source_area = f64::from(crate::tier0::building_metrics(source).area_m2);
    if area <= 1.0e-4 || area > source_area * 0.02 {
        return source.height_m;
    }
    let perimeter: f64 = residual
        .boundary()
        .iter()
        .map(|edge| edge[0].distance(edge[1]))
        .sum();
    // Near-complete part coverage and sub-metre residual depth identify outline
    // registration slivers. Retain their footprint as a base at the lowest mapped
    // ground part's roof; never alter an authored part or a substantial remainder.
    if perimeter > 0.0 && 2.0 * area / perimeter <= 0.75 {
        lowest
    } else {
        source.height_m
    }
}

fn wins(
    candidate_index: usize,
    candidate: &Building,
    current_index: usize,
    current: &Building,
) -> bool {
    if candidate.kind.is_part() != current.kind.is_part() {
        return candidate.kind.is_part();
    }
    candidate.height_m > current.height_m
        || (candidate.height_m == current.height_m && candidate_index < current_index)
}
fn overlapping_sources(bounds: &[Option<(DVec2, DVec2)>]) -> Vec<Vec<usize>> {
    let mut order: Vec<_> = bounds
        .iter()
        .enumerate()
        .filter_map(|(i, b)| b.map(|_| i))
        .collect();
    order.sort_unstable_by(|&a, &b| {
        bounds[a]
            .unwrap()
            .0
            .x
            .total_cmp(&bounds[b].unwrap().0.x)
            .then(a.cmp(&b))
    });
    let mut active = Vec::<usize>::new();
    let mut result = vec![Vec::new(); bounds.len()];
    for i in order {
        let (lo, hi) = bounds[i].unwrap();
        active.retain(|&j| bounds[j].unwrap().1.x > lo.x);
        for &j in &active {
            let (other_lo, other_hi) = bounds[j].unwrap();
            if lo.y < other_hi.y && other_lo.y < hi.y {
                result[i].push(j);
                result[j].push(i);
            }
        }
        active.push(i);
    }
    for neighbors in &mut result {
        neighbors.sort_unstable();
    }
    result
}
fn append_mesh(target: &mut MeshBuffers, source: MeshBuffers) {
    let base = target.vertices.len() as u32;
    target.vertices.extend(source.vertices);
    target
        .indices
        .extend(source.indices.into_iter().map(|i| i + base));
}

fn append_mesh_with_shadows(
    target: &mut MeshBuffers,
    target_omissions: &mut Vec<crate::ShadowOmission>,
    source: MeshBuffers,
    omissions: Vec<crate::ShadowOmission>,
) {
    let index_base = target.indices.len();
    for mut omission in omissions {
        if let Some(first) = usize::try_from(omission.first_index)
            .ok()
            .and_then(|first| first.checked_add(index_base))
            .and_then(|first| u32::try_from(first).ok())
        {
            omission.first_index = first;
            target_omissions.push(omission);
        }
    }
    append_mesh(target, source);
}

// A fixed sixteen-face cone shares the planar roof path. These normals also
// define its height field, so clipping, wall tops and collision agree exactly.
const CONE_NORMALS: [DVec2; 16] = [
    DVec2::new(1.0, 0.0),
    DVec2::new(0.9238795325112867, 0.3826834323650898),
    DVec2::new(
        std::f64::consts::FRAC_1_SQRT_2,
        std::f64::consts::FRAC_1_SQRT_2,
    ),
    DVec2::new(0.3826834323650898, 0.9238795325112867),
    DVec2::new(0.0, 1.0),
    DVec2::new(-0.3826834323650898, 0.9238795325112867),
    DVec2::new(
        -std::f64::consts::FRAC_1_SQRT_2,
        std::f64::consts::FRAC_1_SQRT_2,
    ),
    DVec2::new(-0.9238795325112867, 0.3826834323650898),
    DVec2::new(-1.0, 0.0),
    DVec2::new(-0.9238795325112867, -0.3826834323650898),
    DVec2::new(
        -std::f64::consts::FRAC_1_SQRT_2,
        -std::f64::consts::FRAC_1_SQRT_2,
    ),
    DVec2::new(-0.3826834323650898, -0.9238795325112867),
    DVec2::new(0.0, -1.0),
    DVec2::new(0.3826834323650898, -0.9238795325112867),
    DVec2::new(
        std::f64::consts::FRAC_1_SQRT_2,
        -std::f64::consts::FRAC_1_SQRT_2,
    ),
    DVec2::new(0.9238795325112867, -0.3826834323650898),
];

struct RoofField {
    edges: Vec<RoofEdge>,
    gable_edges: Vec<usize>,
    distance_index: Option<RoofDistanceIndex>,
    convex_distances: Option<ConvexRoofDistances>,
    // Squared nearest-edge cutoffs for the plain and piecewise profiles. Both
    // are cached because regional tiering is selected after construction.
    distance_caps: [f64; 2],
    bounds: [DVec2; 2],
    reentrant: bool,
    origin: DVec2,
    direction: DVec2,
    shape: Option<RoofShape>,
    eave: f64,
    top: f64,
    pitch: f64,
    tiered: bool,
    conical: bool,
}

struct RoofEdge {
    points: [DVec2; 2],
    delta: DVec2,
    length_squared: f64,
    length: f64,
}

fn roof_distance_caps(eave: f64, top: f64, short_span: f64, pitch: f64) -> [f64; 2] {
    let rise = (top - eave).max(0.0);
    if !eave.is_finite() || !top.is_finite() || !rise.is_finite() || rise <= 0.0 {
        return [f64::INFINITY; 2];
    }
    let cap = |distance: f64, saturated: &dyn Fn(f64) -> bool| {
        if !distance.is_finite() || distance < 0.0 {
            return f64::INFINITY;
        }
        let distance = distance.next_up();
        let squared = (distance * distance).next_up();
        // The sentinel must already saturate using the exact operations below,
        // including sqrt rounding. Otherwise retain the unrestricted search.
        if squared.is_finite() && saturated(squared.sqrt()) {
            squared
        } else {
            f64::INFINITY
        }
    };
    let plain = if pitch.is_finite() && pitch > 0.0 {
        cap(rise / pitch, &|distance| distance * pitch >= rise)
    } else {
        f64::INFINITY
    };
    let piecewise = if short_span.is_finite() && short_span > 0.0 {
        cap(short_span * 0.5, &|distance| {
            distance * 2.0 / short_span >= 1.0
        })
    } else {
        f64::INFINITY
    };
    [plain, piecewise]
}
struct RoofSupportLine {
    normal: DVec2,
    offset: f64,
}
struct ConvexRoofDistances {
    lines: Vec<RoofSupportLine>,
    coordinate_scale: f64,
}
impl ConvexRoofDistances {
    fn new(edges: &[RoofEdge], single_ring: bool) -> Option<Self> {
        if !single_ring || !(3..=64).contains(&edges.len()) {
            return None;
        }
        let coordinate_scale = edges
            .iter()
            .flat_map(|edge| edge.points)
            .map(|point| point.abs().max_element())
            .fold(1.0, f64::max);
        if !coordinate_scale.is_finite() {
            return None;
        }
        let area: f64 = edges
            .iter()
            .map(|edge| edge.points[0].perp_dot(edge.points[1]))
            .sum();
        if !area.is_finite() || area == 0.0 {
            return None;
        }
        let margin = coordinate_scale * f64::EPSILON * 256.0;
        let mut lines = Vec::with_capacity(edges.len());
        for (index, edge) in edges.iter().enumerate() {
            if edge.points[1] != edges[(index + 1) % edges.len()].points[0] {
                return None;
            }
            let normal = DVec2::new(-edge.delta.y, edge.delta.x) * (area.signum() / edge.length);
            let offset = -normal.dot(edge.points[0]);
            if !normal.is_finite() || !offset.is_finite() {
                return None;
            }
            // Strictly interior non-endpoint vertices prove a single convex
            // cycle. Collinear, nearly degenerate and uncertain rings retain
            // the segment index, as do every courtyard and disconnected ring.
            for (other, candidate) in edges.iter().enumerate() {
                if other != index
                    && other != (index + 1) % edges.len()
                    && normal.dot(candidate.points[0]) + offset <= margin
                {
                    return None;
                }
            }
            lines.push(RoofSupportLine { normal, offset });
        }
        Some(Self {
            lines,
            coordinate_scale,
        })
    }

    fn nearest_squared(&self, edges: &[RoofEdge], point: DVec2) -> Option<f64> {
        let scale = self.coordinate_scale.max(point.abs().max_element());
        let margin = scale * f64::EPSILON * 256.0;
        let mut nearest = 0;
        let mut distance = f64::INFINITY;
        for (index, line) in self.lines.iter().enumerate() {
            let candidate = line.normal.dot(point) + line.offset;
            if !candidate.is_finite() || candidate <= margin {
                // Boundary and exterior evaluations retain their original
                // segment/vertex regions, including snapped eave endpoints.
                return None;
            }
            if candidate < distance {
                nearest = index;
                distance = candidate;
            }
        }
        let mut best = edges[nearest].distance_squared(point);
        for (index, line) in self.lines.iter().enumerate() {
            if index == nearest {
                continue;
            }
            // Supporting-line distance is a lower bound on segment distance.
            // Include the rounding of normalization, dot/offset arithmetic and
            // the original clamped segment lerp in an outward error allowance.
            // Only candidate selection changes: all possible minima still use
            // the original expression, preserving its final floating bits.
            let lower = (line.normal.dot(point) + line.offset - margin).max(0.0);
            if (lower * lower).next_down() <= best {
                best = best.min(edges[index].distance_squared(point));
            }
        }
        Some(best)
    }
}
struct RoofDistanceNode {
    bounds: [DVec2; 2],
    children: Option<[usize; 2]>,
    edges: std::ops::Range<usize>,
}

struct RoofDistanceIndex {
    order: Vec<usize>,
    nodes: Vec<RoofDistanceNode>,
}
impl RoofDistanceIndex {
    fn new(edges: &[RoofEdge]) -> Self {
        let mut index = Self {
            order: (0..edges.len()).collect(),
            nodes: Vec::with_capacity(edges.len()),
        };
        index.build(edges, 0..edges.len());
        index
    }

    fn build(&mut self, edges: &[RoofEdge], range: std::ops::Range<usize>) -> usize {
        let mut lo = DVec2::splat(f64::INFINITY);
        let mut hi = DVec2::splat(f64::NEG_INFINITY);
        for &id in &self.order[range.clone()] {
            for point in edges[id].points {
                lo = lo.min(point);
                hi = hi.max(point);
            }
        }
        // A closest-point lerp can round just outside its endpoint extent.
        // Inflating the broad phase keeps those exact evaluations eligible.
        let margin = lo.abs().max(hi.abs()).max_element().max(1.0) * f64::EPSILON * 64.0;
        let node = self.nodes.len();
        self.nodes.push(RoofDistanceNode {
            bounds: [
                (lo - DVec2::splat(margin)).map(f64::next_down),
                (hi + DVec2::splat(margin)).map(f64::next_up),
            ],
            children: None,
            edges: range.clone(),
        });
        if range.len() > 4 {
            let axis = usize::from((hi - lo).y > (hi - lo).x);
            self.order[range.clone()].sort_unstable_by(|&a, &b| {
                let midpoint = |id: usize| edges[id].points[0][axis] + edges[id].points[1][axis];
                midpoint(a).total_cmp(&midpoint(b)).then(a.cmp(&b))
            });
            let mid = range.start + range.len() / 2;
            let left = self.build(edges, range.start..mid);
            let right = self.build(edges, mid..range.end);
            self.nodes[node].children = Some([left, right]);
        }
        node
    }

    fn lower_bound(&self, node: usize, point: DVec2) -> f64 {
        let [lo, hi] = self.nodes[node].bounds;
        (lo - point)
            .max(point - hi)
            .max(DVec2::ZERO)
            .length_squared()
    }

    fn nearest(&self, edges: &[RoofEdge], point: DVec2, node: usize, best: &mut f64) {
        self.nearest_with_bound(edges, point, node, self.lower_bound(node, point), best);
    }

    fn nearest_with_bound(
        &self,
        edges: &[RoofEdge],
        point: DVec2,
        node: usize,
        lower_bound: f64,
        best: &mut f64,
    ) {
        if lower_bound > *best {
            return;
        }
        let current = &self.nodes[node];
        if let Some([a, b]) = current.children {
            let bounds = [self.lower_bound(a, point), self.lower_bound(b, point)];
            let [(first, first_bound), (second, second_bound)] = if bounds[0] <= bounds[1] {
                [(a, bounds[0]), (b, bounds[1])]
            } else {
                [(b, bounds[1]), (a, bounds[0])]
            };
            self.nearest_with_bound(edges, point, first, first_bound, best);
            self.nearest_with_bound(edges, point, second, second_bound, best);
        } else {
            for &id in &self.order[current.edges.clone()] {
                *best = best.min(edges[id].distance_squared(point));
            }
        }
    }
}

impl RoofEdge {
    fn distance_squared(&self, point: DVec2) -> f64 {
        let t = ((point - self.points[0]).dot(self.delta) / self.length_squared).clamp(0.0, 1.0);
        point.distance_squared(self.points[0].lerp(self.points[1], t))
    }
}

impl RoofField {
    fn new(rings: &Rings, shape: Option<RoofShape>, eave: f64, top: f64, pitch: f32) -> Self {
        let mut direction = DVec2::X;
        let mut length_cm = 0_u64;
        let mut reentrant = rings.len() > 1;
        for ring in rings {
            let points: Vec<_> = ring
                .iter()
                .map(|p| DVec2::from_array(p.map(f64::from)))
                .collect();
            let area: f64 = (0..points.len())
                .map(|i| points[i].perp_dot(points[(i + 1) % points.len()]))
                .sum();
            for i in 0..points.len() {
                let a = points[i];
                let b = points[(i + 1) % points.len()];
                let previous = points[(i + points.len() - 1) % points.len()];
                reentrant |= (a - previous).perp_dot(b - a) * area < -1.0e-8;
                let candidate_cm = (a.distance(b) * 100.0).round() as u64;
                if candidate_cm > length_cm {
                    length_cm = candidate_cm;
                    direction = (b - a).normalize();
                }
            }
        }
        let (lo, hi) = bounds(rings).unwrap();
        let origin = (lo + hi) * 0.5;
        let rotate = |p: [f32; 2]| {
            let p = DVec2::from_array(p.map(f64::from)) - origin;
            DVec2::new(p.dot(direction), direction.perp_dot(p))
        };
        let mut edges = Vec::new();
        let mut rotated_bounds = [DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY)];
        for ring in rings {
            for i in 0..ring.len() {
                let a = rotate(ring[i]);
                let b = rotate(ring[(i + 1) % ring.len()]);
                rotated_bounds[0] = rotated_bounds[0].min(a);
                rotated_bounds[1] = rotated_bounds[1].max(a);
                if a.distance_squared(b) > 1.0e-12 {
                    let delta = b - a;
                    edges.push(RoofEdge {
                        points: [a, b],
                        delta,
                        length_squared: delta.length_squared(),
                        length: delta.length(),
                    });
                }
            }
        }
        let gable_edges = edges
            .iter()
            .enumerate()
            .filter_map(|(i, edge)| (edge.delta.x.abs() >= edge.delta.y.abs()).then_some(i))
            .collect();
        // A short segment scan beats the support shortlist on small rings;
        // large rings benefit more from logarithmic spatial pruning.
        let convex_distances = ((12..=16).contains(&edges.len())
            && matches!(
                shape,
                Some(RoofShape::Hipped | RoofShape::Pyramidal | RoofShape::Mansard)
            ))
        .then(|| ConvexRoofDistances::new(&edges, rings.len() == 1))
        .flatten();
        let distance_index = (edges.len() >= 12
            && !matches!(
                shape,
                Some(RoofShape::Flat | RoofShape::Skillion | RoofShape::Dome | RoofShape::Onion)
            ))
        .then(|| RoofDistanceIndex::new(&edges));
        let rise = (top - eave).max(0.0);
        let short_span = (rotated_bounds[1].y - rotated_bounds[0].y).max(0.01);
        let effective_pitch = if reentrant {
            f64::from(pitch)
        } else {
            f64::from(pitch).min(rise * 2.0 / short_span)
        };
        Self {
            edges,
            gable_edges,
            distance_index,
            convex_distances,
            distance_caps: roof_distance_caps(eave, top, short_span, effective_pitch),
            bounds: rotated_bounds,
            reentrant,
            origin,
            direction,
            shape,
            eave,
            top,
            pitch: f64::from(pitch),
            tiered: false,
            conical: false,
        }
    }
    fn rotate(&self, p: DVec2) -> DVec2 {
        let delta = p - self.origin;
        DVec2::new(delta.dot(self.direction), self.direction.perp_dot(delta))
    }
    fn unrotate(&self, p: DVec2) -> DVec2 {
        self.origin + self.direction * p.x + DVec2::new(-self.direction.y, self.direction.x) * p.y
    }

    fn planes(&self) -> Option<Vec<RoofPlane>> {
        if self.reentrant
            || self.tiered
            || !(3..=64).contains(&self.edges.len())
            || !matches!(
                self.shape,
                Some(
                    RoofShape::Gabled
                        | RoofShape::Hipped
                        | RoofShape::Pyramidal
                        | RoofShape::Mansard
                        | RoofShape::Gambrel
                        | RoofShape::Skillion
                )
            )
        {
            return None;
        }
        let area: f64 = self
            .edges
            .iter()
            .map(|edge| edge.points[0].perp_dot(edge.points[1]))
            .sum();
        if !area.is_finite() || area.abs() < 1.0e-8 {
            return None;
        }
        let winding = area.signum();
        // The distance to a convex polygon's boundary is the minimum inward
        // supporting-line distance. Prove that domain here; a concave corner
        // or courtyard instead has segment/vertex regions requiring sampling.
        for edge in &self.edges {
            if self.edges.iter().any(|other| {
                winding * edge.delta.perp_dot(other.points[0] - edge.points[0])
                    < -edge.length * 1.0e-8
            }) {
                return None;
            }
        }
        let rise = (self.top - self.eave).max(0.0);
        let short_span = (self.bounds[1].y - self.bounds[0].y).max(0.01);
        if !rise.is_finite() || rise <= 0.0 || !short_span.is_finite() {
            return None;
        }
        let perpendicular = DVec2::new(-self.direction.y, self.direction.x);
        if self.conical {
            let half = (self.bounds[1] - self.bounds[0]) * 0.5;
            if half.min_element() <= 0.0 {
                return None;
            }
            let center = self.unrotate((self.bounds[0] + self.bounds[1]) * 0.5);
            return Some(
                CONE_NORMALS
                    .iter()
                    .map(|normal| {
                        let gradient = -rise * *normal / half;
                        RoofPlane {
                            origin: center,
                            gradient: self.direction * gradient.x + perpendicular * gradient.y,
                            height: self.top,
                        }
                    })
                    .collect(),
            );
        }
        if self.shape == Some(RoofShape::Skillion) {
            // Every source point lies within these rotated bounds, so neither
            // height clamp contributes an interior region of the slope.
            let slope = rise / short_span;
            return Some(vec![RoofPlane {
                origin: self.origin,
                gradient: perpendicular * slope,
                height: self.eave - self.bounds[0].y * slope,
            }]);
        }
        let broken_slope = matches!(self.shape, Some(RoofShape::Mansard | RoofShape::Gambrel));
        let profile = if broken_slope {
            // The source profile has a steep lower slope, then a shallower
            // upper slope at q=.28 and h=.78. Both slopes are positive and
            // decreasing, so applying it to min(edge distances) is exactly
            // the lower envelope of both affine slopes on every source edge.
            // Pitch is intentionally absent: these two shapes use roof rise.
            [
                (rise * 2.0 / short_span * (0.78 / 0.28), 0.0),
                (
                    rise * 2.0 / short_span * (0.22 / 0.72),
                    rise * (0.78 - 0.28 * 0.22 / 0.72),
                ),
            ]
        } else {
            let pitch = self.pitch.min(rise * 2.0 / short_span);
            if !pitch.is_finite() || pitch <= 0.0 {
                return None;
            }
            [(pitch, 0.0), (pitch, 0.0)]
        };
        let gabled = matches!(self.shape, Some(RoofShape::Gabled | RoofShape::Gambrel));
        let mut planes: Vec<RoofPlane> = Vec::new();
        for edge in &self.edges {
            if gabled && edge.delta.x.abs() < edge.delta.y.abs() {
                continue;
            }
            for &(slope, intercept) in &profile[..if broken_slope { 2 } else { 1 }] {
                let gradient =
                    DVec2::new(-edge.delta.y, edge.delta.x) * (winding * slope / edge.length);
                let plane = RoofPlane {
                    origin: self.origin,
                    gradient: self.direction * gradient.x + perpendicular * gradient.y,
                    height: self.eave + intercept - gradient.dot(edge.points[0]),
                };
                // Collinear source edges can describe the same supporting
                // plane. Retain its first owner, including repeated profiles.
                if !planes.iter().any(|other| {
                    plane.gradient.distance_squared(other.gradient) < 1.0e-24
                        && (plane.height - other.height).abs() < 1.0e-9
                }) {
                    planes.push(plane);
                }
            }
        }
        if planes.is_empty() {
            return None;
        }
        planes.push(RoofPlane {
            origin: self.origin,
            gradient: DVec2::ZERO,
            height: self.top,
        });
        Some(planes)
    }

    fn height_bounds(&self) -> Option<[f64; 2]> {
        let top = self.eave + (self.top - self.eave).max(0.0);
        (self.eave.is_finite() && top.is_finite()).then(|| [self.eave.next_down(), top.next_up()])
    }

    fn height(&self, p: DVec2) -> f64 {
        self.height_with_distance_cap::<true>(p)
    }

    fn height_with_distance_cap<const CAP_DISTANCE: bool>(&self, p: DVec2) -> f64 {
        let p = self.rotate(p);
        let rise = (self.top - self.eave).max(0.0);
        let short_span = (self.bounds[1].y - self.bounds[0].y).max(0.01);
        if self.conical {
            let [lo, hi] = self.bounds;
            let radial = (p - (lo + hi) * 0.5) / ((hi - lo) * 0.5);
            let q = CONE_NORMALS
                .iter()
                .map(|normal| normal.dot(radial))
                .fold(0.0, f64::max);
            return self.eave + rise * (1.0 - q).clamp(0.0, 1.0);
        }
        let height = match self.shape {
            Some(
                shape @ (RoofShape::Gabled
                | RoofShape::Hipped
                | RoofShape::Pyramidal
                | RoofShape::Mansard
                | RoofShape::Gambrel),
            ) => {
                let gabled =
                    matches!(shape, RoofShape::Gabled | RoofShape::Gambrel) && !self.reentrant;
                // A roof must be defined on its boundary as well as inside it.
                // Source-edge distances give continuous hip wings; gables use
                // the supporting planes of their long sides, retaining their
                // ridge at either end without cross-section membership tests.
                let distance = if gabled {
                    self.gable_edges
                        .iter()
                        .map(|&i| {
                            let edge = &self.edges[i];
                            edge.delta.perp_dot(p - edge.points[0]).abs() / edge.length
                        })
                        .min_by(f64::total_cmp)
                        .unwrap_or(0.0)
                } else {
                    // Square root is monotonic: selecting the closest squared
                    // distance first preserves the resulting height bits.
                    let squared = if let Some(squared) = self
                        .convex_distances
                        .as_ref()
                        .and_then(|convex| convex.nearest_squared(&self.edges, p))
                    {
                        squared
                    } else if let Some(index) = &self.distance_index {
                        let mut best = if CAP_DISTANCE {
                            self.distance_caps[usize::from(
                                self.tiered
                                    || matches!(shape, RoofShape::Mansard | RoofShape::Gambrel),
                            )]
                        } else {
                            f64::INFINITY
                        };
                        index.nearest(&self.edges, p, 0, &mut best);
                        best
                    } else {
                        self.edges
                            .iter()
                            .map(|edge| edge.distance_squared(p))
                            .min_by(f64::total_cmp)
                            .unwrap_or(0.0)
                    };
                    squared.sqrt()
                };
                let pitch = if self.reentrant {
                    self.pitch
                } else {
                    self.pitch.min(rise * 2.0 / short_span)
                };
                if self.tiered {
                    // Broad horizontal returns separate three pitched tiers.
                    // One continuous source-owned shell retains the exact base
                    // outline and top, including irregular courtyard wings.
                    let q = (distance * 2.0 / short_span).clamp(0.0, 1.0);
                    let profile = if q < 0.28 {
                        q / 0.28 * 0.34
                    } else if q < 0.35 {
                        0.34
                    } else if q < 0.63 {
                        0.34 + (q - 0.35) / 0.28 * 0.36
                    } else if q < 0.70 {
                        0.70
                    } else {
                        0.70 + (q - 0.70) / 0.30 * 0.30
                    };
                    rise * profile
                } else if matches!(shape, RoofShape::Mansard | RoofShape::Gambrel) {
                    // A steep lower roof and shallower upper plane share the
                    // same source boundary, ridge direction and reported top.
                    let q = (distance * 2.0 / short_span).clamp(0.0, 1.0);
                    rise * if q < 0.28 {
                        q / 0.28 * 0.78
                    } else {
                        0.78 + (q - 0.28) / 0.72 * 0.22
                    }
                } else {
                    distance * pitch
                }
            }
            Some(RoofShape::Skillion) => {
                rise * ((p.y - self.bounds[0].y) / short_span).clamp(0.0, 1.0)
            }
            Some(RoofShape::Dome | RoofShape::Onion) => {
                let [lo, hi] = self.bounds;
                let q = ((p - (lo + hi) * 0.5) / ((hi - lo) * 0.5))
                    .length()
                    .min(1.0);
                (1.0 - q * q).max(0.0).sqrt() * rise
            }
            _ => rise,
        };
        self.eave + height.clamp(0.0, rise)
    }
}

fn edge_facade(rings: &Rings, point: DVec2) -> FacadeStamp {
    let mut ordinal = 0u16;
    let mut best = (
        f64::INFINITY,
        FacadeStamp {
            boundary: true,
            corner: false,
            edge: 0,
            along_m: 0.0,
            length_m: 1.0,
            edge_distance_m: 0.0,
        },
    );
    for ring in rings {
        for i in 0..ring.len() {
            let a = DVec2::from_array(ring[i].map(f64::from));
            let b = DVec2::from_array(ring[(i + 1) % ring.len()].map(f64::from));
            let length = a.distance(b);
            if length > 0.001 {
                let t = ((point - a).dot(b - a) / (length * length)).clamp(0.0, 1.0);
                let distance = point.distance_squared(a.lerp(b, t));
                if distance < best.0 {
                    best = (
                        distance,
                        FacadeStamp {
                            boundary: true,
                            corner: false,
                            edge: ordinal,
                            along_m: (t * length) as f32,
                            length_m: length as f32,
                            edge_distance_m: distance.sqrt() as f32,
                        },
                    );
                }
            }
            ordinal = ordinal.wrapping_add(1);
        }
    }
    best.1
}

const MAX_FACADE_DETAIL_VERTICES: usize = 90_000;

#[derive(Clone)]
struct OcclusionPlane {
    anchor: DVec3,
    normal: DVec3,
}

#[derive(Clone, Default)]
struct OpaqueCore {
    // Their intersection is a conservative convex interior of the rendered
    // envelope, even when the source footprint is concave. Rounded vertices
    // and all arch points matter: source-space containment alone does not
    // account for the straight chords used by the renderer near the poles.
    planes: Vec<OcclusionPlane>,
    floor_y: Option<f64>,
    roof_y: Option<f64>,
    invalid: bool,
}

impl OpaqueCore {
    fn wall(&mut self, points: [DVec3; 4], outward: DVec3, foundation: bool) {
        let points = points.map(|point| point.as_vec3().as_dvec3());
        if foundation {
            let high = points[0].y.max(points[1].y);
            self.floor_y = Some(self.floor_y.map_or(high, |value| value.max(high)));
        }
        for triangle in [
            [points[0], points[1], points[2]],
            [points[0], points[2], points[3]],
        ] {
            let normal = (triangle[1] - triangle[0]).cross(triangle[2] - triangle[0]);
            let facing = normal.dot(outward);
            if !normal.is_finite() || normal.length_squared() < 1.0e-16 || facing.abs() < 1.0e-14 {
                self.invalid = true;
                continue;
            }
            self.planes.push(OcclusionPlane {
                anchor: triangle[0],
                normal: normal * facing.signum(),
            });
        }
    }

    fn contains(&self, point: DVec3) -> bool {
        let point = point.as_vec3().as_dvec3();
        !self.invalid
            && self.floor_y.is_some_and(|floor| point.y >= floor)
            && self.roof_y.is_some_and(|roof| point.y <= roof)
            && self.planes.iter().all(|plane| {
                let products = plane.normal * (point - plane.anchor);
                let error = products.abs().element_sum() * f64::EPSILON * 32.0;
                products.element_sum() <= -error
            })
    }
}

struct OpaqueBody {
    query: PolygonQuery,
    core: OpaqueCore,
    foundation: std::cell::RefCell<Option<MeshBuffers>>,
    used: std::cell::Cell<bool>,
    base: f64,
    eave: f64,
}

#[derive(Clone)]
struct PartyWall {
    edge: [DVec2; 2],
    body: Rc<OpaqueBody>,
}

type WallEdgeKey = [[u64; 2]; 2];

fn single_boundary_cycle(boundary: &[[DVec2; 2]]) -> bool {
    let mut next = HashMap::new();
    for &edge in boundary {
        let key = wall_edge_key(edge);
        if next.insert(key[0], key[1]).is_some() {
            return false;
        }
    }
    let Some(first) = boundary.first().map(|&edge| wall_edge_key(edge)[0]) else {
        return false;
    };
    let mut point = first;
    for visited in 0..boundary.len() {
        let Some(&end) = next.get(&point) else {
            return false;
        };
        point = end;
        if point == first {
            return visited + 1 == boundary.len();
        }
    }
    false
}

fn wall_edge_key(edge: [DVec2; 2]) -> WallEdgeKey {
    edge.map(|point| [point.x.to_bits(), point.y.to_bits()])
}

#[derive(Default)]
struct OpaqueWallIndex {
    bodies: Vec<Rc<OpaqueBody>>,
    edges: HashMap<WallEdgeKey, Vec<Rc<OpaqueBody>>>,
}

impl OpaqueWallIndex {
    fn insert(
        &mut self,
        boundary: &[[DVec2; 2]],
        rings: &Rings,
        base: f64,
        eave: f64,
        core: OpaqueCore,
        foundation: MeshBuffers,
    ) {
        let body = Rc::new(OpaqueBody {
            query: PolygonQuery::new(rings),
            core,
            foundation: std::cell::RefCell::new(Some(foundation)),
            used: std::cell::Cell::new(false),
            base,
            eave,
        });
        self.bodies.push(body.clone());
        for &edge in boundary {
            self.edges
                .entry(wall_edge_key(edge))
                .or_default()
                .push(body.clone());
        }
    }

    fn finish(&self, mesh: &mut MeshBuffers) {
        // A foundation can be exposed by terrain or water carving. Close only
        // owners actually used for pruning, with their existing exact cap.
        for body in &self.bodies {
            if body.used.get()
                && let Some(foundation) = body.foundation.borrow_mut().take()
            {
                append_mesh(mesh, foundation);
            }
        }
    }

    fn facing(&self, boundary: &[[DVec2; 2]]) -> Vec<PartyWall> {
        boundary
            .iter()
            .flat_map(|&edge| {
                self.edges
                    .get(&wall_edge_key([edge[1], edge[0]]))
                    .into_iter()
                    .flatten()
                    .map(move |body| PartyWall {
                        edge,
                        body: body.clone(),
                    })
            })
            .collect()
    }
}

fn foundation_from_collision(
    collision: &MeshBuffers,
    appearance: SurfaceAppearance,
) -> MeshBuffers {
    let mut result = MeshBuffers::default();
    for indices in collision.indices.chunks_exact(3) {
        // Collision cap indices reverse the upward triangulation; the source
        // payload retains its upward face id. Walls never satisfy both tests.
        if collision.vertices[indices[0] as usize].face_ao & 7 != 2 {
            continue;
        }
        let points = std::array::from_fn::<_, 3, _>(|i| {
            DVec3::from_array(collision.vertices[indices[i] as usize].pos.map(f64::from))
        });
        if (points[1] - points[0]).cross(points[2] - points[0]).y < 0.0 {
            emit_triangle(&mut result, points, appearance, false);
        }
    }
    result
}

fn mandatory_envelope_inside_cell(mesh: &MeshBuffers, eligible: &[u32; N]) -> bool {
    if mesh.vertices.is_empty() {
        return false;
    }
    let mut lo = DVec2::splat(f64::INFINITY);
    let mut hi = DVec2::splat(f64::NEG_INFINITY);
    for vertex in &mesh.vertices {
        let point = DVec2::new(f64::from(vertex.pos[0]), f64::from(vertex.pos[2]));
        // Cell clipping writes exact 0/N coordinates. Those open cuts cannot
        // establish an opaque volume independently of another cell product.
        if !point.is_finite()
            || point.cmple(DVec2::splat(1.0e-4)).any()
            || point.cmpge(DVec2::splat(N as f64 - 1.0e-4)).any()
        {
            return false;
        }
        lo = lo.min(point);
        hi = hi.max(point);
    }
    (lo.y.floor() as usize..=hi.y.floor() as usize).all(|z| {
        (lo.x.floor() as usize..=hi.x.floor() as usize).all(|x| eligible[z] & (1 << x) != 0)
    })
}

fn segments_proven_disjoint(a: [DVec2; 2], b: [DVec2; 2]) -> bool {
    if disjoint_bounds(
        (a[0].min(a[1]), a[0].max(a[1])),
        (b[0].min(b[1]), b[0].max(b[1])),
    ) {
        return true;
    }
    let separated = |line: [DVec2; 2], points: [DVec2; 2]| {
        let direction = line[1] - line[0];
        let side = points.map(|point| {
            let delta = point - line[0];
            let products = DVec2::new(direction.x * delta.y, direction.y * delta.x);
            (
                products.x - products.y,
                products.abs().element_sum().max(1.0) * f64::EPSILON * 64.0,
            )
        });
        side.iter().all(|&(value, error)| value > error)
            || side.iter().all(|&(value, error)| value < -error)
    };
    separated(a, b) || separated(b, a)
}

impl PartyWall {
    fn covers_heights(&self, low: f64, high: f64) -> bool {
        self.body.base <= low && self.body.eave >= high
    }

    #[cfg(test)]
    fn covers_glazing(
        &self,
        plane: &FacadePlane<'_, '_>,
        x: [f64; 2],
        y: [f64; 2],
        depth: f64,
        shape: OpeningShape,
    ) -> bool {
        self.covers_opening(plane, x, y, depth, &opening_outline(x, y, shape))
    }

    fn covers_opening(
        &self,
        plane: &FacadePlane<'_, '_>,
        x: [f64; 2],
        y: [f64; 2],
        depth: f64,
        outline: &OpeningOutline,
    ) -> bool {
        if !self.covers_heights(y[0], y[1]) {
            return false;
        }
        let points = x.map(|x| plane.origin + plane.direction * x + plane.outward * depth);
        points.iter().all(|&point| self.body.query.contains(point))
            && self
                .body
                .query
                .edges
                .iter()
                .all(|&edge| segments_proven_disjoint(points, edge))
            && outline
                .iter()
                .all(|p| self.body.core.contains(plane.point(p.x, p.y, depth)))
            && {
                self.body.used.set(true);
                true
            }
    }
}

struct FacadeBlocker {
    #[cfg(test)]
    rings: Rings,
    bounds: Option<(DVec2, DVec2)>,
    query: PolygonQuery,
    bottom: f64,
    top: f64,
}

struct ShadowWall {
    backing: std::ops::Range<usize>,
    units: Vec<std::ops::Range<usize>>,
}

struct ShadowBackingTriangle {
    origin: DVec3,
    points: [DVec3; 3],
    inward: [DVec3; 3],
    normal: DVec3,
    scale: f64,
}

fn metric_delta(frame: &ChunkFrame, delta: DVec3) -> DVec3 {
    frame.x_axis * (delta.x * frame.grid_scale.x)
        + frame.y_axis * (delta.y * frame.grid_scale.y)
        + frame.z_axis * (delta.z * frame.grid_scale.z)
}

impl ShadowBackingTriangle {
    fn new(frame: &ChunkFrame, points: [DVec3; 3]) -> Option<Self> {
        let origin = points[0];
        let points = points.map(|point| metric_delta(frame, point - origin));
        let normal = points[1].cross(points[2]).try_normalize()?;
        let mut inward = [DVec3::ZERO; 3];
        for edge in 0..3 {
            inward[edge] = normal
                .cross(points[(edge + 1) % 3] - points[edge])
                .try_normalize()?;
        }
        let scale = points
            .iter()
            .map(|point| point.length())
            .fold(1.0, f64::max);
        Some(Self {
            origin,
            points,
            inward,
            normal,
            scale,
        })
    }

    fn deviation(&self, frame: &ChunkFrame, point: DVec3) -> Option<f64> {
        let point = metric_delta(frame, point - self.origin);
        let margin = self.scale.max(point.length()) * f64::EPSILON * 256.0;
        let distance = self.normal.dot(point);
        let projected = point - self.normal * distance;
        // A strict inset rejects uncertain diagonal, column and cell borders.
        // This proof uses the final clipped, rounded vertices, so removed wall
        // pieces cannot support an offset pane across an ownership boundary.
        if !distance.is_finite()
            || distance.abs() + margin > 0.05
            || (0..3).any(|edge| self.inward[edge].dot(projected - self.points[edge]) <= margin)
        {
            return None;
        }
        Some(distance.abs() + margin)
    }
}

fn prove_shadow_omissions(
    mesh: &MeshBuffers,
    frame: &ChunkFrame,
    walls: &[ShadowWall],
) -> Vec<crate::ShadowOmission> {
    let point = |index: u32| DVec3::from_array(mesh.vertices[index as usize].pos.map(f64::from));
    let mut result: Vec<crate::ShadowOmission> = Vec::new();
    for wall in walls {
        let backing: Vec<_> = mesh.indices[wall.backing.clone()]
            .chunks_exact(3)
            .filter_map(|triangle| {
                ShadowBackingTriangle::new(
                    frame,
                    [point(triangle[0]), point(triangle[1]), point(triangle[2])],
                )
            })
            .collect();
        for unit in &wall.units {
            // Every emitted triangle is inside this one convex backing prism.
            // Testing all its vertices bounds the entire unit's displacement,
            // including arched outlines and flat surrounds behind the glass.
            let bound = backing.iter().find_map(|triangle| {
                mesh.indices[unit.clone()]
                    .iter()
                    .try_fold(0.0_f64, |bound, &index| {
                        triangle
                            .deviation(frame, point(index))
                            .map(|distance| bound.max(distance))
                    })
            });
            let Some(bound) = bound else {
                continue;
            };
            let (Ok(first_index), Ok(index_count)) =
                (u32::try_from(unit.start), u32::try_from(unit.len()))
            else {
                continue;
            };
            if index_count == 0 {
                continue;
            }
            let max_deviation_m = (bound as f32).next_up();
            if let Some(previous) = result.last_mut()
                && previous.first_index.checked_add(previous.index_count) == Some(first_index)
                && let Some(count) = previous.index_count.checked_add(index_count)
            {
                previous.index_count = count;
                previous.max_deviation_m = previous.max_deviation_m.max(max_deviation_m);
            } else {
                result.push(crate::ShadowOmission {
                    first_index,
                    index_count,
                    max_deviation_m,
                });
            }
        }
    }
    result
}

struct FacadeContext {
    #[cfg(test)]
    owner: Rings,
    owner_bounds: Option<(DVec2, DVec2)>,
    owner_query: PolygonQuery,
    blockers: Vec<FacadeBlocker>,
    entrance: Option<(u16, f64)>,
    party_walls: Vec<PartyWall>,
    opaque_core: Option<std::cell::RefCell<OpaqueCore>>,
    shadow_walls: std::cell::RefCell<Vec<ShadowWall>>,
}

impl FacadeContext {
    fn from_sources(
        owner: &Rings,
        buildings: &[Building],
        neighbors: &[usize],
        ground: f64,
    ) -> Self {
        // Decoration uses neighbouring source spans on this owner's stable
        // ground reference. Whether a neighbour is rendered in this cell, or
        // has a sampled roof datum here, must not change an entrance or trim.
        // Nearby construction conservatively shares the local ground plane;
        // no extra elevation acquisition is required for optional projections.
        let blockers = neighbors
            .iter()
            .filter_map(|&index| {
                let building = &buildings[index];
                (building.min_height_m.is_finite()
                    && building.height_m.is_finite()
                    && building.height_m > building.min_height_m)
                    .then_some(FacadeBlocker {
                        #[cfg(test)]
                        rings: building.rings.clone(),
                        bounds: bounds(&building.rings),
                        query: PolygonQuery::new(&building.rings),
                        bottom: ground + f64::from(building.min_height_m.max(0.0)),
                        top: ground + f64::from(building.height_m),
                    })
            })
            .collect();
        Self {
            #[cfg(test)]
            owner: owner.clone(),
            owner_bounds: bounds(owner),
            owner_query: PolygonQuery::new(owner),
            blockers,
            entrance: None,
            party_walls: Vec::new(),
            opaque_core: None,
            shadow_walls: std::cell::RefCell::default(),
        }
    }

    fn hidden_glazing(
        &self,
        plane: &FacadePlane<'_, '_>,
        x: [f64; 2],
        y: [f64; 2],
        depth: f64,
        shape: OpeningShape,
        relief: bool,
    ) -> bool {
        if self.party_walls.is_empty() {
            return false;
        }
        let (outline, _) = plane.opening_outlines(x, y, shape, 0.0, relief);
        self.party_walls
            .iter()
            .any(|wall| wall.covers_opening(plane, x, y, depth, &outline))
    }

    fn clear(&self, points: [DVec2; 3], low: f64, high: f64) -> bool {
        if points.iter().any(|&point| self.owner_query.contains(point)) {
            return false;
        }
        !self.blockers.iter().any(|blocker| {
            blocker.bottom < high
                && blocker.top > low
                && points.iter().any(|&point| blocker.query.contains(point))
        })
    }

    fn clear_rectangle(&self, corners: [DVec2; 4], low: f64, high: f64) -> bool {
        self.clear_rectangle_indexed::<true>(corners, low, high)
    }

    fn clear_rectangle_indexed<const BINNED: bool>(
        &self,
        corners: [DVec2; 4],
        low: f64,
        high: f64,
    ) -> bool {
        let query = clearance_bounds(corners);
        let overlaps = |polygon: &PolygonQuery, bounds: Option<(DVec2, DVec2)>| {
            if bounds.is_some_and(|bounds| disjoint_bounds(query, bounds)) {
                return false;
            }
            if corners.iter().any(|&p| polygon.contains(p)) {
                return true;
            }
            let inside = |p: DVec2| {
                let sides: [f64; 4] = std::array::from_fn(|i| {
                    (corners[(i + 1) % 4] - corners[i]).perp_dot(p - corners[i])
                });
                sides.iter().all(|&s| s >= -1.0e-8) || sides.iter().all(|&s| s <= 1.0e-8)
            };
            let touches = |[a, b]: [DVec2; 2]| {
                if disjoint_bounds(query, (a.min(b), a.max(b))) {
                    return false;
                }
                inside(a)
                    || (0..4)
                        .any(|j| intersection([a, b], [corners[j], corners[(j + 1) % 4]]).is_some())
            };
            if BINNED {
                polygon.any_edge_in_bounds(query, touches)
            } else {
                polygon.edges.iter().copied().any(touches)
            }
        };
        !overlaps(&self.owner_query, self.owner_bounds)
            && !self.blockers.iter().any(|blocker| {
                blocker.bottom < high
                    && blocker.top > low
                    && overlaps(&blocker.query, blocker.bounds)
            })
    }
}

fn disjoint_bounds(a: (DVec2, DVec2), b: (DVec2, DVec2)) -> bool {
    a.0.cmpgt(b.1).any() || b.0.cmpgt(a.1).any()
}

fn clearance_bounds(corners: [DVec2; 4]) -> (DVec2, DVec2) {
    let lo = corners.into_iter().reduce(DVec2::min).unwrap();
    let hi = corners.into_iter().reduce(DVec2::max).unwrap();
    let edges = std::array::from_fn::<_, 4, _>(|i| corners[(i + 1) % 4] - corners[i]);
    let shortest = edges
        .into_iter()
        .map(DVec2::length)
        .fold(f64::INFINITY, f64::min);
    // The exact inside predicate permits 1e-8 in cross-product units. Keep
    // that tolerance and coordinate rounding outside the broad-phase bounds;
    // degenerate rectangles use the original unrestricted predicate.
    if shortest <= 1.0e-12
        || edges[0].perp_dot(edges[1]).abs() < edges[0].length() * edges[1].length() * 0.5
    {
        return (DVec2::splat(f64::NEG_INFINITY), DVec2::splat(f64::INFINITY));
    }
    let margin = 4.0e-8 / shortest + lo.abs().max(hi.abs()).max_element() * f64::EPSILON * 64.0;
    (
        (lo - DVec2::splat(margin)).map(f64::next_down),
        (hi + DVec2::splat(margin)).map(f64::next_up),
    )
}

// Select a supported source edge, never a courtyard or Boolean cut. Integer
// centimetres avoid choosing another entrance after harmless reprojection ulps.
fn entrance_on_source_boundary(
    rings: &Rings,
    boundary: &[[DVec2; 2]],
    floor: f64,
    context: &FacadeContext,
    front_axis: Option<DVec2>,
) -> Option<(u16, f64)> {
    let outer = rings
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| {
            let area = |ring: &Vec<[f32; 2]>| {
                let origin = DVec2::from_array(ring[0].map(f64::from));
                (0..ring.len())
                    .map(|i| {
                        let a = DVec2::from_array(ring[i].map(f64::from)) - origin;
                        let b =
                            DVec2::from_array(ring[(i + 1) % ring.len()].map(f64::from)) - origin;
                        a.perp_dot(b)
                    })
                    .sum::<f64>()
                    .abs()
            };
            area(a).total_cmp(&area(b))
        })?
        .0;
    let first: usize = rings[..outer].iter().map(Vec::len).sum();
    let last = first + rings[outer].len();
    boundary
        .iter()
        .filter_map(|edge| {
            let length = edge[0].distance(edge[1]);
            if length < 2.4 {
                return None;
            }
            let middle = (edge[0] + edge[1]) * 0.5;
            let source = edge_facade(rings, middle);
            if source.edge_distance_m > 0.01 || !(first..last).contains(&usize::from(source.edge)) {
                return None;
            }
            let tangent = (edge[1] - edge[0]) / length;
            let outward = DVec2::new(tangent.y, -tangent.x);
            if ![0.025, 0.25].into_iter().all(|depth| {
                let front = middle + outward * depth;
                context.clear(
                    [front - tangent * 0.65, front, front + tangent * 0.65],
                    floor,
                    floor + 2.2,
                )
            }) {
                return None;
            }
            Some((
                front_axis.map_or(0, |axis| (outward.dot(axis).abs() * 100.0).round() as u32),
                (length * 100.0).round() as u64,
                std::cmp::Reverse(source.edge),
                source.along_m,
            ))
        })
        .max_by_key(|&(alignment, length, edge, _)| (alignment, length, edge))
        .map(|(_, _, edge, along)| (edge.0, f64::from(along)))
}

#[derive(Clone, Copy)]
struct FacadeResolution {
    feature_m: f64,
}

impl FacadeResolution {
    #[cfg(test)]
    const FULL: Self = Self { feature_m: 0.0 };

    fn for_frame(frame: &ChunkFrame) -> Self {
        // A settled terrain leaf targets three pixels per sample. Individual
        // architectural relief therefore needs one third of its sample pitch.
        // Saturated cuts and loading ancestors can be magnified: this is a
        // stable detail policy, not a guarantee about every camera frame.
        Self {
            feature_m: frame
                .grid_scale
                .x
                .min(frame.grid_scale.z)
                .min(frame.addr.cell.sample_pitch_m())
                / 3.0,
        }
    }

    fn resolves(self, feature_m: f64) -> bool {
        self.feature_m == 0.0 || feature_m > self.feature_m
    }

    fn resolves_glazing(self, width_m: f64, height_m: f64) -> bool {
        // Thin architectural relief uses the one-pixel feature threshold.
        // A separate bright pane needs two pixels in both dimensions for
        // stable coverage during motion. This applies through street levels;
        // their finer source pitch naturally retains resolvable panes.
        self.feature_m == 0.0 || width_m.min(height_m) >= self.feature_m * 2.0
    }
}

fn detail_screen_threshold(radius_m: f32, resolution: FacadeResolution) -> f32 {
    // A finite physical floor also keeps full-resolution fixtures from
    // requesting an infinite projected diameter.
    let feature_m = resolution.feature_m.max(0.12) as f32;
    (2.0 * radius_m * 2.0 / feature_m).max(72.0)
}

fn coarse_building_pixel_m(frame: &ChunkFrame) -> Option<f64> {
    // Three pixels per terrain sample is the settled selector target. Keep
    // the existing architecture from L16 onward, where individual facades
    // and roof profiles become resolvable.
    (frame.addr.cell.level <= 15).then(|| frame.addr.cell.sample_pitch_m() / 3.0)
}

struct FacadePlane<'a, 'f> {
    shadow_units: Option<&'a std::cell::RefCell<Vec<std::ops::Range<usize>>>>,
    projection: &'a Projection<'f>,
    origin: DVec2,
    direction: DVec2,
    outward: DVec2,
}

impl FacadePlane<'_, '_> {
    fn point(&self, x: f64, y: f64, depth: f64) -> DVec3 {
        self.projection
            .point(self.origin + self.direction * x + self.outward * depth, y)
    }

    fn quad(
        &self,
        mesh: &mut MeshBuffers,
        x: [f64; 2],
        y: [f64; 2],
        depth: f64,
        appearance: SurfaceAppearance,
    ) {
        emit_quad(
            mesh,
            [
                self.point(x[0], y[0], depth),
                self.point(x[1], y[0], depth),
                self.point(x[1], y[1], depth),
                self.point(x[0], y[1], depth),
            ],
            appearance,
            false,
        );
    }

    fn polygon(
        &self,
        mesh: &mut MeshBuffers,
        points: &OpeningOutline,
        depth: f64,
        appearance: SurfaceAppearance,
    ) {
        let projected = self.project_outline(points, depth);
        for i in 1..points.len().saturating_sub(1) {
            emit_triangle(
                mesh,
                [projected[0], projected[i], projected[i + 1]],
                appearance,
                false,
            );
        }
    }

    fn project_outline(&self, points: &OpeningOutline, depth: f64) -> [DVec3; 9] {
        let mut projected = [DVec3::ZERO; 9];
        for (target, point) in projected.iter_mut().zip(points.iter()) {
            *target = self.point(point.x, point.y, depth);
        }
        projected
    }

    fn aperture(
        &self,
        mesh: &mut MeshBuffers,
        x: [f64; 2],
        y: [f64; 2],
        depth: f64,
        shape: OpeningShape,
        appearance: SurfaceAppearance,
        full_outline: bool,
    ) {
        let first_index = mesh.indices.len();
        let (outline, _) = self.opening_outlines(x, y, shape, 0.0, full_outline);
        self.polygon(mesh, &outline, depth, appearance);
        self.record_shadow_unit(
            mesh,
            first_index,
            depth.abs() <= 0.05
                && matches!(
                    appearance.material,
                    crate::material::WINDOW | crate::material::LIT_WINDOW
                ),
        );
    }

    #[cfg(test)]
    fn opening(
        &self,
        mesh: &mut MeshBuffers,
        x: [f64; 2],
        y: [f64; 2],
        depth: f64,
        shape: OpeningShape,
        frame: f64,
        glass: SurfaceAppearance,
        trim: SurfaceAppearance,
        relief: bool,
    ) {
        self.opening_with_surround(mesh, x, y, depth, shape, frame, glass, trim, relief, true);
    }

    #[cfg(test)]
    fn opening_with_surround(
        &self,
        mesh: &mut MeshBuffers,
        x: [f64; 2],
        y: [f64; 2],
        depth: f64,
        shape: OpeningShape,
        frame: f64,
        glass: SurfaceAppearance,
        trim: SurfaceAppearance,
        relief: bool,
        surround: bool,
    ) {
        let first_index = mesh.indices.len();
        let flat_glass = !relief
            && depth.abs() <= 0.05
            && matches!(
                glass.material,
                crate::material::WINDOW | crate::material::LIT_WINDOW
            );
        let (inner, outer) = self.opening_outlines(x, y, shape, frame, relief);
        self.polygon(mesh, &inner, depth, glass);
        let Some(outer) = outer.filter(|_| surround) else {
            self.record_shadow_unit(mesh, first_index, flat_glass);
            return;
        };
        if !relief {
            self.polygon(mesh, &outer, depth - 0.008, trim);
            self.record_shadow_unit(mesh, first_index, flat_glass);
            return;
        }
        let inner_projected = self.project_outline(&inner, depth + 0.05);
        let outer_projected = self.project_outline(&outer, depth + 0.05);
        for i in 0..inner.len() {
            let next = (i + 1) % inner.len();
            emit_quad(
                mesh,
                [
                    outer_projected[i],
                    outer_projected[next],
                    inner_projected[next],
                    inner_projected[i],
                ],
                trim,
                false,
            );
        }
    }

    fn record_shadow_unit(&self, mesh: &MeshBuffers, first_index: usize, flat_glass: bool) {
        if flat_glass
            && first_index < mesh.indices.len()
            && let Some(units) = self.shadow_units
        {
            units.borrow_mut().push(first_index..mesh.indices.len());
        }
    }

    fn opening_outlines(
        &self,
        x: [f64; 2],
        y: [f64; 2],
        shape: OpeningShape,
        frame: f64,
        relief: bool,
    ) -> (OpeningOutline, Option<OpeningOutline>) {
        let inner = opening_outline(x, y, shape);
        let outer = (frame > 0.0).then(|| {
            opening_outline(
                [x[0] - frame, x[1] + frame],
                [y[0] - frame, y[1] + frame],
                shape,
            )
        });
        if !relief {
            let grid = self.projection.frame.grid_scale;
            let error = (grid.x.min(grid.z) / 40.0).min(0.12);
            if let Some(reduced) = minified_opening(&inner, error) {
                if let Some(outer) = &outer {
                    if let Some(reduced_outer) = minified_opening(outer, error)
                        && reduced
                            .iter()
                            .all(|&p| convex_outline_contains(&reduced_outer, p))
                    {
                        return (reduced, Some(reduced_outer));
                    }
                } else {
                    return (reduced, None);
                }
            }
        }
        (inner, outer)
    }

    // A supported shallow extrusion: front, top, bottom and two returns.
    // The source wall behind it remains the closed structural envelope.
    fn strip(
        &self,
        mesh: &mut MeshBuffers,
        x: [f64; 2],
        y: [f64; 2],
        depth: [f64; 2],
        appearance: SurfaceAppearance,
    ) {
        if mesh.vertices.len() + 96 > MAX_FACADE_DETAIL_VERTICES {
            return;
        }
        self.structural_strip(mesh, x, y, depth, appearance);
    }

    fn structural_strip(
        &self,
        mesh: &mut MeshBuffers,
        x: [f64; 2],
        y: [f64; 2],
        depth: [f64; 2],
        appearance: SurfaceAppearance,
    ) {
        let corners = [(x[0], y[0]), (x[1], y[0]), (x[1], y[1]), (x[0], y[1])];
        let front = corners.map(|(x, y)| self.point(x, y, depth[1]));
        let back = corners.map(|(x, y)| self.point(x, y, depth[0]));
        emit_quad(mesh, front, appearance, false);
        let side = SurfaceAppearance {
            tint: appearance.tint.map(|v| (f32::from(v) * 0.82) as u8),
            ..appearance
        };
        for edge in 0..4 {
            let next = (edge + 1) % 4;
            emit_quad(
                mesh,
                [back[edge], back[next], front[next], front[edge]],
                side,
                false,
            );
        }
    }

    fn resolved_strip(
        &self,
        mesh: &mut MeshBuffers,
        x: [f64; 2],
        y: [f64; 2],
        depth: [f64; 2],
        appearance: SurfaceAppearance,
        resolution: FacadeResolution,
    ) {
        if resolution.resolves((depth[1] - depth[0]).abs()) {
            self.structural_strip(mesh, x, y, depth, appearance);
        } else {
            self.quad(mesh, x, y, depth[1], appearance);
        }
    }

    fn clear(&self, context: &FacadeContext, x: [f64; 2], y: [f64; 2], depth: f64) -> bool {
        // The entire footprint must be free: endpoint samples can miss a thin
        // neighbouring part between an oriel's attachment and its outer face.
        let point = |x, d| self.origin + self.direction * x + self.outward * d;
        context.clear_rectangle(
            [
                point(x[0], 0.025),
                point(x[1], 0.025),
                point(x[1], depth.max(0.026)),
                point(x[0], depth.max(0.026)),
            ],
            y[0],
            y[1],
        )
    }
}

struct OpeningOutline {
    points: [DVec2; 9],
    len: usize,
}

impl std::ops::Deref for OpeningOutline {
    type Target = [DVec2];

    fn deref(&self) -> &Self::Target {
        &self.points[..self.len]
    }
}

fn opening_outline(x: [f64; 2], y: [f64; 2], shape: OpeningShape) -> OpeningOutline {
    let mut points = [DVec2::ZERO; 9];
    points[0] = DVec2::new(x[0], y[0]);
    points[1] = DVec2::new(x[1], y[0]);
    if shape == OpeningShape::Rectangular {
        points[2] = DVec2::new(x[1], y[1]);
        points[3] = DVec2::new(x[0], y[1]);
        return OpeningOutline { points, len: 4 };
    }
    let half = (x[1] - x[0]) * 0.5;
    let center = (x[0] + x[1]) * 0.5;
    let rise = match shape {
        OpeningShape::SegmentalArch => half * 0.38,
        OpeningShape::RoundArch => half,
        OpeningShape::PointedArch => half * 1.25,
        OpeningShape::Rectangular => unreachable!(),
    }
    .min((y[1] - y[0]) * 0.50);
    let spring = y[1] - rise;
    // Six convex crown facets retain the opening's silhouette without tiny
    // tessellation at world scale. Both sides share exact springing points.
    for i in 0..=6 {
        let t = i as f64 / 6.0;
        let q = 1.0 - (2.0 * t - 1.0).abs();
        let lift = if shape == OpeningShape::PointedArch {
            q.powf(0.78)
        } else {
            (std::f64::consts::PI * t).sin()
        };
        points[i + 2] = DVec2::new(
            center + half * (std::f64::consts::PI * t).cos(),
            spring + rise * lift,
        );
    }
    OpeningOutline { points, len: 9 }
}

fn outline_area(points: &OpeningOutline) -> f64 {
    (1..points.len().saturating_sub(1))
        .map(|i| (points[i] - points[0]).perp_dot(points[i + 1] - points[0]))
        .sum::<f64>()
        * 0.5
}

fn convex_outline_contains(points: &OpeningOutline, p: DVec2) -> bool {
    (0..points.len())
        .all(|i| (points[(i + 1) % points.len()] - points[i]).perp_dot(p - points[i]) >= 0.0)
}

fn outline_distance_bound(a: &OpeningOutline, b: &OpeningOutline) -> f64 {
    // For convex polygons, the Hausdorff distance of the filled silhouettes is
    // attained at a vertex. Distance to the other's boundary is conservative
    // for vertices lying inside, and avoids a tolerance-sensitive inside test.
    let directed = |from: &OpeningOutline, to: &OpeningOutline| {
        from.iter()
            .map(|&p| {
                (0..to.len())
                    .map(|i| {
                        let start = to[i];
                        let delta = to[(i + 1) % to.len()] - start;
                        let t = ((p - start).dot(delta) / delta.length_squared()).clamp(0.0, 1.0);
                        p.distance_squared(start + delta * t)
                    })
                    .fold(f64::INFINITY, f64::min)
            })
            .fold(0.0_f64, f64::max)
    };
    directed(a, b).max(directed(b, a)).sqrt()
}

fn minified_opening(original: &OpeningOutline, error: f64) -> Option<OpeningOutline> {
    if original.len() != 9 || error <= 0.0 {
        return None;
    }
    let width = original[1].x - original[0].x;
    let height = original[5].y - original[0].y;
    let area = outline_area(original);
    if !area.is_finite() || width <= 0.0 || height <= 0.0 {
        return None;
    }
    // A triangular crown has half the area of its bounding rectangle. Raising
    // its spring preserves the source opening's integrated glass/night area,
    // apex, sill and both jamb bounds while removing four unresolvable facets.
    let rise = 2.0 * (height - area / width);
    let spring = original[5].y - rise;
    if rise <= 0.0 || spring <= original[0].y {
        return None;
    }
    let mut points = [DVec2::ZERO; 9];
    points[..5].copy_from_slice(&[
        original[0],
        original[1],
        DVec2::new(original[1].x, spring),
        original[5],
        DVec2::new(original[0].x, spring),
    ]);
    let reduced = OpeningOutline { points, len: 5 };
    if (outline_area(&reduced) - area).abs() > area.max(1.0) * 1.0e-10
        || outline_distance_bound(original, &reduced) > error
    {
        return None;
    }
    Some(reduced)
}

fn configure_roof_style(roof: &mut RoofField, source: &Building, stamp: &BuildingStamp) {
    roof.tiered = !stamp.lidar
        && source.roof_shape.is_none()
        && expressed_style(stamp).is_some_and(|style| style.roofline == Roofline::TieredHip);
    // A cone is the selected physical roof, not facade ornament. Keep the
    // source and domain constraints independent of the expression fade.
    roof.conical = !stamp.lidar
        && source.roof_shape.is_none()
        && source.kind != BuildingKind::RoofOnly
        && !roof.reentrant
        && stamp.roof_shape == Some(RoofShape::Pyramidal)
        && stamp.design.style.profile().roofline == Roofline::Conical;
}

fn expressed_style(stamp: &BuildingStamp) -> Option<StyleProfile> {
    (stamp.design.expression >= 96).then(|| stamp.design.style.profile())
}

fn emit_style_bands(
    plane: &FacadePlane<'_, '_>,
    mesh: &mut MeshBuffers,
    context: &FacadeContext,
    length: f64,
    base: f64,
    floor: f64,
    eave: f64,
    floors: u32,
    floor_height: f64,
    style: StyleProfile,
    stamp: &BuildingStamp,
    trim: SurfaceAppearance,
    resolution: FacadeResolution,
) {
    let x = [0.025, length - 0.025];
    if x[1] <= x[0] {
        return;
    }
    let band = f64::from(style.band_width_m).clamp(0.08, 0.32);
    let reach = 0.025;
    let base_top = (floor + f64::from(style.base_height_m)).min(eave - 0.4);
    let base_material = if style.banding == Banding::TimberFrame {
        crate::material::STONE_WALL
    } else {
        stamp.design.wall_material
    };
    let foundation = SurfaceAppearance {
        material: base_material,
        tint: stamp
            .design
            .wall
            .map(|v| (f32::from(v) * 0.77).round() as u8),
    };
    if base_top > base && plane.clear(context, x, [base, base_top], 0.07) {
        plane.quad(mesh, x, [base, base_top], 0.014, foundation);
    }
    if style.banding != Banding::None && resolution.resolves(band) {
        for level in 0..=floors {
            let selected = level == floors
                || (level > 0
                    && match style.banding {
                        Banding::EveryFloor | Banding::TimberFrame => true,
                        Banding::Alternating => level == 1 || level + 1 == floors || level % 3 == 0,
                        _ => false,
                    });
            if !selected {
                continue;
            }
            let top = floor + f64::from(level) * floor_height;
            let y = [top - band, top - 0.012];
            if y[0] >= base && plane.clear(context, x, y, reach) {
                plane.quad(mesh, x, y, 0.02, trim);
            }
        }
    }
    if (style.pilasters || style.banding == Banding::TimberFrame) && resolution.resolves(band) {
        // Strong corner piers frame the whole source elevation. Repetition is
        // fitted in source coordinates below, never restarted at a cell cut.
        for x in [
            [0.035, 0.035 + band],
            [length - 0.035 - band, length - 0.035],
        ] {
            if x[1] < length && plane.clear(context, x, [floor, eave], reach) {
                plane.quad(mesh, x, [floor, eave - 0.02], 0.02, trim);
            }
        }
    }
}

fn emit_style_bay(
    plane: &FacadePlane<'_, '_>,
    mesh: &mut MeshBuffers,
    context: &FacadeContext,
    mut x: [f64; 2],
    mut y: [f64; 2],
    floor: f64,
    floor_height: f64,
    length: f64,
    bay_width: f32,
    storey: u32,
    bay: u32,
    front: bool,
    style: StyleProfile,
    stamp: &BuildingStamp,
    glass: SurfaceAppearance,
    trim: SurfaceAppearance,
    simple: bool,
    resolution: FacadeResolution,
) {
    // Outline minification remains source-scale driven; retained glass is a
    // flat pane, independently of optional detail capacity.
    let full_outline = !simple
        && plane
            .projection
            .frame
            .grid_scale
            .x
            .max(plane.projection.frame.grid_scale.z)
            < f64::from(bay_width) * 0.6;
    let joinery = SurfaceAppearance {
        material: crate::material::TIMBER_WALL,
        tint: stamp.design.joinery,
    };
    let dark = SurfaceAppearance {
        tint: glass.tint.map(|v| (f32::from(v) * 0.62).round() as u8),
        ..glass
    };
    let ground = storey == 0 && front && stamp.min_h <= 0.0;
    let residential = stamp.design.use_kind == vearth_data::vectors::BuildingUse::Residential;
    let sheltered = ground
        && (!residential || style.ground_storey == GroundStorey::Veranda)
        && matches!(
            style.ground_storey,
            GroundStorey::Arcade | GroundStorey::Veranda
        );
    let shop = ground && !residential && style.ground_storey == GroundStorey::Shopfront;
    if sheltered || shop {
        let center = (x[0] + x[1]) * 0.5;
        let half = f64::from(bay_width) * 0.39;
        x = [
            (center - half).max(0.18),
            (center + half).min(length - 0.18),
        ];
        y = [floor + 0.08, floor + floor_height * 0.83];
    }
    let frame = f64::from(style.frame_m)
        .min((x[1] - x[0]) * 0.12)
        .min((y[0] - floor - 0.01).max(0.0))
        .min((floor + floor_height - y[1] - 0.01).max(0.0));
    let rim = [x[0] - frame, x[1] + frame];
    let opening = if shop || (sheltered && style.ground_storey == GroundStorey::Veranda) {
        OpeningShape::Rectangular
    } else {
        style.opening
    };
    if resolution.resolves_glazing(x[1] - x[0], y[1] - y[0])
        && !context.hidden_glazing(plane, x, y, 0.025, opening, full_outline)
    {
        plane.aperture(
            mesh,
            x,
            y,
            0.025,
            opening,
            if sheltered { dark } else { glass },
            full_outline,
        );
    }
    if shop && resolution.resolves(floor_height * 0.10) {
        let fascia = [floor + floor_height * 0.86, floor + floor_height * 0.96];
        if plane.clear(context, rim, fascia, 0.03) {
            plane.quad(mesh, rim, fascia, 0.025, joinery);
        }
    }
    let reach = f64::from(style.projection_m).clamp(0.0, 1.8);
    let projection = storey > 0
        && style.projection != ProjectionKind::None
        && reach >= 0.15
        && (matches!(
            style.projection,
            ProjectionKind::Gallery | ProjectionKind::Juliet
        ) || (bay + ((stamp.seed >> 9) & 1) as u32) % 2 == 0);
    let shutter_width = ((f64::from(bay_width) - (x[1] - x[0])) * 0.38).clamp(0.0, 0.55);
    if style.shutters != ShutterKind::None && !sheltered && !shop && shutter_width >= 0.16 {
        for side in [
            [x[0] - frame - shutter_width, x[0] - frame - 0.025],
            [x[1] + frame + 0.025, x[1] + frame + shutter_width],
        ] {
            if side[0] < 0.02 || side[1] > length - 0.02 || !plane.clear(context, side, y, 0.035) {
                continue;
            }
            plane.quad(mesh, side, y, 0.03, joinery);
        }
    }
    if sheltered {
        let reach = reach.max(f64::from(style.eave_m)).clamp(0.55, 1.8);
        let center = (x[0] + x[1]) * 0.5;
        let span = [
            (center - f64::from(bay_width) * 0.5).max(0.02),
            (center + f64::from(bay_width) * 0.5).min(length - 0.02),
        ];
        let top = floor + floor_height * 0.97;
        if plane.clear(context, span, [floor, top], reach) {
            plane.resolved_strip(
                mesh,
                span,
                [top - 0.15, top],
                [0.025, reach],
                trim,
                resolution,
            );
            if resolution.resolves(0.14) {
                for pier in [[span[0], span[0] + 0.14], [span[1] - 0.14, span[1]]] {
                    plane.structural_strip(
                        mesh,
                        pier,
                        [floor + 0.02, top - 0.15],
                        [reach - 0.15, reach],
                        trim,
                    );
                }
            }
            // The front arch/frieze and dark recessed shop opening share one
            // bay, giving a covered street edge without invented interiors.
            if style.ground_storey == GroundStorey::Arcade {
                let inner = opening_outline(x, [floor + 0.05, top - 0.15], style.opening);
                let outer = opening_outline(
                    [x[0] - 0.10, x[1] + 0.10],
                    [floor, top - 0.04],
                    style.opening,
                );
                for i in 1..inner.len().saturating_sub(1) {
                    emit_quad(
                        mesh,
                        [outer[i], outer[i + 1], inner[i + 1], inner[i]]
                            .map(|p| plane.point(p.x, p.y, reach)),
                        trim,
                        false,
                    );
                }
            }
        }
    }
    if !projection {
        return;
    }
    let span = if style.projection == ProjectionKind::Gallery {
        let center = (x[0] + x[1]) * 0.5;
        [
            (center - f64::from(bay_width) * 0.5).max(0.02),
            (center + f64::from(bay_width) * 0.5).min(length - 0.02),
        ]
    } else {
        [(x[0] - 0.16).max(0.02), (x[1] + 0.16).min(length - 0.02)]
    };
    let top = (y[1] + frame + 0.14).min(floor + floor_height - 0.025);
    if !plane.clear(context, span, [floor, top], reach + 0.10) {
        return;
    }
    match style.projection {
        ProjectionKind::TimberBay | ProjectionKind::StoneBay | ProjectionKind::ScreenBay => {
            let finish = if style.projection == ProjectionKind::StoneBay {
                trim
            } else {
                joinery
            };
            plane.structural_strip(
                mesh,
                span,
                [(y[0] - 0.16).max(floor), top],
                [0.03, reach],
                finish,
            );
            let front_x = [x[0] + 0.07, x[1] - 0.07];
            if resolution.resolves_glazing(front_x[1] - front_x[0], y[1] - y[0]) {
                plane.aperture(
                    mesh,
                    front_x,
                    y,
                    reach + 0.008,
                    style.opening,
                    glass,
                    full_outline,
                );
            }
        }
        ProjectionKind::Juliet | ProjectionKind::Balcony | ProjectionKind::Gallery => {
            let deck = floor + 0.10;
            plane.resolved_strip(mesh, span, [floor, deck], [0.025, reach], trim, resolution);
            let rail = (deck + 0.95).min(y[1] - 0.10);
            plane.structural_strip(
                mesh,
                span,
                [deck + 0.12, rail],
                [reach - 0.07, reach],
                joinery,
            );
            if style.projection == ProjectionKind::Gallery {
                plane.resolved_strip(
                    mesh,
                    span,
                    [top - 0.09, top],
                    [0.025, reach],
                    joinery,
                    resolution,
                );
            }
        }
        ProjectionKind::None => {}
    }
}

fn facade_trim(stamp: &BuildingStamp) -> SurfaceAppearance {
    if stamp.highrise.is_some() {
        return SurfaceAppearance {
            material: crate::material::PANEL_WALL,
            tint: stamp.design.joinery,
        };
    }
    use crate::regional_architecture::DetailFamily;
    let material = if let Some(style) = expressed_style(stamp) {
        if style.banding == Banding::TimberFrame {
            crate::material::TIMBER_WALL
        } else {
            crate::material::WALL
        }
    } else if matches!(
        stamp.design.detail,
        DetailFamily::Modern | DetailFamily::Industrial
    ) {
        crate::material::STEEL
    } else if stamp.design.detail == DetailFamily::Timber {
        crate::material::TIMBER_WALL
    } else {
        crate::material::WALL
    };
    SurfaceAppearance {
        material,
        tint: crate::art_direction::architecture_surface(material, stamp.design.accent),
    }
}

#[cfg(test)]
fn emit_wall(
    projection: &Projection<'_>,
    mesh: &mut MeshBuffers,
    details: &mut MeshBuffers,
    edge: [DVec2; 2],
    base: f64,
    floor: f64,
    eave: f64,
    stamp: &BuildingStamp,
    rings: &Rings,
    context: &FacadeContext,
    simple: bool,
) {
    emit_wall_with_resolution(
        projection,
        mesh,
        details,
        edge,
        base,
        floor,
        eave,
        stamp,
        rings,
        context,
        simple,
        FacadeResolution::for_frame(projection.frame),
    );
}

fn emit_wall_with_resolution(
    projection: &Projection<'_>,
    mesh: &mut MeshBuffers,
    details: &mut MeshBuffers,
    edge: [DVec2; 2],
    base: f64,
    floor: f64,
    eave: f64,
    stamp: &BuildingStamp,
    rings: &Rings,
    context: &FacadeContext,
    simple: bool,
    resolution: FacadeResolution,
) {
    use crate::regional_architecture::DetailFamily;
    if eave <= base + 0.01 {
        return;
    }
    let mut wall_stamp = *stamp;
    wall_stamp.facade.boundary = false;
    let coarse = resolution.feature_m >= 2.0 && coarse_building_pixel_m(projection.frame).is_some();
    let levels = if let Some(system) = stamp.highrise {
        let floor_m = system.profile().floor_m;
        [
            base,
            (floor + floor_m * 2.0).min(eave),
            (eave - floor_m).max(floor + floor_m * 2.0).min(eave),
            eave,
        ]
    } else {
        [
            base,
            floor + (eave - floor) * 0.13,
            floor + (eave - floor) * 0.88,
            eave,
        ]
    };
    let highrise = stamp
        .highrise
        .filter(|_| stamp.facade_openings)
        .filter(|_| edge_facade(rings, (edge[0] + edge[1]) * 0.5).edge_distance_m < 0.01);
    let simple_levels = [base, eave];
    let plinth = expressed_style(stamp).filter(|style| style.plinth_material.is_some());
    let plinth_levels = [
        base,
        (floor + plinth.map_or(0.0, |style| f64::from(style.base_height_m))).clamp(base, eave),
        eave,
    ];
    let levels: &[f64] = if simple || coarse {
        &simple_levels
    } else if plinth.is_some() {
        &plinth_levels
    } else {
        &levels
    };
    let outward = context.opaque_core.as_ref().map(|_| {
        let midpoint = (edge[0] + edge[1]) * 0.5;
        let tangent = (edge[1] - edge[0]).normalize();
        let outside = midpoint + DVec2::new(tangent.y, -tangent.x) * 0.1;
        projection.point(outside, floor) - projection.point(midpoint, floor)
    });
    let backing_start = mesh.indices.len();
    for band in levels.windows(2) {
        if band[1] <= band[0] {
            continue;
        }
        let points = [
            projection.point(edge[0], band[0]),
            projection.point(edge[1], band[0]),
            projection.point(edge[1], band[1]),
            projection.point(edge[0], band[1]),
        ];
        if let (Some(core), Some(outward)) = (&context.opaque_core, outward) {
            core.borrow_mut().wall(points, outward, band[0] == base);
        }
        if context.party_walls.iter().any(|wall| {
            wall.edge == edge
                && wall.covers_heights(band[0], band[1])
                && points.iter().all(|&point| wall.body.core.contains(point))
                && {
                    wall.body.used.set(true);
                    true
                }
        }) {
            continue;
        }
        let mut appearance = crate::building_appearance(
            (band[0] + band[1]) * 0.5,
            floor,
            eave,
            &wall_stamp,
            None,
            0.001,
        );
        if let Some(system) = highrise {
            appearance.tint = highrise_wall(stamp);
            if coarse {
                // Integrate the same selected wall/glass contract over the
                // source edge; unresolved piers need no additional geometry.
                appearance.material = system.facade_material(stamp.night_eligible);
                let source = edge_facade(rings, (edge[0] + edge[1]) * 0.5);
                let midpoint = f64::from(source.along_m);
                let half_length = edge[0].distance(edge[1]) * 0.5;
                appearance.tint = system.coarse_tint(
                    appearance.tint,
                    f64::from(source.length_m),
                    [midpoint - half_length, midpoint + half_length],
                    base,
                    floor,
                    eave,
                );
            }
        }
        if !simple
            && !coarse
            && let Some(style) = plinth
            && band[1] <= floor + f64::from(style.base_height_m)
        {
            appearance.material = style.plinth_material.expect("plinth finish");
            appearance.tint = [115, 119, 114];
        }
        let first = mesh.vertices.len();
        emit_quad(mesh, points, appearance, false);
        if coarse && highrise.is_some() {
            apply_finish_variant(&mut mesh.vertices[first..], 1);
        } else if appearance.material == stamp.design.wall_material {
            apply_finish_variant(&mut mesh.vertices[first..], stamp.design.wall_variant);
        }
    }
    let backing_end = mesh.indices.len();
    if let Some(system) = highrise {
        if coarse {
            return;
        }
        let units = emit_highrise_panels(projection, mesh, edge, floor, eave, stamp, rings, system);
        if backing_start < backing_end && !units.is_empty() {
            context.shadow_walls.borrow_mut().push(ShadowWall {
                backing: backing_start..backing_end,
                units,
            });
        }
        emit_highrise_relief(
            projection, details, edge, floor, eave, rings, context, resolution, simple, system,
            stamp,
        );
        return;
    }
    if !stamp.facade_openings
        || coarse
        || stamp.building_kind == BuildingKind::RoofOnly
        || eave - floor < 1.0
    {
        return;
    }
    let middle = (edge[0] + edge[1]) * 0.5;
    let source_edge = edge_facade(rings, middle);
    let a = edge_facade(rings, edge[0]);
    let b = edge_facade(rings, edge[1]);
    // Opening coordinates belong to original mapped edges, not Boolean cuts.
    if source_edge.edge_distance_m > 0.01
        || (a.edge != source_edge.edge && b.edge != source_edge.edge)
    {
        return;
    }
    let grammar = crate::facade_grammar(stamp);
    let floor_count = ((eave - floor) / grammar.floor_m).round().clamp(1.0, 192.0) as u32;
    let bay_count = (source_edge.length_m / grammar.bay_m)
        .round()
        .clamp(1.0, 128.0) as u32;
    let floor_height = (eave - floor) / f64::from(floor_count);
    let bay_width = source_edge.length_m / bay_count as f32;
    let direction = (edge[1] - edge[0]).normalize();
    let shadow_units = std::cell::RefCell::default();
    let plane = FacadePlane {
        shadow_units: Some(&shadow_units),
        projection,
        origin: edge[0],
        direction,
        outward: DVec2::new(direction.y, -direction.x),
    };
    let sign = (edge_facade(rings, middle + direction * 0.01).along_m
        - edge_facade(rings, middle - direction * 0.01).along_m)
        .signum() as f64;
    if sign == 0.0 {
        return;
    }
    let source_start = f64::from(source_edge.along_m) - sign * edge[0].distance(middle);
    let length = edge[0].distance(edge[1]);
    let profile = architecture_facade::production_detail_profile(&stamp.design);
    let style = expressed_style(stamp);
    let trim = facade_trim(stamp);
    let door = context
        .entrance
        .filter(|(ordinal, _)| *ordinal == source_edge.edge)
        .map(|(_, along)| (along - source_start) / sign)
        .filter(|&center| {
            center >= 0.70 && center <= length - 0.70 && stamp.min_h <= 0.0 && eave - floor >= 2.4
        })
        .map(|center| [center - 0.52, center + 0.52]);

    if let Some(style) = style {
        emit_style_bands(
            &plane,
            mesh,
            context,
            length,
            base,
            floor,
            eave,
            floor_count,
            floor_height,
            style,
            stamp,
            trim,
            resolution,
        );
    }
    if !simple && style.is_none() && resolution.resolves(0.14) {
        let x = [0.03, length - 0.03];
        for (height, thickness, reach) in [
            (floor + 0.15, 0.14, profile.cornice_m * 0.6),
            (eave - 0.16, 0.14, profile.cornice_m),
        ] {
            let y = [height - thickness * 0.5, height + thickness * 0.5];
            if x[1] > x[0] && y[0] >= base && plane.clear(context, x, y, reach) {
                plane.quad(details, x, y, 0.02, trim);
            }
        }
        // Slab edges and restrained string courses organize the full facade.
        // Their cadence is floor-based and remains the same after cell clipping.
        if matches!(
            stamp.design.detail,
            DetailFamily::Balcony | DetailFamily::Classical
        ) {
            for storey in 1..floor_count {
                if stamp.design.detail == DetailFamily::Classical && storey != 1 {
                    continue;
                }
                let height = floor + f64::from(storey) * floor_height;
                let y = [height - 0.12, height - 0.015];
                if plane.clear(context, x, y, 0.09) {
                    plane.quad(details, x, y, 0.02, trim);
                }
            }
        }
    }

    for storey in 0..floor_count {
        let (xb, yb) = crate::facade_floor_bands(grammar, storey, floor_count);
        let nominal_h0 = floor + (f64::from(storey) + yb.0) * floor_height;
        let h1 = floor + (f64::from(storey) + yb.1) * floor_height;
        for bay in 0..bay_count {
            let a = (f64::from((bay as f32 + xb.0) * bay_width) - source_start) / sign;
            let b = (f64::from((bay as f32 + xb.1) * bay_width) - source_start) / sign;
            let (d0, d1) = (a.min(b), a.max(b));
            if d0 < 0.14 || d1 > length - 0.14 || d1 - d0 < 0.25 {
                continue;
            }
            if storey == 0 && door.is_some_and(|door| door[0] < d1 + 0.10 && door[1] > d0 - 0.10) {
                continue;
            }
            let balcony = style.is_none()
                && stamp.design.detail == DetailFamily::Balcony
                && storey > 0
                && (bay + ((stamp.seed >> 7) as u32).wrapping_add(u32::from(source_edge.edge)) % 3)
                    % 3
                    == 0
                && bay_width >= 2.0;
            let h0 = if balcony {
                floor + f64::from(storey) * floor_height + 0.14
            } else {
                nominal_h0
            };
            let glazing_resolved = resolution.resolves_glazing(d1 - d0, h1 - h0);
            let generic_detail_resolved = !simple
                && resolution.resolves(profile.shade_m.max(if balcony { 0.86 } else { 0.0 }))
                && details.vertices.len() + 480 <= MAX_FACADE_DETAIL_VERTICES;
            if style.is_none() && !glazing_resolved && !generic_detail_resolved {
                continue;
            }
            let mut sample = *stamp;
            sample.facade = FacadeStamp {
                along_m: (bay as f32 + (xb.0 + xb.1) * 0.5) * bay_width,
                ..source_edge
            };
            let mut glass = crate::building_appearance(
                (nominal_h0 + h1) * 0.5,
                floor,
                eave,
                &sample,
                None,
                0.001,
            );
            if !matches!(
                glass.material,
                crate::material::WINDOW | crate::material::LIT_WINDOW
            ) {
                continue;
            }
            // The stable room state chooses night emission, while the whole
            // source building shares its regional daytime glass tone.
            glass.tint = crate::facade_window_color(stamp, stamp.seed);
            if let Some(style) = style {
                let front = context
                    .entrance
                    .is_some_and(|(ordinal, _)| ordinal == source_edge.edge);
                emit_style_bay(
                    &plane,
                    mesh,
                    context,
                    [d0, d1],
                    [nominal_h0, h1],
                    floor + f64::from(storey) * floor_height,
                    floor_height,
                    length,
                    bay_width,
                    storey,
                    bay,
                    front,
                    style,
                    stamp,
                    glass,
                    trim,
                    simple,
                    resolution,
                );
                continue;
            }
            // Resolvable glazing remains mandatory regardless of the optional budget.
            if glazing_resolved
                && !context.hidden_glazing(
                    &plane,
                    [d0, d1],
                    [h0, h1],
                    0.018,
                    OpeningShape::Rectangular,
                    false,
                )
            {
                let first_index = mesh.indices.len();
                plane.quad(mesh, [d0, d1], [h0, h1], 0.018, glass);
                plane.record_shadow_unit(mesh, first_index, true);
            }
            if !generic_detail_resolved {
                continue;
            }
            if profile.shade_m > 0.0 {
                let x = [d0 - 0.12, d1 + 0.12];
                let y = [h1 + 0.09, h1 + 0.18];
                if plane.clear(context, x, y, profile.shade_m) {
                    plane.strip(details, x, y, [0.025, profile.shade_m], trim);
                }
            }
            if balcony {
                let deck = floor + f64::from(storey) * floor_height + 0.08;
                let x = [d0 - 0.18, d1 + 0.18];
                if !plane.clear(context, x, [deck - 0.13, deck + 0.98], 0.65) {
                    continue;
                }
                plane.strip(details, x, [deck - 0.13, deck], [0.025, 0.65], trim);
                // Opaque parapets avoid alpha overdraw and a thicket of thin rails.
                plane.strip(details, x, [deck + 0.12, deck + 0.98], [0.57, 0.65], trim);
                for x in [[x[0], x[0] + 0.07], [x[1] - 0.07, x[1]]] {
                    plane.strip(details, x, [deck + 0.12, deck + 0.98], [0.025, 0.65], trim);
                }
            }
        }
    }
    if let Some(x) = door {
        let top = floor + 2.15f64.min(floor_height * 0.88);
        let material = if matches!(
            stamp.design.detail,
            DetailFamily::Modern | DetailFamily::Industrial
        ) {
            crate::material::STEEL
        } else {
            crate::material::TIMBER_WALL
        };
        let finish = SurfaceAppearance {
            material,
            tint: stamp.design.joinery,
        };
        if let Some(style) = style {
            let frame = f64::from(style.frame_m).clamp(0.06, 0.14);
            plane.aperture(
                mesh,
                x,
                [floor + frame + 0.01, top],
                0.024,
                style.entrance,
                finish,
                !simple,
            );
        } else {
            plane.quad(mesh, x, [floor + 0.025, top], 0.024, finish);
        }
    }

    let units = shadow_units.into_inner();
    if backing_start < backing_end && !units.is_empty() {
        context.shadow_walls.borrow_mut().push(ShadowWall {
            backing: backing_start..backing_end,
            units,
        });
    }
}

// Fitted panels follow the complete mapped edge, including fragments shared
// across chunks. Their count is bounded independently of the number of floors.
fn highrise_wall(stamp: &BuildingStamp) -> [u8; 3] {
    let palette =
        crate::art_direction::architecture_surface(stamp.design.wall_material, stamp.design.wall);
    let shade = 0.96 + ((stamp.seed >> 23) & 7) as f32 * 0.01;
    palette.map(|channel| (f32::from(channel) * shade).round().clamp(0.0, 255.0) as u8)
}

fn emit_highrise_panels(
    projection: &Projection<'_>,
    mesh: &mut MeshBuffers,
    edge: [DVec2; 2],
    floor: f64,
    eave: f64,
    stamp: &BuildingStamp,
    rings: &Rings,
    system: crate::highrise::System,
) -> Vec<std::ops::Range<usize>> {
    let length = edge[0].distance(edge[1]);
    if length < 0.02 || eave - floor < 4.0 {
        return Vec::new();
    }
    let midpoint = (edge[0] + edge[1]) * 0.5;
    let source = edge_facade(rings, midpoint);
    let direction = (edge[1] - edge[0]).normalize();
    let sign =
        (edge_facade(rings, midpoint + direction * 0.01).along_m - source.along_m).signum() as f64;
    if sign == 0.0 {
        return Vec::new();
    }
    let start = f64::from(source.along_m) - sign * length * 0.5;
    let layout = system.panel_layout(f64::from(source.length_m), eave - floor);
    let (count, width, pier) = (layout.count, layout.width, layout.pier);
    let bottom = floor + layout.bottom_inset;
    let top = eave - layout.top_inset;
    let units = std::cell::RefCell::default();
    let plane = FacadePlane {
        shadow_units: Some(&units),
        projection,
        origin: edge[0],
        direction,
        outward: DVec2::new(direction.y, -direction.x),
    };
    let finish = SurfaceAppearance {
        material: system.facade_material(stamp.night_eligible),
        tint: highrise_wall(stamp),
    };
    for bay in 0..count {
        let a = (f64::from(bay) * width + pier - start) / sign;
        let b = (f64::from(bay + 1) * width - pier - start) / sign;
        let x = [a.min(b).max(0.0), a.max(b).min(length)];
        if x[1] - x[0] < 0.01 {
            continue;
        }
        let first = mesh.indices.len();
        plane.quad(mesh, x, [bottom, top], 0.018, finish);
        plane.record_shadow_unit(mesh, first, true);
    }
    units.into_inner()
}

// Large facade divisions survive in the opaque panel material. Only resolved
// physical projections enter the optional detail stream, with a per-source cap.
fn emit_highrise_relief(
    projection: &Projection<'_>,
    details: &mut MeshBuffers,
    edge: [DVec2; 2],
    floor: f64,
    eave: f64,
    rings: &Rings,
    context: &FacadeContext,
    resolution: FacadeResolution,
    simple: bool,
    system: crate::highrise::System,
    stamp: &BuildingStamp,
) {
    if simple || details.vertices.len() >= 4096 || eave - floor < 4.0 {
        return;
    }
    let profile = system.profile();
    let length = edge[0].distance(edge[1]);
    if length < 2.0 {
        return;
    }
    let direction = (edge[1] - edge[0]).normalize();
    let plane = FacadePlane {
        shadow_units: None,
        projection,
        origin: edge[0],
        direction,
        outward: DVec2::new(direction.y, -direction.x),
    };
    let trim = SurfaceAppearance {
        material: crate::material::PANEL_WALL,
        tint: stamp.design.joinery,
    };
    let dark = SurfaceAppearance {
        material: crate::material::PANEL_WALL,
        tint: stamp.design.wall.map(|v| (f32::from(v) * 0.60) as u8),
    };
    let x = [0.20, length - 0.20];
    let datum = 0.0;
    let first = ((floor - datum) / profile.floor_m).ceil().max(0.0) as u32;
    let last = ((eave - datum) / profile.floor_m).floor().max(0.0) as u32;
    for storey in first..last.min(first.saturating_add(256)) {
        if details.vertices.len() + 96 > 4096 {
            break;
        }
        let y = datum + f64::from(storey) * profile.floor_m;
        let mechanical = profile.mechanical_every > 0
            && storey > 2
            && storey.is_multiple_of(profile.mechanical_every);
        if mechanical && resolution.resolves(1.0) && y + 1.0 < eave {
            if plane.clear(context, x, [y, y + 1.0], 0.16) {
                plane.strip(details, x, [y, y + 1.0], [0.025, 0.16], dark);
            }
        } else if profile.balcony_m > 0.0 && storey > 1 && resolution.resolves(profile.balcony_m) {
            if plane.clear(context, x, [y - 0.18, y + 0.88], profile.balcony_m) {
                plane.strip(details, x, [y - 0.18, y], [0.02, profile.balcony_m], trim);
                plane.strip(
                    details,
                    x,
                    [y + 0.12, y + 0.86],
                    [profile.balcony_m - 0.10, profile.balcony_m],
                    trim,
                );
            }
        }
    }
    if resolution.resolves(0.30) && details.vertices.len() + 96 <= 4096 {
        // Crown and entrance canopy are confined to this exposed source face.
        for (y, reach) in [(eave - 0.40, 0.30), (floor + profile.floor_m * 2.0, 0.65)] {
            if y > floor && y + 0.30 < eave && plane.clear(context, x, [y, y + 0.30], reach) {
                plane.strip(details, x, [y, y + 0.30], [0.02, reach], trim);
            }
        }
    }
    if profile.pier_m >= 0.60 && resolution.resolves(0.28) {
        let midpoint = (edge[0] + edge[1]) * 0.5;
        let source = edge_facade(rings, midpoint);
        let sign = (edge_facade(rings, midpoint + direction * 0.01).along_m - source.along_m)
            .signum() as f64;
        if sign == 0.0 {
            return;
        }
        let start = f64::from(source.along_m) - sign * length * 0.5;
        let bays = (f64::from(source.length_m) / profile.bay_m.max(4.0))
            .round()
            .clamp(1.0, 12.0) as u32;
        for bay in 1..bays {
            if details.vertices.len() + 48 > 4096 {
                break;
            }
            let center =
                (f64::from(source.length_m) * f64::from(bay) / f64::from(bays) - start) / sign;
            let x = [
                (center - profile.pier_m * 0.25).max(0.0),
                (center + profile.pier_m * 0.25).min(length),
            ];
            if x[1] - x[0] > 0.01 && plane.clear(context, x, [floor + 0.20, eave - 0.60], 0.28) {
                plane.strip(details, x, [floor + 0.20, eave - 0.60], [0.02, 0.28], trim);
            }
        }
    }
}

fn emit_highrise_roof_plant(
    projection: &Projection<'_>,
    details: &mut MeshBuffers,
    domain: &Footprint<'_>,
    source: &Building,
    roof: FlatRoof,
    stamp: &BuildingStamp,
) {
    if roof.top - roof.deck < 1.2 || details.vertices.len() + 60 > 4096 {
        return;
    }
    let Some(system) = stamp.highrise else {
        return;
    };
    if source.seed % 3 != 0 {
        return;
    }
    let Some((lo, hi)) = bounds(&source.rings) else {
        return;
    };
    let size = (hi - lo) * 0.18;
    if size.min_element() < 2.0 {
        return;
    }
    let shift = DVec2::new(
        ((source.seed >> 5) & 3) as f64 - 1.5,
        ((source.seed >> 9) & 3) as f64 - 1.5,
    ) * 0.06;
    let center = (lo + hi) * 0.5 + (hi - lo) * shift;
    let half = size.min(DVec2::splat(8.0)) * 0.5;
    let corners = [
        center - half,
        center + DVec2::new(half.x, -half.y),
        center + half,
        center + DVec2::new(-half.x, half.y),
    ];
    // A crossing courtyard or excluded upper part rejects the entire box.
    let box_lo = center - half;
    let box_hi = center + half;
    let inside = |p: DVec2| p.cmpge(box_lo).all() && p.cmple(box_hi).all();
    if !corners.into_iter().all(|p| domain.contains(p))
        || domain.edges.iter().any(|edge| {
            edge.iter().copied().any(inside)
                || (0..4).any(|i| intersection(*edge, [corners[i], corners[(i + 1) % 4]]).is_some())
        })
    {
        return;
    }
    let finish = SurfaceAppearance {
        material: crate::material::PANEL_WALL,
        tint: system
            .profile()
            .wall
            .map(|v| (f32::from(v) * 0.72).round() as u8),
    };
    let top = roof.top - 0.20;
    emit_quad(
        details,
        corners.map(|p| projection.point(p, top)),
        finish,
        true,
    );
    for i in 0..4 {
        let a = corners[i];
        let b = corners[(i + 1) % 4];
        emit_quad(
            details,
            [
                projection.point(b, roof.deck),
                projection.point(a, roof.deck),
                projection.point(a, top),
                projection.point(b, top),
            ],
            finish,
            false,
        );
    }
}

struct RoofTrace {
    started: vearth_platform::time::Instant,
    record: crate::contracts::SlowRoofRecord,
}
impl RoofTrace {
    fn start(
        frame: &ChunkFrame,
        source: &Building,
        ordinal: usize,
        roof: &RoofField,
        simple: bool,
        measured: bool,
        parent_body: bool,
    ) -> Option<Self> {
        if !crate::contracts::generation_timing_enabled() {
            return None;
        }
        let shape = match roof.shape {
            None => 0,
            Some(RoofShape::Flat) => 1,
            Some(RoofShape::Gabled) => 2,
            Some(RoofShape::Hipped) => 3,
            Some(RoofShape::Pyramidal) => 4,
            Some(RoofShape::Mansard) => 5,
            Some(RoofShape::Gambrel) => 6,
            Some(RoofShape::Skillion) => 7,
            Some(RoofShape::Dome) => 8,
            Some(RoofShape::Onion) => 9,
            Some(RoofShape::Sawtooth) => 10,
        };
        Some(Self {
            started: vearth_platform::time::Instant::now(),
            record: crate::contracts::SlowRoofRecord {
                cell_face: frame.addr.cell.face.0,
                cell_level: frame.addr.cell.level,
                cell_i: frame.addr.cell.i,
                cell_j: frame.addr.cell.j,
                source_ordinal: ordinal as u32,
                seed: source.seed,
                parent_body,
                simple,
                shape,
                reentrant: roof.reentrant,
                tiered: roof.tiered,
                measured,
                rings: source.rings.len() as u32,
                edges: roof.edges.len() as u32,
                extent_m: (roof.bounds[1] - roof.bounds[0]).to_array(),
                rise_m: roof.top - roof.eave,
                pitch: roof.pitch,
                work: crate::contracts::RoofWorkCounters::default(),
            },
        })
    }
}
impl Drop for RoofTrace {
    fn drop(&mut self) {
        self.record.work.elapsed_ns = self.started.elapsed().as_nanos() as u64;
        crate::contracts::record_roof(self.record);
    }
}

const ROOF_PLANE_ERROR_M: f64 = 0.012;
const MAX_ROOF_REFINEMENT: u8 = 9;
const MAX_ROOF_TRIANGLES: usize = 65_536;

#[derive(Clone, Copy)]
struct RoofResolution {
    sample_error: f64,
    cap_scale: f64,
    envelope_rise_m: f64,
    /// Interior sampling floor on refining levels with a finer level above
    /// them: a quarter of the pixel at the selector's three-pixel target.
    pixel_error: f64,
}

impl RoofResolution {
    fn for_frame(frame: &ChunkFrame) -> Self {
        Self {
            sample_error: frame.grid_scale.x.min(frame.grid_scale.z) / 60.0,
            // Grow at most one octave per coarser level. The original near
            // policy remains exact, including its minimum error for low roofs.
            cap_scale: f64::from(1u32 << 17u8.saturating_sub(frame.addr.cell.level).min(3))
                .min(5.0),
            // This is a source-level model allowance. Loading ancestors and
            // saturated cuts can exceed the selector's three-pixel target.
            envelope_rise_m: if frame.addr.cell.level < 17 {
                frame.addr.cell.sample_pitch_m() / 12.0
            } else {
                0.0
            },
            // The flat-cap allowance above already accepts this vertical
            // deviation. The finest level has no floor: the camera may
            // approach it without a finer replacement.
            pixel_error: if coarse_building_pixel_m(frame).is_none()
                && frame.addr.cell.level < vearth_core::cell::MAX_LEVEL
            {
                frame.addr.cell.sample_pitch_m() / 12.0
            } else {
                0.0
            },
        }
    }

    fn contained_cap_top(
        self,
        projection: &Projection<'_>,
        source: &Building,
        stamp: &BuildingStamp,
        roof: &RoofField,
    ) -> Option<f64> {
        let rise = roof.top - roof.eave;
        if stamp.lidar
            || roof.tiered
            || source.roof_shape.is_some()
            || source.kind == BuildingKind::RoofOnly
            || !matches!(
                roof.shape,
                Some(
                    RoofShape::Gabled
                        | RoofShape::Hipped
                        | RoofShape::Pyramidal
                        | RoofShape::Mansard
                        | RoofShape::Gambrel
                        | RoofShape::Skillion
                )
            )
            || !rise.is_finite()
            || rise <= 0.0
            || !(rise <= self.envelope_rise_m)
        {
            return None;
        }
        // Both the old field and its constant top stay in this complete
        // geodetic height interval. Keep source facts and signature roofs.
        let fascia = architecture_facade::production_detail_profile(&stamp.design).fascia_m;
        let (lo, hi) = bounds(&source.rings)?;
        // emit_eave caps horizontal reach at two metres; its fascia moves
        // with the roof. Gable crowns are replaced by the shared cap closure.
        let (center, radius) = roof_envelope_sphere(
            projection,
            [lo - DVec2::splat(2.0), hi + DVec2::splat(2.0)],
            [roof.eave - fascia, roof.top],
        )?;
        roof_sphere_inside_owner(projection.frame, center, radius).then_some(roof.top)
    }

    fn interior_allowance(self, rise: f64, shape: Option<RoofShape>) -> f64 {
        // Square-root skirts have unbounded curvature at their rim. Keep their
        // existing sampling until a curvature-aware decomposition is available.
        if matches!(shape, Some(RoofShape::Dome | RoofShape::Onion)) {
            return ROOF_PLANE_ERROR_M;
        }
        // Only interior refinement uses this source-resolution allowance.
        // Source grids, boundary curves and planar-fit validation stay fixed.
        self.sample_error
            .min(0.10 * self.cap_scale)
            .min(rise.max(0.0) * 0.02 * self.cap_scale)
            .max(ROOF_PLANE_ERROR_M)
    }

    fn sampling_tolerance(self, rise: f64, shape: Option<RoofShape>) -> f64 {
        let allowance = self.interior_allowance(rise, shape);
        if matches!(shape, Some(RoofShape::Dome | RoofShape::Onion)) {
            return allowance;
        }
        let source_tolerance = if self.cap_scale == 1.0 {
            allowance
        } else {
            let original = Self {
                cap_scale: 1.0,
                ..self
            }
            .interior_allowance(rise, shape);
            // Seven probe positions do not certify the entire triangle. Reserve
            // half the extra far allowance instead of spending it all on probes.
            original + (allowance - original) * 0.5
        };
        source_tolerance.max(self.pixel_error)
    }

    #[cfg(test)]
    fn original(frame: &ChunkFrame) -> Self {
        Self {
            sample_error: frame.grid_scale.x.min(frame.grid_scale.z) / 60.0,
            cap_scale: 1.0,
            envelope_rise_m: 0.0,
            pixel_error: 0.0,
        }
    }
}

fn roof_envelope_sphere(
    projection: &Projection<'_>,
    bounds: [DVec2; 2],
    heights: [f64; 2],
) -> Option<(DVec3, f64)> {
    let [lo, hi] = bounds;
    if !lo.is_finite()
        || !hi.is_finite()
        || (hi - lo).min_element() < 0.0
        || !heights.iter().all(|h| h.is_finite())
        || heights[1] < heights[0]
    {
        return None;
    }
    let latitudes = [
        projection.lat + lo.y / MEAN_RADIUS,
        projection.lat + hi.y / MEAN_RADIUS,
    ];
    if latitudes
        .iter()
        .any(|lat| !lat.is_finite() || lat.abs() > std::f64::consts::FRAC_PI_2)
    {
        return None;
    }
    let middle = (lo + hi) * 0.5;
    let height = (heights[0] + heights[1]) * 0.5;
    let center = projection.world_point(middle, height);
    // Longitude span is measured before wrapping at the dateline. Along a
    // geodetic path the latitude derivative is at most A²/B+|h|, longitude
    // adds cos(latitude), and the height derivative has unit length.
    let half_latitude = (hi.y - lo.y) * 0.5 / MEAN_RADIUS;
    let half_longitude =
        (hi.x - lo.x) * 0.5 / (MEAN_RADIUS * projection.lat.cos().abs().max(1.0e-6));
    let max_cos = if latitudes[0] <= 0.0 && latitudes[1] >= 0.0 {
        1.0
    } else {
        latitudes[0].cos().max(latitudes[1].cos())
    }
    .next_up();
    let derivative = (ellipsoid::WGS84_A * ellipsoid::WGS84_A / ellipsoid::WGS84_B
        + heights[0].abs().max(heights[1].abs()))
    .next_up();
    let mut radius =
        derivative * (half_latitude + half_longitude * max_cos) + (heights[1] - heights[0]) * 0.5;
    let frame = projection.frame;
    let bx = frame.x_axis * frame.grid_scale.x;
    let by = frame.y_axis * frame.grid_scale.y;
    let bz = frame.z_axis * frame.grid_scale.z;
    let determinant = bx.dot(by.cross(bz));
    let dual_norms = DVec3::new(
        by.cross(bz).length(),
        bz.cross(bx).length(),
        bx.cross(by).length(),
    ) / determinant.abs();
    let local_extent = frame.ecef_to_local(center).abs() + dual_norms * radius + DVec3::ONE;
    // Include the eventual local-f32 vertex storage as well as arithmetic in
    // the enclosing sphere and plane tests. No rotated-f32 roof is used here.
    radius += 1024.0 * f64::EPSILON * (center.abs().element_sum() + derivative + radius)
        + 64.0 * f64::from(f32::EPSILON) * local_extent.dot(frame.grid_scale.abs());
    (center.is_finite() && radius.is_finite() && radius >= 0.0)
        .then_some((center, radius.next_up()))
}

fn roof_sphere_inside_owner(frame: &ChunkFrame, center: DVec3, radius: f64) -> bool {
    if !center.is_finite() || !radius.is_finite() || radius < 0.0 {
        return false;
    }
    let normals = frame.face_normals();
    for (normal, axis, scale) in [
        (normals[0], frame.x_axis, frame.grid_scale.x),
        (normals[2], frame.z_axis, frame.grid_scale.z),
    ] {
        if !(normal.dot(center - frame.origin) > radius)
            || !(normal.dot(frame.origin + axis * (N as f64 * scale) - center) > radius)
        {
            return false;
        }
    }
    // Coverage uses geocentric cube cones, while generation clips a skewed
    // geodetic tangent prism. Require both so a capped component cannot cross
    // a settled mixed-LOD owner boundary. Partial arrivals retain whole parents.
    let cell = frame.addr.cell;
    let (u0, v0, u1, v1) = cell.uv_bounds();
    let n = cell.face.normal();
    let right = cell.face.right();
    let up = cell.face.up();
    let tan = |v: f64| (v * std::f64::consts::FRAC_PI_4).tan();
    [
        right - n * tan(u0),
        n * tan(u1) - right,
        up - n * tan(v0),
        n * tan(v1) - up,
    ]
    .into_iter()
    .all(|normal| normal.normalize().dot(center) > radius)
}

#[cfg(test)]
fn interior_roof_error(frame: &ChunkFrame, rise: f64, shape: Option<RoofShape>) -> f64 {
    RoofResolution::for_frame(frame).sampling_tolerance(rise, shape)
}

#[derive(Clone, Copy)]
struct RoofTriangle {
    points: [DVec2; 3],
    heights: [f64; 3],
}

#[derive(Clone, Copy)]
struct RoofBoundary {
    points: [DVec2; 2],
    heights: [f64; 2],
}

struct RoofGeometry {
    triangles: Vec<RoofTriangle>,
    boundary: Vec<RoofBoundary>,
}

#[derive(Clone, Copy)]
struct RoofPlane {
    origin: DVec2,
    gradient: DVec2,
    height: f64,
}
impl RoofPlane {
    fn at(&self, p: DVec2) -> f64 {
        self.height + self.gradient.dot(p - self.origin)
    }
}

fn clip_roof_plane(polygon: &[DVec2], plane: RoofPlane, other: RoofPlane) -> Vec<DVec2> {
    let mut result = Vec::new();
    for i in 0..polygon.len() {
        let a = polygon[i];
        let b = polygon[(i + 1) % polygon.len()];
        let da = plane.at(a) - other.at(a);
        let db = plane.at(b) - other.at(b);
        if da <= 0.0 {
            result.push(a);
        }
        if (da <= 0.0) != (db <= 0.0) {
            result.push(a.lerp(b, da / (da - db)));
        }
    }
    result.dedup_by(|a, b| a.distance_squared(*b) < 1.0e-16);
    result
}

fn point_on_roof_edge(point: DVec2, [a, b]: [DVec2; 2]) -> bool {
    let length2 = a.distance_squared(b);
    if length2 < 1.0e-16 {
        return point.distance_squared(a) < 1.0e-16;
    }
    let t = ((point - a).dot(b - a) / length2).clamp(0.0, 1.0);
    point.distance_squared(a + (b - a) * t) < 1.0e-10
}

fn snap_roof_boundary(point: DVec2, boundary: &[[DVec2; 2]]) -> DVec2 {
    for [a, b] in boundary {
        let t = ((point - *a).dot(*b - *a) / a.distance_squared(*b)).clamp(0.0, 1.0);
        let closest = a.lerp(*b, t);
        if point.distance_squared(closest) < 1.0e-8 {
            return closest;
        }
    }
    point
}

fn analytic_roof(
    footprint: &Footprint<'_>,
    boundary: &[[DVec2; 2]],
    roof: &RoofField,
    rotated: bool,
    perimeter_step_m: f64,
    projection: &Projection<'_>,
) -> Option<RoofGeometry> {
    let planes = roof.planes()?;
    let unrotate = |p| if rotated { roof.unrotate(p) } else { p };
    let cells = footprint.roof_cells(4096.0)?;
    // Only the boundary needs the former grid's fine partitions: eave
    // obstruction and profile sampling remain local while interiors use planes.
    let perimeter: Vec<_> = footprint
        .roof_perimeter_samples(perimeter_step_m)
        .or_else(|| footprint.roof_perimeter_samples(4096.0))?
        .into_iter()
        .map(unrotate)
        .collect();
    analytic_roof_geometry(
        cells.into_iter().map(|cell| cell.map(unrotate)),
        boundary,
        &perimeter,
        roof,
        &planes,
        &|p, h| projection.world_point(p, h),
    )
}

fn analytic_roof_geometry(
    cells: impl IntoIterator<Item = [DVec2; 4]>,
    boundary: &[[DVec2; 2]],
    perimeter: &[DVec2],
    roof: &RoofField,
    planes: &[RoofPlane],
    project: &dyn Fn(DVec2, f64) -> DVec3,
) -> Option<RoofGeometry> {
    let mut triangles = Vec::new();
    let perimeter: Vec<_> = perimeter
        .iter()
        .map(|&point| snap_roof_boundary(point, boundary))
        .collect();
    let mut domain_area = 0.0;
    let mut emitted_area = 0.0;
    for cell in cells {
        let cell = cell.map(|p| snap_roof_boundary(p, boundary));
        domain_area += (0..4)
            .map(|i| (cell[i] - cell[0]).perp_dot(cell[(i + 1) % 4] - cell[0]))
            .sum::<f64>()
            .abs();
        for (i, plane) in planes.iter().enumerate() {
            let mut polygon = cell.to_vec();
            for (j, other) in planes.iter().enumerate() {
                if i != j {
                    polygon = clip_roof_plane(&polygon, *plane, *other);
                }
                if polygon.len() < 3 {
                    break;
                }
            }
            if polygon.len() < 3 {
                continue;
            }
            // The cone has a flat apron outside its polygonal base. Split
            // each face at the eave before triangulating either affine region.
            let floor = RoofPlane {
                origin: plane.origin,
                gradient: DVec2::ZERO,
                height: roof.eave,
            };
            let regions = if roof.conical {
                [
                    (clip_roof_plane(&polygon, floor, *plane), *plane),
                    (clip_roof_plane(&polygon, *plane, floor), floor),
                ]
            } else {
                [(polygon, *plane), (Vec::new(), floor)]
            };
            for (polygon, plane) in regions {
                if polygon.len() < 3 {
                    continue;
                }
                let mut outline = Vec::new();
                for i in 0..polygon.len() {
                    let a = polygon[i];
                    let b = polygon[(i + 1) % polygon.len()];
                    let delta = b - a;
                    let length2 = delta.length_squared();
                    if length2 < 1.0e-16 {
                        continue;
                    }
                    let mut cuts = vec![(0.0, a)];
                    for &point in &perimeter {
                        let t = (point - a).dot(delta) / length2;
                        if t > 0.0 && t < 1.0 && point_on_roof_edge(point, [a, b]) {
                            cuts.push((t, point));
                        }
                    }
                    cuts.sort_by(|a, b| a.0.total_cmp(&b.0));
                    cuts.dedup_by(|a, b| a.1.distance_squared(b.1) < 1.0e-16);
                    outline.extend(cuts.into_iter().map(|(_, point)| point));
                }
                if outline.len() < 3 {
                    continue;
                }
                // A centroid fan keeps every collinear perimeter knot in the
                // emitted mesh. Those knots retain local eave clearance decisions.
                let center = outline.iter().copied().sum::<DVec2>() / outline.len() as f64;
                for i in 0..outline.len() {
                    let points = [outline[i], outline[(i + 1) % outline.len()], center];
                    let area = (points[1] - points[0]).perp_dot(points[2] - points[0]);
                    if area.abs() < 1.0e-12 {
                        continue;
                    }
                    let heights = points.map(|p| roof.height(p));
                    if heights.iter().any(|h| !h.is_finite())
                        || (0..3).any(|j| (heights[j] - plane.at(points[j])).abs() > 1.0e-5)
                    {
                        return None;
                    }
                    let world: [_; 3] = std::array::from_fn(|j| project(points[j], heights[j]));
                    let probes = [
                        ([0.5, 0.5, 0.0], (world[0] + world[1]) * 0.5),
                        ([0.0, 0.5, 0.5], (world[1] + world[2]) * 0.5),
                        ([0.5, 0.0, 0.5], (world[0] + world[2]) * 0.5),
                        ([1.0 / 3.0; 3], (world[0] + world[1] + world[2]) / 3.0),
                    ];
                    for (weights, chord) in probes {
                        let p = points[0] * weights[0]
                            + points[1] * weights[1]
                            + points[2] * weights[2];
                        let h = roof.height(p);
                        let interpolated = heights[0] * weights[0]
                            + heights[1] * weights[1]
                            + heights[2] * weights[2];
                        // The EN height field is planar, but the Earth projection
                        // is not. Long roofs and polar distortion retain sampling
                        // unless their rendered chord also meets the metric bound.
                        if !h.is_finite()
                            || (h - interpolated).abs() > ROOF_PLANE_ERROR_M
                            || project(p, h).distance(chord) > ROOF_PLANE_ERROR_M
                        {
                            return None;
                        }
                    }
                    emitted_area += area.abs();
                    triangles.push(RoofTriangle { points, heights });
                    if triangles.len() > MAX_ROOF_TRIANGLES {
                        return None;
                    }
                }
            }
        }
    }
    if (domain_area - emitted_area).abs() > domain_area.max(1.0) * 1.0e-7 {
        return None;
    }
    let sampled_boundary = RoofBoundaryIndex::new(&triangles).boundary(boundary);
    let original_length: f64 = boundary.iter().map(|edge| edge[0].distance(edge[1])).sum();
    let emitted_length: f64 = sampled_boundary
        .iter()
        .map(|edge| edge.points[0].distance(edge.points[1]))
        .sum();
    if (original_length - emitted_length).abs() > 1.0e-4 {
        return None;
    }
    Some(RoofGeometry {
        triangles,
        boundary: sampled_boundary,
    })
}

// A lower envelope of affine planes is concave. The existing fit requires
// every known height to be within 1mm, so a midpoint/centroid deficit above
// 2mm is impossible even before discovering the candidate planes. Bounds must
// describe the entire callback, including offset probes, not just these seven
// samples; unbounded/measured callbacks retain the original fitter.
fn roof_plane_roundoff_bound(
    points: [DVec2; 3],
    heights: [f64; 7],
    bounds: [f64; 2],
    a: DVec2,
    b: DVec2,
    determinant: f64,
) -> Option<f64> {
    if !bounds.into_iter().all(f64::is_finite)
        || bounds[0] > bounds[1]
        || !points.into_iter().all(DVec2::is_finite)
        || !heights
            .into_iter()
            .all(|h| h.is_finite() && h >= bounds[0] && h <= bounds[1])
        || !determinant.is_finite()
        || determinant.abs() < 1.0e-12
    {
        return None;
    }
    // Mirror the actual rounded difference/product/subtraction/division path
    // with outward bounds, including skinny triangles' amplified gradients.
    let delta = (bounds[1] - bounds[0]).next_up();
    let gradient_bound = |x: f64, y: f64| {
        (((delta * x.abs()).next_up() + (delta * y.abs()).next_up()).next_up() / determinant.abs())
            .next_up()
    };
    let gradient = (gradient_bound(b.y, a.y) + gradient_bound(a.x, b.x)).next_up();
    let coordinate = points
        .into_iter()
        .map(|p| p.abs().max_element())
        .fold(0.0, f64::max);
    let height = bounds[0].abs().max(bounds[1].abs());
    // Candidate construction, affine evaluation and convex-combination
    // locations use fewer than 64 rounded operations. This scale includes the
    // weighted probes/origin offsets (each bounded by twice |coordinate|),
    // both dot products and rounded known-height means. Loose/nonfinite
    // bounds simply retain the existing fit.
    let scale = (height + (16.0 * coordinate * gradient).next_up() + 1.0).next_up();
    let gamma = (64.0 * f64::EPSILON) / (1.0 - 64.0 * f64::EPSILON);
    let roundoff = (gamma * scale).next_up();
    (roundoff.is_finite() && roundoff <= 0.001).then_some(roundoff)
}

fn roof_sample_concavity_deficit(heights: [f64; 7], roundoff: f64) -> bool {
    let allowance = (0.002 + roundoff).next_up();
    (0..3).any(|i| (heights[i] + heights[(i + 1) % 3]) * 0.5 - heights[i + 3] > allowance)
        || heights[..3].iter().sum::<f64>() / 3.0 - heights[6] > allowance
}

#[cfg(test)]
fn roof_samples_cannot_be_concave(
    points: [DVec2; 3],
    heights: [f64; 7],
    bounds: [f64; 2],
    a: DVec2,
    b: DVec2,
    determinant: f64,
) -> bool {
    roof_plane_roundoff_bound(points, heights, bounds, a, b, determinant)
        .is_some_and(|roundoff| roof_sample_concavity_deficit(heights, roundoff))
}

struct RoofSampler<
    'a,
    const CACHE_GRADIENT_PROBES: bool = false,
    const DIAGNOSTICS: bool = false,
    const EARLY_PLANE_REJECTION: bool = true,
> {
    stats: Option<&'a mut crate::contracts::RoofWorkCounters>,
    failure: u8,
    height_bounds: Option<[f64; 2]>,
    at: &'a dyn Fn(DVec2) -> f64,
    interior_error: f64,
    boundary_error: f64,
    // Exact coordinate bits are the key; this map is never iterated. A numeric
    // hasher changes lookup cost only, including distinct signed-zero samples.
    cache: rustc_hash::FxHashMap<(u64, u64), f64>,
}
impl<const CACHE_GRADIENT_PROBES: bool, const DIAGNOSTICS: bool, const EARLY_PLANE_REJECTION: bool>
    RoofSampler<'_, CACHE_GRADIENT_PROBES, DIAGNOSTICS, EARLY_PLANE_REJECTION>
{
    fn height(&mut self, point: DVec2) -> Option<f64> {
        let value = *self
            .cache
            .entry((point.x.to_bits(), point.y.to_bits()))
            .or_insert_with(|| (self.at)(point));
        value.is_finite().then_some(value)
    }
    fn gradient_height(&mut self, point: DVec2) -> Option<f64> {
        if CACHE_GRADIENT_PROBES {
            return self.height(point);
        }
        // Gradient probes belong to this one attempted plane, not the shared
        // subdivision grid. The pure analytic field is cheaper than retaining
        // their mostly unique coordinates in the whole-roof sample cache.
        let value = (self.at)(point);
        value.is_finite().then_some(value)
    }
    fn planar_sections(
        &mut self,
        points: [DVec2; 3],
        known_heights: [f64; 7],
    ) -> Option<Vec<RoofTriangle>> {
        let mut reason = 0;
        let result = self.planar_sections_inner(points, known_heights, &mut reason);
        if DIAGNOSTICS {
            let stats = self.stats.as_deref_mut().unwrap();
            stats.plane_attempts += 1;
            if result.is_some() {
                stats.plane_successes += 1;
            } else {
                stats.plane_rejections[reason] += 1;
            }
        }
        result
    }
    fn planar_sections_inner(
        &mut self,
        points: [DVec2; 3],
        known_heights: [f64; 7],
        reason: &mut usize,
    ) -> Option<Vec<RoofTriangle>> {
        let origin = (points[0] + points[1] + points[2]) / 3.0;
        let a = (points[1] - points[0]) * 0.02;
        let b = (points[2] - points[0]) * 0.02;
        let determinant = a.perp_dot(b);
        if determinant.abs() < 1.0e-12 {
            if DIAGNOSTICS {
                *reason = 1;
            }
            return None;
        }
        let plane_roundoff = self.height_bounds.and_then(|bounds| {
            roof_plane_roundoff_bound(points, known_heights, bounds, a, b, determinant)
        });
        if plane_roundoff
            .is_some_and(|roundoff| roof_sample_concavity_deficit(known_heights, roundoff))
        {
            if DIAGNOSTICS {
                *reason = 4;
                self.stats
                    .as_deref_mut()
                    .unwrap()
                    .plane_concavity_rejections += 1;
            }
            return None;
        }
        let mut planes = [RoofPlane {
            origin: DVec2::ZERO,
            gradient: DVec2::ZERO,
            height: 0.0,
        }; 4];
        let mut plane_count = 0;
        let early_rejection = EARLY_PLANE_REJECTION && plane_roundoff.is_some();
        let known_points = [
            points[0],
            points[1],
            points[2],
            (points[0] + points[1]) * 0.5,
            (points[1] + points[2]) * 0.5,
            (points[2] + points[0]) * 0.5,
            origin,
        ];
        let mut predicted_heights = [f64::INFINITY; 7];
        for weights in [
            [0.82, 0.09, 0.09],
            [0.09, 0.82, 0.09],
            [0.09, 0.09, 0.82],
            [0.48, 0.48, 0.04],
            [0.48, 0.04, 0.48],
            [0.04, 0.48, 0.48],
            [1.0 / 3.0; 3],
        ] {
            let p = points[0] * weights[0] + points[1] * weights[1] + points[2] * weights[2];
            let h = self.gradient_height(p)?;
            let da = self.gradient_height(p + a)? - h;
            let db = self.gradient_height(p + b)? - h;
            let gradient = DVec2::new(
                (da * b.y - db * a.y) / determinant,
                (a.x * db - b.x * da) / determinant,
            );
            let plane = RoofPlane {
                origin,
                gradient,
                height: h + gradient.dot(origin - p),
            };
            if !planes[..plane_count].iter().any(|other| {
                other.gradient.distance(gradient) < 1.0e-3
                    && (other.height - plane.height).abs() < 0.001
            }) {
                // The candidate set only grows; a fifth distinct plane can
                // never produce the supported two-to-four-plane section.
                if plane_count == planes.len() {
                    if DIAGNOSTICS {
                        *reason = 2;
                    }
                    return None;
                }
                planes[plane_count] = plane;
                plane_count += 1;
                if early_rejection {
                    // Adding finite planes can only lower their envelope. A
                    // retained candidate already more than 1mm below a known
                    // height can never recover. The callback-range bound also
                    // certifies finite future gradients/evaluations; unbounded
                    // callbacks keep the original total-order/NaN behavior.
                    for ((point, height), predicted) in known_points
                        .iter()
                        .zip(known_heights)
                        .zip(&mut predicted_heights)
                    {
                        let value = plane.at(*point);
                        if value.total_cmp(predicted).is_lt() {
                            *predicted = value;
                        }
                        if height - *predicted > 0.001 {
                            if DIAGNOSTICS {
                                *reason = 4;
                            }
                            return None;
                        }
                    }
                }
            }
        }
        if plane_count < 2 {
            if DIAGNOSTICS {
                *reason = 3;
            }
            return None;
        }
        let planes = &planes[..plane_count];
        for (index, (p, height)) in known_points.into_iter().zip(known_heights).enumerate() {
            let predicted = if early_rejection {
                predicted_heights[index]
            } else {
                planes
                    .iter()
                    .map(|plane| plane.at(p))
                    .min_by(f64::total_cmp)?
            };
            if (height - predicted).abs() > 0.001 {
                if DIAGNOSTICS {
                    *reason = 4;
                }
                return None;
            }
        }
        let mut result = Vec::new();
        for (i, plane) in planes.iter().enumerate() {
            let mut polygon = points.to_vec();
            for (j, other) in planes.iter().enumerate() {
                if i != j {
                    polygon = clip_roof_plane(&polygon, *plane, *other);
                }
            }
            if polygon.len() < 3 {
                continue;
            }
            let center = polygon.iter().copied().sum::<DVec2>() / polygon.len() as f64;
            if (self.height(center)? - plane.at(center)).abs() > 0.001 {
                if DIAGNOSTICS {
                    *reason = 5;
                }
                return None;
            }
            for i in 0..polygon.len() {
                let midpoint = (polygon[i] + polygon[(i + 1) % polygon.len()]) * 0.5;
                if (self.height(midpoint)? - plane.at(midpoint)).abs() > 0.001 {
                    if DIAGNOSTICS {
                        *reason = 5;
                    }
                    return None;
                }
            }
            for i in 1..polygon.len().saturating_sub(1) {
                let p = [polygon[0], polygon[i], polygon[i + 1]];
                if (p[1] - p[0]).perp_dot(p[2] - p[0]).abs() < 1.0e-12 {
                    continue;
                }
                let heights = [self.height(p[0])?, self.height(p[1])?, self.height(p[2])?];
                if (self.height((p[0] + p[1] + p[2]) / 3.0)? - heights.iter().sum::<f64>() / 3.0)
                    .abs()
                    > ROOF_PLANE_ERROR_M
                    || (0..3).any(|j| {
                        let next = (j + 1) % 3;
                        self.height((p[j] + p[next]) * 0.5).is_none_or(|h| {
                            (h - (heights[j] + heights[next]) * 0.5).abs() > ROOF_PLANE_ERROR_M
                        })
                    })
                {
                    if DIAGNOSTICS {
                        *reason = 6;
                    }
                    return None;
                }
                result.push(RoofTriangle { points: p, heights });
            }
        }
        (!result.is_empty()).then_some(result)
    }

    fn triangle(
        &mut self,
        points: [DVec2; 3],
        refine: bool,
        depth: u8,
        output: &mut Vec<RoofTriangle>,
        boundary_edges: u8,
    ) -> Option<()> {
        self.triangle_sampled(points, refine, depth, output, None, boundary_edges)
    }

    fn triangle_sampled(
        &mut self,
        points: [DVec2; 3],
        refine: bool,
        depth: u8,
        output: &mut Vec<RoofTriangle>,
        known_heights: Option<[f64; 3]>,
        boundary_edges: u8,
    ) -> Option<()> {
        if DIAGNOSTICS {
            self.failure = 0;
            if refine {
                self.stats.as_deref_mut().unwrap().refinement_visits[usize::from(depth).min(9)] +=
                    1;
            }
        }
        if (points[1] - points[0])
            .perp_dot(points[2] - points[0])
            .abs()
            < 1.0e-12
        {
            return Some(());
        }
        if output.len() >= MAX_ROOF_TRIANGLES {
            if DIAGNOSTICS {
                self.failure = 2;
            }
            return None;
        }
        let heights = match known_heights {
            Some(heights) => heights,
            None => [
                self.height(points[0])?,
                self.height(points[1])?,
                self.height(points[2])?,
            ],
        };
        if refine {
            let midpoints = [
                (points[0] + points[1]) * 0.5,
                (points[1] + points[2]) * 0.5,
                (points[2] + points[0]) * 0.5,
            ];
            let middle = (points[0] + points[1] + points[2]) / 3.0;
            let middle_height = self.height(middle)?;
            let mut error = (middle_height - heights.iter().sum::<f64>() / 3.0).abs();
            let midpoint_heights = [
                self.height(midpoints[0])?,
                self.height(midpoints[1])?,
                self.height(midpoints[2])?,
            ];
            let mut boundary_error = 0.0_f64;
            for i in 0..3 {
                let midpoint_error =
                    (midpoint_heights[i] - (heights[i] + heights[(i + 1) % 3]) * 0.5).abs();
                error = error.max(midpoint_error);
                if boundary_edges & (1 << i) != 0 {
                    boundary_error = boundary_error.max(midpoint_error);
                }
            }
            if error > self.interior_error || boundary_error > self.boundary_error {
                if let Some(sections) = self.planar_sections(
                    points,
                    [
                        heights[0],
                        heights[1],
                        heights[2],
                        midpoint_heights[0],
                        midpoint_heights[1],
                        midpoint_heights[2],
                        middle_height,
                    ],
                ) {
                    if output.len() + sections.len() > MAX_ROOF_TRIANGLES {
                        if DIAGNOSTICS {
                            self.failure = 2;
                        }
                        return None;
                    }
                    output.extend(sections);
                    return Some(());
                }
                if depth >= MAX_ROOF_REFINEMENT {
                    if DIAGNOSTICS {
                        self.failure = 1;
                    }
                    return None;
                }
                for (child, heights, child_boundary) in [
                    (
                        [points[0], midpoints[0], midpoints[2]],
                        [heights[0], midpoint_heights[0], midpoint_heights[2]],
                        boundary_edges & 0b101,
                    ),
                    (
                        [midpoints[0], points[1], midpoints[1]],
                        [midpoint_heights[0], heights[1], midpoint_heights[1]],
                        boundary_edges & 0b011,
                    ),
                    (
                        [midpoints[2], midpoints[1], points[2]],
                        [midpoint_heights[2], midpoint_heights[1], heights[2]],
                        boundary_edges & 0b110,
                    ),
                    (
                        [midpoints[0], midpoints[1], midpoints[2]],
                        midpoint_heights,
                        0,
                    ),
                ] {
                    self.triangle_sampled(
                        child,
                        true,
                        depth + 1,
                        output,
                        Some(heights),
                        child_boundary,
                    )?;
                }
                return Some(());
            }
        }
        output.push(RoofTriangle { points, heights });
        Some(())
    }
}

struct RoofBoundarySample {
    point: DVec2,
    height: f64,
    first_visit: usize,
    last_visit: usize,
}

struct RoofBoundaryIndex {
    samples: Vec<RoofBoundarySample>,
    by_axis: [Vec<usize>; 2],
    next_visit: Vec<usize>,
}

impl RoofBoundaryIndex {
    fn new(triangles: &[RoofTriangle]) -> Self {
        Self::with_hasher::<rustc_hash::FxBuildHasher>(triangles)
    }

    #[cfg(test)]
    fn new_std(triangles: &[RoofTriangle]) -> Self {
        Self::with_hasher::<std::collections::hash_map::RandomState>(triangles)
    }

    fn with_hasher<H: std::hash::BuildHasher + Default>(triangles: &[RoofTriangle]) -> Self {
        let mut samples: Vec<RoofBoundarySample> = Vec::new();
        // Only coordinate lookup uses the map; insertion order owns sample IDs
        // and visit ties. Never derive boundary output from map iteration.
        let mut seen = HashMap::with_hasher(H::default());
        let mut next_visit = Vec::with_capacity(triangles.len() * 3);
        for triangle in triangles {
            for i in 0..3 {
                let point = triangle.points[i];
                let visit = next_visit.len();
                next_visit.push(usize::MAX);
                let key = (point.x.to_bits(), point.y.to_bits());
                if let Some(&index) = seen.get(&key) {
                    let sample: &mut RoofBoundarySample = &mut samples[index];
                    next_visit[sample.last_visit] = visit;
                    sample.last_visit = visit;
                } else {
                    seen.insert(key, samples.len());
                    samples.push(RoofBoundarySample {
                        point,
                        height: triangle.heights[i],
                        first_visit: visit,
                        last_visit: visit,
                    });
                }
            }
        }
        let by_axis = std::array::from_fn(|axis| {
            let mut indices: Vec<_> = (0..samples.len()).collect();
            indices.sort_by(|&a, &b| {
                samples[a].point[axis]
                    .total_cmp(&samples[b].point[axis])
                    .then(a.cmp(&b))
            });
            indices
        });
        Self {
            samples,
            by_axis,
            next_visit,
        }
    }

    fn candidates(&self, edge: [DVec2; 2]) -> impl Iterator<Item = usize> + '_ {
        // Include the reconstructed endpoint used by the exact predicate and
        // round outward so this index cannot narrow its 1e-5 metre tolerance.
        let end = edge[0] + (edge[1] - edge[0]);
        let lo = edge[0].min(edge[1]).min(end) - DVec2::splat(1.0e-5);
        let hi = edge[0].max(edge[1]).max(end) + DVec2::splat(1.0e-5);
        let bounds: [[f64; 2]; 2] =
            std::array::from_fn(|axis| [lo[axis].next_down(), hi[axis].next_up()]);
        let ranges: [_; 2] = std::array::from_fn(|axis| {
            let indices = &self.by_axis[axis];
            indices.partition_point(|&i| self.samples[i].point[axis] < bounds[axis][0])
                ..indices.partition_point(|&i| self.samples[i].point[axis] <= bounds[axis][1])
        });
        let axis = usize::from(ranges[1].len() < ranges[0].len());
        self.by_axis[axis][ranges[axis].clone()]
            .iter()
            .copied()
            .filter(move |&i| {
                let point = self.samples[i].point;
                (0..2).all(|axis| point[axis] >= bounds[axis][0] && point[axis] <= bounds[axis][1])
            })
    }

    fn boundary(&self, boundary: &[[DVec2; 2]]) -> Vec<RoofBoundary> {
        let mut result = Vec::new();
        for &edge in boundary {
            let direction = edge[1] - edge[0];
            let length2 = direction.length_squared();
            let mut cuts = Vec::new();
            for index in self.candidates(edge) {
                let sample = &self.samples[index];
                let t = ((sample.point - edge[0]).dot(direction) / length2).clamp(0.0, 1.0);
                if sample.point.distance_squared(edge[0] + direction * t) < 1.0e-10 {
                    // Shared points have one cached height. Keep their original
                    // visits for stable ties between distinct nearby points;
                    // the later tolerance-based dedup must see the same order.
                    let mut visit = sample.first_visit;
                    while visit != usize::MAX {
                        cuts.push((t, visit, index));
                        visit = self.next_visit[visit];
                    }
                }
            }
            cuts.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            cuts.dedup_by(|a, b| {
                self.samples[a.2]
                    .point
                    .distance_squared(self.samples[b.2].point)
                    < 1.0e-12
            });
            for pair in cuts.windows(2) {
                let a = &self.samples[pair[0].2];
                let b = &self.samples[pair[1].2];
                if a.point.distance_squared(b.point) > 1.0e-12 {
                    result.push(RoofBoundary {
                        points: [a.point, b.point],
                        heights: [a.height, b.height],
                    });
                }
            }
        }
        result
    }
}

#[cfg(test)]
fn roof_geometry(
    cells: impl IntoIterator<Item = [DVec2; 4]>,
    boundary: &[[DVec2; 2]],
    at: &dyn Fn(DVec2) -> f64,
    refine: bool,
) -> Option<RoofGeometry> {
    roof_geometry_with_error(cells, boundary, at, refine, ROOF_PLANE_ERROR_M)
}

#[cfg(test)]
fn roof_geometry_with_error(
    cells: impl IntoIterator<Item = [DVec2; 4]>,
    boundary: &[[DVec2; 2]],
    at: &dyn Fn(DVec2) -> f64,
    refine: bool,
    interior_error: f64,
) -> Option<RoofGeometry> {
    let mut sampler = RoofSampler::<false> {
        stats: None,
        failure: 0,
        height_bounds: None,
        at,
        interior_error,
        boundary_error: ROOF_PLANE_ERROR_M,
        cache: rustc_hash::FxHashMap::default(),
    };
    roof_geometry_sampled(cells.into_iter().collect(), boundary, &mut sampler, refine)
}

fn coarse_roof_cap(
    domain: &Footprint<'_>,
    boundary: &[[DVec2; 2]],
    top: f64,
    stats: Option<&mut crate::contracts::RoofWorkCounters>,
) -> Option<RoofGeometry> {
    roof_geometry_profiled(
        domain.coarse_roof_cells()?.iter().copied(),
        boundary,
        &|_| top,
        false,
        ROOF_PLANE_ERROR_M,
        Some([top, top]),
        stats,
    )
}

fn distant_roof_cap(
    domain: &Footprint<'_>,
    boundary: &[[DVec2; 2]],
    top: f64,
    stats: Option<&mut crate::contracts::RoofWorkCounters>,
) -> Option<RoofGeometry> {
    let cells = domain.coalesced_cells(4096.0)?;
    let mut triangles = Vec::with_capacity(cells.len() * 2);
    for cell in &cells {
        for points in [[cell[0], cell[1], cell[2]], [cell[0], cell[2], cell[3]]] {
            if (points[1] - points[0])
                .perp_dot(points[2] - points[0])
                .abs()
                > 1.0e-12
            {
                triangles.push(RoofTriangle {
                    points,
                    heights: [top; 3],
                });
            }
        }
    }
    if let Some(stats) = stats {
        stats.initial_cells = cells.len() as u64;
    }
    Some(RoofGeometry {
        triangles,
        boundary: boundary
            .iter()
            .map(|&points| RoofBoundary {
                points,
                heights: [top; 2],
            })
            .collect(),
    })
}

fn seal_coarse_roof_cuts(
    frame: &ChunkFrame,
    mesh: &mut MeshBuffers,
    first_roof_index: usize,
    eave_m: f64,
    appearance: SurfaceAppearance,
) {
    // Mixed-resolution neighbours may resolve different portions of a roof.
    // Close only owner cuts down to the shared eave; source outlines and
    // courtyards continue to use their actual walls and roof closures.
    let bottom = |point: DVec3| {
        let (lat, lon, height) = ellipsoid::ecef_to_geodetic(frame.local_to_ecef(point));
        let foot = frame.ecef_to_local(ellipsoid::geodetic_to_ecef(
            lat,
            lon,
            height.min(eave_m) - 0.05,
        ));
        DVec3::new(point.x, foot.y, point.z)
    };
    let mut seams = Vec::new();
    for triangle in mesh.indices[first_roof_index..].chunks_exact(3) {
        let points = [triangle[0], triangle[1], triangle[2]]
            .map(|index| DVec3::from_array(mesh.vertices[index as usize].pos.map(f64::from)));
        if (points[1] - points[0])
            .cross(points[2] - points[0])
            .length_squared()
            < 1.0e-16
        {
            continue;
        }
        for edge in 0..3 {
            let a = points[edge];
            let b = points[(edge + 1) % 3];
            if [0, 2].into_iter().any(|axis| {
                [0.0, N as f64]
                    .into_iter()
                    .any(|limit| a[axis] == limit && b[axis] == limit)
            }) {
                seams.push([a, bottom(a), bottom(b), b]);
            }
        }
    }
    for seam in seams {
        emit_quad(mesh, seam, appearance, false);
    }
}

fn roof_geometry_profiled(
    cells: impl IntoIterator<Item = [DVec2; 4]>,
    boundary: &[[DVec2; 2]],
    at: &dyn Fn(DVec2) -> f64,
    refine: bool,
    interior_error: f64,
    height_bounds: Option<[f64; 2]>,
    stats: Option<&mut crate::contracts::RoofWorkCounters>,
) -> Option<RoofGeometry> {
    if let Some(stats) = stats {
        let mut sampler = RoofSampler::<false, true> {
            at,
            interior_error,
            boundary_error: ROOF_PLANE_ERROR_M,
            cache: rustc_hash::FxHashMap::default(),
            stats: Some(stats),
            failure: 0,
            height_bounds,
        };
        roof_geometry_sampled(cells.into_iter().collect(), boundary, &mut sampler, refine)
    } else {
        let mut sampler = RoofSampler::<false> {
            stats: None,
            failure: 0,
            height_bounds,
            at,
            interior_error,
            boundary_error: ROOF_PLANE_ERROR_M,
            cache: rustc_hash::FxHashMap::default(),
        };
        roof_geometry_sampled(cells.into_iter().collect(), boundary, &mut sampler, refine)
    }
}

fn roof_geometry_sampled<
    const CACHE_GRADIENT_PROBES: bool,
    const DIAGNOSTICS: bool,
    const EARLY_PLANE_REJECTION: bool,
>(
    cells: Vec<[DVec2; 4]>,
    boundary: &[[DVec2; 2]],
    sampler: &mut RoofSampler<'_, CACHE_GRADIENT_PROBES, DIAGNOSTICS, EARLY_PLANE_REJECTION>,
    refine: bool,
) -> Option<RoofGeometry> {
    if DIAGNOSTICS {
        let stats = sampler.stats.as_deref_mut().unwrap();
        if stats.initial_cells == 0 {
            stats.initial_cells = cells.len() as u64;
        }
    }
    let mut triangles = Vec::new();
    let snap = |point: DVec2| {
        for [a, b] in boundary {
            let t = ((point - *a).dot(*b - *a) / a.distance_squared(*b)).clamp(0.0, 1.0);
            let closest = a.lerp(*b, t);
            if point.distance_squared(closest) < 1.0e-8 {
                return (closest, true);
            }
        }
        (point, false)
    };
    for cell in &cells {
        let snapped = cell.map(snap);
        let cell = snapped.map(|(point, _)| point);
        let center = cell.iter().copied().sum::<DVec2>() * 0.25;
        for edge in 0..4 {
            if sampler
                .triangle(
                    [cell[edge], cell[(edge + 1) % 4], center],
                    refine,
                    0,
                    &mut triangles,
                    // The fan's outer side may lie on the source boundary.
                    // Different boundary edges at its endpoints conservatively
                    // retain the fine tolerance as well.
                    u8::from(snapped[edge].1 && snapped[(edge + 1) % 4].1),
                )
                .is_none()
            {
                // Refinement is optional: the whole-feature base grid keeps
                // the same height field and shared boundary within budget.
                // Invalid samples still fail the unrefined pass.
                return if refine {
                    if DIAGNOSTICS {
                        let stats = sampler.stats.as_deref_mut().unwrap();
                        stats.discarded_triangles += triangles.len() as u64;
                        match sampler.failure {
                            1 => stats.fallback_depth += 1,
                            2 => stats.fallback_budget += 1,
                            _ => stats.fallback_invalid += 1,
                        }
                    }
                    // The unrefined mesh uses the same pure height field and
                    // coordinate bits. Its already-sampled points remain valid.
                    roof_geometry_sampled(cells, boundary, sampler, false)
                } else {
                    None
                };
            }
        }
    }
    // Roofs, gable closures and eaves consume the same boundary polyline.
    // Re-sampling that boundary on a different grid opens gaps at crests.
    let sampled_boundary = RoofBoundaryIndex::new(&triangles).boundary(boundary);
    Some(RoofGeometry {
        triangles,
        boundary: sampled_boundary,
    })
}

// Only the caller's envelope span receives the finish. Matching every material
// in a finished building would also paint timber trim, shutters, and railings.
fn apply_finish_variant(vertices: &mut [SurfaceVertex], variant: u8) {
    debug_assert!(variant < 4);
    if variant == 0 {
        return;
    }
    for vertex in vertices {
        vertex.face_ao = (vertex.face_ao & !(3 << 13)) | (u32::from(variant & 3) << 13);
    }
}

fn emit_roof(
    projection: &Projection<'_>,
    mesh: &mut MeshBuffers,
    roof: &RoofGeometry,
    voxel: SurfaceAppearance,
) {
    for triangle in &roof.triangles {
        emit_triangle(
            mesh,
            std::array::from_fn(|i| projection.point(triangle.points[i], triangle.heights[i])),
            voxel,
            true,
        );
    }
}

// Collision retains mapped ownership, courtyards and raised-part clearance,
// while a flat cap avoids duplicating the refined visual roof tessellation.
fn emit_collision_prism(
    projection: &Projection<'_>,
    mesh: &mut MeshBuffers,
    footprint: &Footprint<'_>,
    boundary: Option<&[[DVec2; 2]]>,
    base: f64,
    top: f64,
) -> Option<()> {
    let cells = footprint.coarse_roof_cells()?;
    let appearance = SurfaceAppearance {
        material: crate::material::WALL,
        tint: [0; 3],
    };
    for &cell in cells.iter() {
        emit_quad(
            mesh,
            cell.map(|p| projection.point(p, top)),
            appearance,
            true,
        );
        let first = mesh.indices.len();
        emit_quad(
            mesh,
            cell.map(|p| projection.point(p, base)),
            appearance,
            true,
        );
        for triangle in mesh.indices[first..].chunks_exact_mut(3) {
            triangle.swap(1, 2);
        }
    }
    let boundary = boundary
        .map(std::borrow::Cow::Borrowed)
        .unwrap_or_else(|| std::borrow::Cow::Owned(footprint.boundary()));
    for &[a, b] in boundary.iter() {
        emit_quad(
            mesh,
            [
                projection.point(a, base),
                projection.point(b, base),
                projection.point(b, top),
                projection.point(a, top),
            ],
            appearance,
            false,
        );
    }
    Some(())
}

fn emit_roof_closure(
    projection: &Projection<'_>,
    mesh: &mut MeshBuffers,
    edge: &RoofBoundary,
    eave: f64,
    voxel: SurfaceAppearance,
) {
    emit_quad(
        mesh,
        [
            projection.point(edge.points[0], eave),
            projection.point(edge.points[1], eave),
            projection.point(edge.points[1], edge.heights[1]),
            projection.point(edge.points[0], edge.heights[0]),
        ],
        voxel,
        false,
    );
}

fn emit_eyebrow_dormers(
    projection: &Projection<'_>,
    details: &mut MeshBuffers,
    roof: &RoofField,
    domain: &Footprint<'_>,
    source: &Building,
    stamp: &BuildingStamp,
    covering: SurfaceAppearance,
    resolution: FacadeResolution,
) {
    if !expressed_style(stamp).is_some_and(|p| p.roofline == Roofline::EyebrowDormers)
        || roof.reentrant
        || roof.top - roof.eave < 1.0
        || !resolution.resolves(0.4)
    {
        return;
    }
    let Some(ring) = source.rings.first() else {
        return;
    };
    let mut count = 0;
    for i in 0..ring.len() {
        let a = DVec2::from_array(ring[i].map(f64::from));
        let b = DVec2::from_array(ring[(i + 1) % ring.len()].map(f64::from));
        if a.distance(b) < 5.0 {
            continue;
        }
        let mut along = (b - a).normalize();
        if along.dot(roof.direction).abs() < 0.8 {
            continue;
        }
        let mut inward = DVec2::new(-along.y, along.x);
        if !domain.contains((a + b) * 0.5 + inward * 0.1) {
            inward = -inward;
            along = -along;
        }
        for fraction in [0.35, 0.65] {
            if count == 4 || details.vertices.len() + 96 > MAX_FACADE_DETAIL_VERTICES {
                return;
            }
            let center = a.lerp(b, fraction) + inward * 1.0;
            let back = center + inward * 0.9;
            if [
                center - along * 0.7,
                center + along * 0.7,
                back - along * 0.7,
                back + along * 0.7,
            ]
            .iter()
            .any(|&p| !domain.contains(p))
            {
                continue;
            }
            let floor = roof.height(center) + 0.025;
            if floor + 0.45 >= roof.top {
                continue;
            }
            let slit = SurfaceAppearance {
                material: crate::material::WINDOW,
                tint: [45, 49, 43],
            };
            // The front lens and short curved hood share source-stable positions.
            // They remain optional visual detail and never add collision or height.
            for segment in 0..6 {
                let x0 = -0.7 + segment as f64 * 1.4 / 6.0;
                let x1 = -0.7 + (segment + 1) as f64 * 1.4 / 6.0;
                let brow = |x: f64| 0.4 * (1.0 - (x / 0.7).powi(2)).max(0.0);
                let p0 = center + along * x0;
                let p1 = center + along * x1;
                emit_quad(
                    details,
                    [
                        projection.point(p0, floor),
                        projection.point(p1, floor),
                        projection.point(p1, floor + brow(x1)),
                        projection.point(p0, floor + brow(x0)),
                    ],
                    slit,
                    false,
                );
                let first = details.vertices.len();
                let q0 = back + along * x0;
                let q1 = back + along * x1;
                emit_quad(
                    details,
                    [
                        projection.point(p0, floor + brow(x0)),
                        projection.point(p1, floor + brow(x1)),
                        projection.point(q1, (roof.height(q1) + 0.015).min(roof.top)),
                        projection.point(q0, (roof.height(q0) + 0.015).min(roof.top)),
                    ],
                    covering,
                    false,
                );
                apply_finish_variant(&mut details.vertices[first..], stamp.design.roof_variant);
            }
            count += 1;
        }
    }
}

fn emit_gable_crown(
    projection: &Projection<'_>,
    mesh: &mut MeshBuffers,
    edge: &RoofBoundary,
    roof: &RoofField,
    source: &Rings,
    domain: &Footprint<'_>,
    stamp: &BuildingStamp,
    wall: SurfaceAppearance,
    resolution: FacadeResolution,
) {
    let Some(style) = expressed_style(stamp) else {
        return;
    };
    if !matches!(style.roofline, Roofline::FrontGable | Roofline::ShapedGable)
        || roof.top - roof.eave < 0.4
        || roof.reentrant
    {
        return;
    }
    let length = edge.points[0].distance(edge.points[1]);
    let direction = (edge.points[1] - edge.points[0]) / length;
    if direction.dot(roof.direction).abs() > 0.35
        || edge_facade(source, (edge.points[0] + edge.points[1]) * 0.5).edge_distance_m > 0.01
    {
        return;
    }
    let inward = DVec2::new(-direction.y, direction.x);
    if !domain.contains((edge.points[0] + edge.points[1]) * 0.5 + inward * 0.06) {
        return;
    }
    let shaped = style.roofline == Roofline::ShapedGable;
    let stepped = shaped && stamp.seed & 1 == 0;
    let [lo, hi] = roof.bounds;
    let q_at = |p: DVec2| ((roof.rotate(p).y - lo.y) / (hi.y - lo.y)).clamp(0.0, 1.0);
    let qa = q_at(edge.points[0]);
    let qb = q_at(edge.points[1]);
    let mut breaks = vec![0.0, 1.0];
    if (qb - qa).abs() > 1.0e-8 {
        for i in 1..12 {
            let t = (i as f64 / 12.0 - qa) / (qb - qa);
            if t > 1.0e-7 && t < 1.0 - 1.0e-7 {
                breaks.push(t);
            }
        }
    }
    breaks.sort_by(f64::total_cmp);
    let shape_height = |q: f64| {
        let triangle = (1.0 - (q * 2.0 - 1.0).abs()).max(0.0);
        let factor = if stepped {
            (triangle * 6.0).ceil() / 6.0
        } else if shaped {
            triangle.sqrt()
        } else {
            triangle
        };
        roof.eave + (roof.top - roof.eave) * factor
    };
    let trim = facade_trim(stamp);
    for pair in breaks.windows(2) {
        let a = edge.points[0].lerp(edge.points[1], pair[0]);
        let b = edge.points[0].lerp(edge.points[1], pair[1]);
        let heights = if stepped {
            let height = shape_height(q_at((a + b) * 0.5))
                .max(roof.height(a))
                .max(roof.height(b))
                .min(roof.top);
            [height; 2]
        } else {
            [a, b].map(|p| shape_height(q_at(p)).max(roof.height(p)).min(roof.top))
        };
        let point = |p, h| projection.point(p, h);
        let mut ia = a + inward * 0.12;
        let mut ib = b + inward * 0.12;
        for _ in 0..8 {
            let center = (a + b + ia + ib) * 0.25;
            if [ia, ib, (ia + ib) * 0.5]
                .into_iter()
                .all(|p| domain.contains(p.lerp(center, 1.0e-4)))
            {
                break;
            }
            ia = a.lerp(ia, 0.5);
            ib = b.lerp(ib, 0.5);
        }
        emit_quad(
            mesh,
            [
                point(a, roof.eave),
                point(b, roof.eave),
                point(b, heights[1]),
                point(a, heights[0]),
            ],
            wall,
            false,
        );
        emit_quad(
            mesh,
            [
                point(ib, roof.height(ib)),
                point(ia, roof.height(ia)),
                point(ia, heights[0]),
                point(ib, heights[1]),
            ],
            wall,
            false,
        );
        emit_quad(
            mesh,
            [
                point(a, heights[0]),
                point(b, heights[1]),
                point(ib, heights[1]),
                point(ia, heights[0]),
            ],
            trim,
            true,
        );
        for (outer, inner, height) in [(a, ia, heights[0]), (b, ib, heights[1])] {
            emit_quad(
                mesh,
                [
                    point(outer, roof.height(outer)),
                    point(inner, roof.height(inner)),
                    point(inner, height),
                    point(outer, height),
                ],
                wall,
                false,
            );
        }
        if resolution.resolves(0.12) {
            emit_quad(
                mesh,
                [
                    point(a - inward * 0.012, (heights[0] - 0.12).max(roof.eave)),
                    point(b - inward * 0.012, (heights[1] - 0.12).max(roof.eave)),
                    point(b - inward * 0.012, heights[1]),
                    point(a - inward * 0.012, heights[0]),
                ],
                trim,
                false,
            );
        }
    }
}

#[derive(Clone, Copy)]
struct FlatRoof {
    deck: f64,
    top: f64,
    thickness: f64,
    stepped: bool,
}

impl FlatRoof {
    fn with_resolution(self, pixel_m: f64) -> Self {
        // Use the level's nominal pitch, shared by every source visitor, so
        // adjacent cell frames cannot choose different terrace deck heights.
        // The outer top and source footprint stay fixed; filling an unresolved
        // recess avoids an open rim and keeps the flat-roof eave exclusion.
        if !self.stepped
            && pixel_m.is_finite()
            && pixel_m > 0.0
            && self.top - self.deck < pixel_m
            && self.thickness < pixel_m
        {
            Self {
                deck: self.top,
                thickness: 0.0,
                ..self
            }
        } else {
            self
        }
    }

    fn for_source(source: &Building, stamp: &BuildingStamp, floor: f64, top: f64) -> Option<Self> {
        let (lo, hi) = bounds(&source.rings)?;
        if stamp.lidar
            || !stamp.facade_openings
            || source.kind == BuildingKind::RoofOnly
            || stamp.roof_shape != Some(RoofShape::Flat)
            || top - floor < 2.4
            || (hi - lo).min_element() < 1.0
        {
            return None;
        }
        let stepped =
            expressed_style(stamp).is_some_and(|style| style.roofline == Roofline::SteppedParapet);
        let recess = (if stamp.highrise.is_some() {
            1.6
        } else if stepped {
            0.90
        } else {
            0.45
        } + ((source.seed >> 17) & 3) as f64 * (0.20 / 3.0))
            .min((top - floor) * 0.22);
        Some(Self {
            deck: top - recess,
            top,
            thickness: architecture_facade::production_detail_profile(&stamp.design)
                .coping_m
                .clamp(0.12, 0.18),
            stepped,
        })
    }
}

// Offset only original footprint edges. Adjacent Boolean cut edges have zero
// offset, so rim ends meet the ownership cut without erecting a fence along it.
// The full source domain is joined before chunk clipping; cell borders cannot
// influence either the recess height or the parapet's corner positions.
fn emit_flat_parapets(
    projection: &Projection<'_>,
    mesh: &mut MeshBuffers,
    domain: &Footprint<'_>,
    boundary: &[[DVec2; 2]],
    source: &Rings,
    roof: FlatRoof,
    wall: SurfaceAppearance,
    coping: SurfaceAppearance,
) {
    if roof.thickness <= 0.0 || roof.deck >= roof.top {
        return;
    }
    let crown_top = roof.top;
    let roof = FlatRoof {
        top: if roof.stepped {
            roof.top - (roof.top - roof.deck).min(0.28)
        } else {
            roof.top
        },
        ..roof
    };
    let mut starts: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    let mut ends: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    let key = |p: DVec2| {
        (
            (p.x / GEOMETRY_EPSILON).round() as i64,
            (p.y / GEOMETRY_EPSILON).round() as i64,
        )
    };
    let mut directions = Vec::with_capacity(boundary.len());
    let mut widths = Vec::with_capacity(boundary.len());
    for (i, edge) in boundary.iter().enumerate() {
        starts.entry(key(edge[0])).or_default().push(i);
        ends.entry(key(edge[1])).or_default().push(i);
        let length = edge[0].distance(edge[1]);
        directions.push((edge[1] - edge[0]) / length);
        let source_edge = edge_facade(source, (edge[0] + edge[1]) * 0.5);
        widths.push(
            if f64::from(source_edge.edge_distance_m) <= GEOMETRY_EPSILON * 8.0 {
                roof.thickness.min(length * 0.2)
            } else {
                0.0
            },
        );
    }
    let adjacent = |map: &HashMap<(i64, i64), Vec<usize>>, point: DVec2, endpoint: usize| {
        let (x, y) = key(point);
        for dx in -1..=1 {
            for dy in -1..=1 {
                if let Some(indices) = map.get(&(x + dx, y + dy)) {
                    for &i in indices {
                        if boundary[i][endpoint].distance_squared(point) <= GEOMETRY_EPSILON.powi(2)
                        {
                            return Some(i);
                        }
                    }
                }
            }
        }
        None
    };
    let corner = |point: DVec2, incoming: usize, outgoing: usize| {
        let a = directions[incoming];
        let b = directions[outgoing];
        let na = DVec2::new(-a.y, a.x);
        let nb = DVec2::new(-b.y, b.x);
        let pa = point + na * widths[incoming];
        let pb = point + nb * widths[outgoing];
        let cross = a.perp_dot(b);
        if cross.abs() < 1.0e-8 {
            return (pa + pb) * 0.5;
        }
        let joined = pa + a * ((pb - pa).perp_dot(b) / cross);
        let reach = roof.thickness * 4.0;
        if joined.distance_squared(point) <= reach * reach {
            joined
        } else {
            // Bound very acute mitres instead of projecting spikes beyond the
            // source envelope. The strip validation below handles thin wings.
            point + (joined - point).normalize() * reach
        }
    };
    let inner_wall = SurfaceAppearance {
        tint: wall
            .tint
            .map(|channel| (f32::from(channel) * 0.82).round() as u8),
        ..wall
    };
    for (i, &[a, b]) in boundary.iter().enumerate() {
        if widths[i] == 0.0 {
            continue;
        }
        let inward = DVec2::new(-directions[i].y, directions[i].x);
        let previous = adjacent(&ends, a, 1);
        let next = adjacent(&starts, b, 0);
        let mut ia = previous.map_or(a + inward * widths[i], |j| corner(a, j, i));
        let mut ib = next.map_or(b + inward * widths[i], |j| corner(b, i, j));
        // A narrow or reentrant wing can have less interior room than the
        // nominal coping width. Contract inward, never enlarge its footprint.
        for _ in 0..8 {
            let center = (a + b + ia + ib) * 0.25;
            if [ia, ib, (ia + ib) * 0.5, center]
                .into_iter()
                .all(|p| domain.contains(p.lerp(center, 1.0e-4)))
            {
                break;
            }
            ia = a.lerp(ia, 0.5);
            ib = b.lerp(ib, 0.5);
        }
        let point = |p, height| projection.point(p, height);
        emit_quad(
            mesh,
            [
                point(a, roof.deck),
                point(b, roof.deck),
                point(b, roof.top),
                point(a, roof.top),
            ],
            wall,
            false,
        );
        emit_quad(
            mesh,
            [
                point(ib, roof.deck),
                point(ia, roof.deck),
                point(ia, roof.top),
                point(ib, roof.top),
            ],
            inner_wall,
            false,
        );
        emit_quad(
            mesh,
            [
                point(a, roof.top),
                point(b, roof.top),
                point(ib, roof.top),
                point(ia, roof.top),
            ],
            coping,
            true,
        );
        if previous.is_none_or(|j| widths[j] == 0.0) {
            emit_quad(
                mesh,
                [
                    point(ia, roof.deck),
                    point(a, roof.deck),
                    point(a, roof.top),
                    point(ia, roof.top),
                ],
                inner_wall,
                false,
            );
        }
        if next.is_none_or(|j| widths[j] == 0.0) {
            emit_quad(
                mesh,
                [
                    point(b, roof.deck),
                    point(ib, roof.deck),
                    point(ib, roof.top),
                    point(b, roof.top),
                ],
                inner_wall,
                false,
            );
        }
        if roof.stepped {
            let length = a.distance(b);
            let middle = (a + b) * 0.5;
            let source_edge = edge_facade(source, middle);
            let sign = (edge_facade(source, middle + directions[i] * 0.01).along_m
                - edge_facade(source, middle - directions[i] * 0.01).along_m)
                .signum() as f64;
            if sign == 0.0 {
                continue;
            }
            let start = f64::from(source_edge.along_m) - sign * length * 0.5;
            let mut breaks = vec![0.0, 1.0];
            for q in [0.15, 0.40, 0.60, 0.85] {
                let t = (q * f64::from(source_edge.length_m) - start) / (sign * length);
                if t > 1.0e-7 && t < 1.0 - 1.0e-7 {
                    breaks.push(t);
                }
            }
            breaks.sort_by(f64::total_cmp);
            for pair in breaks.windows(2) {
                let q = (start + sign * length * (pair[0] + pair[1]) * 0.5)
                    / f64::from(source_edge.length_m);
                if !(q < 0.15 || q > 0.85 || (0.40..0.60).contains(&q)) {
                    continue;
                }
                let outside = [a.lerp(b, pair[0]), a.lerp(b, pair[1])];
                let inside = [ia.lerp(ib, pair[0]), ia.lerp(ib, pair[1])];
                emit_quad(
                    mesh,
                    [
                        point(outside[0], roof.top),
                        point(outside[1], roof.top),
                        point(outside[1], crown_top),
                        point(outside[0], crown_top),
                    ],
                    wall,
                    false,
                );
                emit_quad(
                    mesh,
                    [
                        point(inside[1], roof.top),
                        point(inside[0], roof.top),
                        point(inside[0], crown_top),
                        point(inside[1], crown_top),
                    ],
                    inner_wall,
                    false,
                );
                emit_quad(
                    mesh,
                    [
                        point(outside[0], crown_top),
                        point(outside[1], crown_top),
                        point(inside[1], crown_top),
                        point(inside[0], crown_top),
                    ],
                    coping,
                    true,
                );
                for (outer, inner) in outside.into_iter().zip(inside) {
                    emit_quad(
                        mesh,
                        [
                            point(outer, roof.top),
                            point(inner, roof.top),
                            point(inner, crown_top),
                            point(outer, crown_top),
                        ],
                        inner_wall,
                        false,
                    );
                }
            }
        }
    }
}

fn emit_eave(
    projection: &Projection<'_>,
    mesh: &mut MeshBuffers,
    edge: &RoofBoundary,
    pitched: bool,
    voxel: SurfaceAppearance,
    design: &crate::regional_architecture::BuildingDesign,
    maximum_extension: f64,
) {
    let first = mesh.vertices.len();
    let profile = architecture_facade::production_detail_profile(design);
    let direction = (edge.points[1] - edge.points[0]).normalize();
    let outward = DVec2::new(direction.y, -direction.x);
    let extension = if pitched {
        if design.expression >= 96 {
            f64::from(design.style.profile().eave_m)
        } else {
            profile.eave_m
        }
    } else {
        profile.coping_m
    }
    .min(maximum_extension);
    let [a, b] = edge.points;
    let [ha, hb] = edge.heights;
    let oa = a + outward * extension;
    let ob = b + outward * extension;
    emit_quad(
        mesh,
        [
            projection.point(a, ha),
            projection.point(b, hb),
            projection.point(ob, hb),
            projection.point(oa, ha),
        ],
        voxel,
        true,
    );
    let dark = SurfaceAppearance {
        tint: voxel.tint.map(|v| (f32::from(v) * 0.7) as u8),
        ..voxel
    };
    emit_quad(
        mesh,
        [
            projection.point(oa, ha - f64::from(profile.fascia_m)),
            projection.point(ob, hb - f64::from(profile.fascia_m)),
            projection.point(ob, hb),
            projection.point(oa, ha),
        ],
        dark,
        false,
    );
    apply_finish_variant(&mut mesh.vertices[first..], design.roof_variant);
}

fn clip_to_eligible(source: MeshBuffers, eligible: &[u32; N]) -> MeshBuffers {
    clip_to_eligible_recorded(source, eligible, None)
}

fn clip_to_eligible_recorded(
    source: MeshBuffers,
    eligible: &[u32; N],
    mut shadow_walls: Option<&mut Vec<ShadowWall>>,
) -> MeshBuffers {
    if eligible.iter().all(|row| *row == u32::MAX) {
        return source;
    }
    let mut ordinal_map = shadow_walls
        .as_ref()
        .filter(|walls| !walls.is_empty())
        .map(|_| {
            let mut map = Vec::with_capacity(source.indices.len() / 3 + 1);
            map.push(0);
            map
        });
    let mut result = MeshBuffers::default();
    for indices in source.indices.chunks_exact(3) {
        let template = source.vertices[indices[0] as usize];
        let triangle: Vec<_> = indices
            .iter()
            .map(|index| DVec3::from_array(source.vertices[*index as usize].pos.map(f64::from)))
            .collect();
        let lo = triangle.iter().copied().reduce(DVec3::min).unwrap();
        let hi = triangle.iter().copied().reduce(DVec3::max).unwrap();
        let mut polygons = vec![triangle];
        for z in (lo.z.floor().max(0.0) as usize)..=(hi.z.floor().min((N - 1) as f64) as usize) {
            for x in (lo.x.floor().max(0.0) as usize)..=(hi.x.floor().min((N - 1) as f64) as usize)
            {
                if eligible[z] & (1 << x) != 0 {
                    continue;
                }
                let mut surviving = Vec::new();
                for polygon in polygons {
                    let mut inside = polygon;
                    for (axis, limit, greater) in [
                        (0, x as f64, true),
                        (0, (x + 1) as f64, false),
                        (2, z as f64, true),
                        (2, (z + 1) as f64, false),
                    ] {
                        let outside = if inside.iter().all(|p| (p[axis] - limit).abs() < 1.0e-8) {
                            Vec::new()
                        } else {
                            clip_half(&inside, axis, limit, !greater)
                        };
                        if outside.len() >= 3 {
                            surviving.push(outside);
                        }
                        inside = clip_half(&inside, axis, limit, greater);
                        if inside.len() < 3 {
                            break;
                        }
                    }
                }
                polygons = surviving;
            }
        }
        for polygon in polygons {
            let base = result.vertices.len() as u32;
            result
                .vertices
                .extend(polygon.iter().map(|p| SurfaceVertex {
                    pos: p.as_vec3().to_array(),
                    ..template
                }));
            for i in 1..polygon.len().saturating_sub(1) {
                if (polygon[i] - polygon[0])
                    .cross(polygon[i + 1] - polygon[0])
                    .length_squared()
                    > 1.0e-14
                {
                    result
                        .indices
                        .extend([base, base + i as u32, base + i as u32 + 1]);
                }
            }
        }
        if let Some(map) = &mut ordinal_map {
            map.push(result.indices.len());
        }
    }
    if let (Some(walls), Some(map)) = (&mut shadow_walls, ordinal_map) {
        for wall in walls.iter_mut() {
            wall.backing = map[wall.backing.start / 3]..map[wall.backing.end / 3];
            for unit in &mut wall.units {
                *unit = map[unit.start / 3]..map[unit.end / 3];
            }
            wall.units.retain(|unit| !unit.is_empty());
        }
        walls.retain(|wall| !wall.backing.is_empty() && !wall.units.is_empty());
    }
    result
}
fn clip_half(polygon: &[DVec3], axis: usize, limit: f64, greater: bool) -> Vec<DVec3> {
    let mut result = Vec::new();
    for i in 0..polygon.len() {
        let a = polygon[i];
        let b = polygon[(i + 1) % polygon.len()];
        let inside = |p: DVec3| {
            if greater {
                p[axis] >= limit
            } else {
                p[axis] <= limit
            }
        };
        if inside(a) {
            result.push(a);
        }
        if inside(a) != inside(b) {
            let mut p = a.lerp(b, (limit - a[axis]) / (b[axis] - a[axis]));
            p[axis] = limit;
            result.push(p);
        }
    }
    result
}

fn emit_quad(mesh: &mut MeshBuffers, p: [DVec3; 4], voxel: SurfaceAppearance, up: bool) {
    emit_triangle(mesh, [p[0], p[1], p[2]], voxel, up);
    emit_triangle(mesh, [p[0], p[2], p[3]], voxel, up);
}
fn emit_triangle(
    mesh: &mut MeshBuffers,
    mut points: [DVec3; 3],
    voxel: SurfaceAppearance,
    up: bool,
) {
    let mut normal = (points[1] - points[0]).cross(points[2] - points[0]);
    if !normal.is_finite() || normal.length_squared() < 1.0e-16 {
        return;
    }
    if up && normal.y < 0.0 {
        points.swap(1, 2);
        normal = -normal;
    }
    let normal = normal.normalize();
    if points
        .iter()
        .all(|p| p.x >= 0.0 && p.x <= N as f64 && p.z >= 0.0 && p.z <= N as f64)
    {
        emit_clipped_polygon(mesh, &points, normal, voxel);
        return;
    }
    if [0, 2].into_iter().any(|axis| {
        points.iter().all(|p| p[axis] < 0.0) || points.iter().all(|p| p[axis] > N as f64)
    }) {
        return;
    }
    let mut polygon = points.to_vec();
    let mut scratch = Vec::new();
    for (axis, limit, greater) in [
        (0, 0.0, true),
        (0, N as f64, false),
        (2, 0.0, true),
        (2, N as f64, false),
    ] {
        if polygon.len() < 3 {
            return;
        }
        let inside = |p: DVec3| {
            if greater {
                p[axis] >= limit
            } else {
                p[axis] <= limit
            }
        };
        if polygon.iter().copied().all(inside) {
            continue;
        }
        // Most architectural triangles lie wholly inside the cell. Boundary
        // triangles reuse their clipping storage through the four planes.
        std::mem::swap(&mut polygon, &mut scratch);
        polygon.clear();
        for i in 0..scratch.len() {
            let a = scratch[i];
            let b = scratch[(i + 1) % scratch.len()];
            if inside(a) {
                polygon.push(a);
            }
            if inside(a) != inside(b) {
                let mut p = a.lerp(b, (limit - a[axis]) / (b[axis] - a[axis]));
                p[axis] = limit;
                polygon.push(p);
            }
        }
    }
    if polygon.len() < 3 {
        return;
    }
    emit_clipped_polygon(mesh, &polygon, normal, voxel);
}

fn emit_clipped_polygon(
    mesh: &mut MeshBuffers,
    polygon: &[DVec3],
    normal: DVec3,
    voxel: SurfaceAppearance,
) {
    let base = mesh.vertices.len() as u32;
    let face = if normal.y > 0.65 {
        2
    } else if normal.y < -0.65 {
        3
    } else if normal.x.abs() > normal.z.abs() {
        if normal.x > 0.0 { 0 } else { 1 }
    } else if normal.z > 0.0 {
        4
    } else {
        5
    };
    let face_ao = encode_surface_normal(
        face | 3 << 3 | u32::from(voxel.material) << 5,
        normal.as_vec3().to_array(),
    );
    for p in polygon {
        mesh.vertices.push(SurfaceVertex {
            pos: p.as_vec3().to_array(),
            face_ao,
            rgba: [voxel.tint[0], voxel.tint[1], voxel.tint[2], 255],
        });
    }
    for i in 1..polygon.len().saturating_sub(1) {
        mesh.indices
            .extend([base, base + i as u32, base + i as u32 + 1]);
    }
}

const MAX_FOOTPRINT_EDGES: usize = 512;
/// Building envelopes subtract every overlapping mapped part, so large
/// multi-part buildings legitimately exceed the general footprint budget.
/// Rejecting one drops the building and leaves its cell permanently
/// uncacheable. Slab rows remain bounded by `MAX_ROOF_CELLS`.
const MAX_BUILDING_FOOTPRINT_EDGES: usize = 4096;
const MAX_ROOF_CELLS: usize = 8_192;
const GEOMETRY_EPSILON: f64 = 1.0e-7;
const COARSE_FOOTPRINT_CACHE_BYTES: usize = 1024 * 1024;

pub(crate) fn source_polygon_cells(rings: &Rings, max_step_m: f64) -> Option<Vec<[DVec2; 4]>> {
    Footprint::new(vec![rings], Vec::new())?.roof_cells(max_step_m)
}
/// Clip source rings before triangulation, bounding work for regional lakes.
/// Even-odd ring membership retains holes and disconnected clipped components.
pub(crate) fn source_polygon_cells_clipped(
    rings: &Rings,
    min: DVec2,
    max: DVec2,
    max_step: f64,
) -> Option<Vec<[DVec2; 4]>> {
    clipped_source_cells::<false>(rings, min, max, max_step)
}

pub(crate) fn source_polygon_cells_clipped_coalesced(
    rings: &Rings,
    min: DVec2,
    max: DVec2,
    max_step: f64,
) -> Option<Vec<[DVec2; 4]>> {
    clipped_source_cells::<true>(rings, min, max, max_step)
}

fn clipped_source_cells<const COALESCED: bool>(
    rings: &Rings,
    min: DVec2,
    max: DVec2,
    max_step: f64,
) -> Option<Vec<[DVec2; 4]>> {
    if !min.is_finite()
        || !max.is_finite()
        || min.cmpge(max).any()
        || rings.iter().map(Vec::len).sum::<usize>() > 131_072
    {
        return None;
    }
    let mut clipped = Vec::new();
    for ring in rings {
        if ring.len() < 3 || ring.iter().flatten().any(|value| !value.is_finite()) {
            return None;
        }
        let mut points: Vec<_> = ring
            .iter()
            .map(|p| DVec3::new(f64::from(p[0]), 0.0, f64::from(p[1])))
            .collect();
        for (axis, limit, greater) in [
            (0, min.x, true),
            (0, max.x, false),
            (2, min.y, true),
            (2, max.y, false),
        ] {
            points = clip_half(&points, axis, limit, greater);
            if points.len() < 3 {
                break;
            }
        }
        if points.len() >= 3 {
            // A clip contact on a single box side has no two-dimensional
            // domain. Keep malformed interior rings on the failure path.
            if [(0, min.x), (0, max.x), (2, min.y), (2, max.y)]
                .into_iter()
                .any(|(axis, limit)| points.iter().all(|point| point[axis] == limit))
            {
                continue;
            }
            clipped.push(
                points
                    .into_iter()
                    .map(|p| [p.x as f32, p.z as f32])
                    .collect(),
            );
        }
    }
    if clipped.is_empty() {
        return Some(Vec::new());
    }
    let footprint = Footprint::with_limits(vec![&clipped], Vec::new(), 8_192, 32_768)?;
    if COALESCED {
        footprint.coalesced_cells(max_step)
    } else {
        footprint.roof_cells(max_step)
    }
}
pub(crate) fn source_polygon_boundary(rings: &Rings) -> Option<Vec<[DVec2; 2]>> {
    Some(Footprint::new(vec![rings], Vec::new())?.boundary())
}

enum FootprintFailure {
    ShortRing,
    Nonfinite,
    EdgeBudget { required_edges: usize },
    Empty,
}

struct CoarseFootprintCache {
    remaining_bytes: Rc<std::cell::Cell<usize>>,
    cells: std::cell::OnceCell<Option<Box<[[DVec2; 4]]>>>,
}

struct Footprint<'a> {
    required: Vec<&'a Rings>,
    excluded: Vec<&'a Rings>,
    required_queries: Vec<PolygonQuery>,
    excluded_queries: Vec<PolygonQuery>,
    edges: Vec<[DVec2; 2]>,
    arrangement_bounds: [DVec2; 2],
    max_cells: usize,
    coarse_cache: Option<CoarseFootprintCache>,
}

impl<'a> Footprint<'a> {
    fn new(required: Vec<&'a Rings>, excluded: Vec<&'a Rings>) -> Option<Self> {
        Self::with_limits(required, excluded, MAX_FOOTPRINT_EDGES, MAX_ROOF_CELLS)
    }
    fn for_building(required: Vec<&'a Rings>, excluded: Vec<&'a Rings>, level: u8) -> Option<Self> {
        Self::checked_with_limits(
            required,
            excluded,
            MAX_BUILDING_FOOTPRINT_EDGES,
            MAX_ROOF_CELLS,
        )
        .map_err(|failure| {
            let reason = match failure {
                FootprintFailure::ShortRing => 12,
                FootprintFailure::Nonfinite => 13,
                FootprintFailure::EdgeBudget { required_edges } => {
                    crate::contracts::record_building_failure(
                        if required_edges > MAX_BUILDING_FOOTPRINT_EDGES {
                            16
                        } else {
                            17
                        },
                        level,
                    );
                    14
                }
                FootprintFailure::Empty => 15,
            };
            crate::contracts::record_building_failure(reason, level);
        })
        .ok()
    }
    fn with_limits(
        required: Vec<&'a Rings>,
        excluded: Vec<&'a Rings>,
        max_edges: usize,
        max_cells: usize,
    ) -> Option<Self> {
        Self::checked_with_limits(required, excluded, max_edges, max_cells).ok()
    }
    fn checked_with_limits(
        required: Vec<&'a Rings>,
        excluded: Vec<&'a Rings>,
        max_edges: usize,
        max_cells: usize,
    ) -> Result<Self, FootprintFailure> {
        if required.is_empty() {
            return Err(FootprintFailure::Empty);
        }
        let mut edges = Vec::new();
        let mut original_required_edges = 0;
        for (polygon, rings) in required.iter().chain(&excluded).enumerate() {
            for ring in *rings {
                if ring.len() < 3 {
                    return Err(FootprintFailure::ShortRing);
                }
                if ring.iter().flatten().any(|value| !value.is_finite()) {
                    return Err(FootprintFailure::Nonfinite);
                }
                for i in 0..ring.len() {
                    let a = DVec2::from_array(ring[i].map(f64::from));
                    let b = DVec2::from_array(ring[(i + 1) % ring.len()].map(f64::from));
                    if a.distance_squared(b) > GEOMETRY_EPSILON * GEOMETRY_EPSILON {
                        edges.push([a, b]);
                        original_required_edges += usize::from(polygon < required.len());
                    }
                }
            }
        }
        if edges.is_empty() {
            return Err(FootprintFailure::Empty);
        }
        // An intersection cannot extend beyond any required polygon's bounds.
        // Keep full polygons for membership, but restrict the arrangement to
        // original segments that can touch this common domain. An enclosing
        // exclusion can have no local edges and still erase the whole domain.
        let arrangement_bounds = required.iter().fold(
            [DVec2::splat(f64::NEG_INFINITY), DVec2::splat(f64::INFINITY)],
            |[lo, hi], rings| {
                let [required_lo, required_hi] = rings.iter().flatten().fold(
                    [DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY)],
                    |[lo, hi], point| {
                        let point = DVec2::from_array(point.map(f64::from));
                        [lo.min(point), hi.max(point)]
                    },
                );
                [lo.max(required_lo), hi.min(required_hi)]
            },
        );
        let [lo, hi] = arrangement_bounds;
        let disjoint = lo.cmpgt(hi).any();
        let mut seen = rustc_hash::FxHashSet::default();
        let mut visited_edges = 0;
        let mut required_edges = 0;
        edges.retain(|edge| {
            let required = visited_edges < original_required_edges;
            visited_edges += 1;
            if disjoint
                || edge[0].min(edge[1]).cmpgt(hi).any()
                || edge[0].max(edge[1]).cmplt(lo).any()
            {
                return false;
            }
            // No synthetic clipping edges or changed interpolation endpoints.
            // Exact duplicates retain their first source occurrence and do not
            // consume the arrangement budget twice; membership is unchanged.
            let mut key = edge.map(|point| point.to_array().map(f64::to_bits));
            if key[1] < key[0] {
                key.swap(0, 1);
            }
            let unique = seen.insert(key);
            required_edges += usize::from(required && unique);
            unique
        });
        if edges.len() > max_edges {
            return Err(FootprintFailure::EdgeBudget { required_edges });
        }
        Ok(Self {
            required_queries: required
                .iter()
                .map(|rings| PolygonQuery::new(rings))
                .collect(),
            excluded_queries: excluded
                .iter()
                .map(|rings| PolygonQuery::new(rings))
                .collect(),
            required,
            excluded,
            edges,
            arrangement_bounds,
            max_cells,
            coarse_cache: None,
        })
    }

    fn contains(&self, point: DVec2) -> bool {
        self.required_queries
            .iter()
            .all(|polygon| polygon.contains(point))
            && self
                .excluded_queries
                .iter()
                .all(|polygon| !polygon.contains(point))
    }

    fn roof_cells(&self, max_step_m: f64) -> Option<Vec<[DVec2; 4]>> {
        let mut result = Vec::new();
        self.visit_roof_slabs(max_step_m, |[a, b, c, d], count| {
            for i in 0..count {
                let lo = i as f64 / count as f64;
                let hi = (i + 1) as f64 / count as f64;
                result.push([
                    DVec2::new(a.x + (b.x - a.x) * lo, a.y),
                    DVec2::new(a.x + (b.x - a.x) * hi, a.y),
                    DVec2::new(d.x + (c.x - d.x) * hi, c.y),
                    DVec2::new(d.x + (c.x - d.x) * lo, c.y),
                ]);
            }
        })?;
        Some(result)
    }

    fn coarse_roof_cells(&self) -> Option<std::borrow::Cow<'_, [[DVec2; 4]]>> {
        let Some(cache) = &self.coarse_cache else {
            return self.roof_cells(4096.0).map(std::borrow::Cow::Owned);
        };
        if let Some(cells) = cache.cells.get() {
            return cells.as_deref().map(std::borrow::Cow::Borrowed);
        }
        let result = self.roof_cells(4096.0);
        let bytes = result
            .as_ref()
            .map_or(0, |cells| cells.len() * std::mem::size_of::<[DVec2; 4]>());
        if bytes > cache.remaining_bytes.get() {
            return result.map(std::borrow::Cow::Owned);
        }
        // Admission, render caps and collision share these immutable quads.
        // All planned owners charge one cell-local budget; oversized results
        // keep the original demand-driven path and its exact failure limits.
        cache
            .remaining_bytes
            .set(cache.remaining_bytes.get() - bytes);
        cache
            .cells
            .get_or_init(|| result.map(Vec::into_boxed_slice))
            .as_deref()
            .map(std::borrow::Cow::Borrowed)
    }

    fn coalesced_cells(&self, max_step_m: f64) -> Option<Vec<[DVec2; 4]>> {
        let mut result = Vec::<[DVec2; 4]>::new();
        let mut last = HashMap::<(usize, usize), usize>::new();
        self.visit_roof_slabs_with_edges(max_step_m, |cell, count, edges| {
            if count == 1 {
                if let Some(&index) = last.get(&edges) {
                    let prior = &mut result[index];
                    // The same two original segments bound both adjacent
                    // strips. Keep every source vertex/crossing event and
                    // merge only their artificial shared horizontal cut.
                    if prior[2] == cell[1]
                        && prior[3] == cell[0]
                        && cell[2].y - prior[0].y <= max_step_m
                    {
                        prior[2] = cell[2];
                        prior[3] = cell[3];
                        return;
                    }
                }
                last.insert(edges, result.len());
                result.push(cell);
            } else {
                last.remove(&edges);
                let [a, b, c, d] = cell;
                for i in 0..count {
                    let lo = i as f64 / count as f64;
                    let hi = (i + 1) as f64 / count as f64;
                    result.push([
                        DVec2::new(a.x + (b.x - a.x) * lo, a.y),
                        DVec2::new(a.x + (b.x - a.x) * hi, a.y),
                        DVec2::new(d.x + (c.x - d.x) * hi, c.y),
                        DVec2::new(d.x + (c.x - d.x) * lo, c.y),
                    ]);
                }
            }
        })?;
        Some(result)
    }

    fn roof_perimeter_samples(&self, max_step_m: f64) -> Option<Vec<DVec2>> {
        let boundary = self.boundary();
        let mut result = Vec::new();
        self.visit_roof_slabs(max_step_m, |cell, count| {
            result.extend(cell);
            for [a, b] in [[cell[0], cell[1]], [cell[3], cell[2]]] {
                if boundary.iter().any(|edge| {
                    edge.iter().all(|p| (p.y - a.y).abs() < 1.0e-5)
                        && edge[0].x.min(edge[1].x) <= a.x.max(b.x)
                        && edge[0].x.max(edge[1].x) >= a.x.min(b.x)
                }) {
                    for i in 1..count {
                        let t = i as f64 / count as f64;
                        result.push(DVec2::new(a.x + (b.x - a.x) * t, a.y));
                    }
                }
            }
        })?;
        result.retain(|point| {
            boundary
                .iter()
                .any(|edge| point_on_roof_edge(*point, *edge))
        });
        result.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
        result.dedup_by(|a, b| a.distance_squared(*b) < 1.0e-16);
        Some(result)
    }

    fn visit_roof_slabs(
        &self,
        max_step_m: f64,
        mut visit: impl FnMut([DVec2; 4], usize),
    ) -> Option<()> {
        self.visit_roof_slabs_with_edges(max_step_m, |cell, count, _| visit(cell, count))
    }

    fn visit_roof_slabs_with_edges(
        &self,
        max_step_m: f64,
        mut visit: impl FnMut([DVec2; 4], usize, (usize, usize)),
    ) -> Option<()> {
        if !max_step_m.is_finite() || max_step_m <= 0.0 {
            return None;
        }
        if self.edges.is_empty() {
            return Some(());
        }
        // Segment crossings and source vertices delimit slabs in which edge
        // order cannot change. Even-odd ring membership preserves courtyards,
        // reentrant corners and disconnected outlines without filling a hull.
        let mut rows: Vec<f64> = self.edges.iter().flatten().map(|p| p.y).collect();
        for (i, a) in self.edges.iter().enumerate() {
            for b in &self.edges[i + 1..] {
                if a[0].min(a[1]).cmpgt(b[0].max(b[1])).any()
                    || b[0].min(b[1]).cmpgt(a[0].max(a[1])).any()
                {
                    continue;
                }
                if let Some((t, _)) = intersection(*a, *b) {
                    rows.push(a[0].lerp(a[1], t).y);
                }
            }
        }
        let [lo, hi] = self.arrangement_bounds.map(|point| point.y);
        rows.retain(|&y| y >= lo && y <= hi);
        rows.extend([lo, hi, (lo + hi) * 0.5]);
        sorted_unique(&mut rows);
        let mut refined = Vec::new();
        for row in rows.windows(2) {
            let count = ((row[1] - row[0]) / max_step_m).ceil().max(1.0) as usize;
            if refined.len().saturating_add(count) > self.max_cells {
                return None;
            }
            for i in 0..count {
                refined.push(row[0] + (row[1] - row[0]) * i as f64 / count as f64);
            }
        }
        refined.push(*rows.last()?);
        let mut cell_count = 0_usize;
        for row in refined.windows(2) {
            let middle_y = (row[0] + row[1]) * 0.5;
            let mut crossings: Vec<_> = self
                .edges
                .iter()
                .enumerate()
                .filter_map(|(index, edge)| {
                    if (edge[0].y > middle_y) == (edge[1].y > middle_y) {
                        return None;
                    }
                    let x = edge_x(*edge, middle_y);
                    Some((x, index))
                })
                .collect();
            crossings.sort_by(|a, b| a.0.total_cmp(&b.0));
            for adjacent in crossings.windows(2) {
                if adjacent[1].0 - adjacent[0].0 <= GEOMETRY_EPSILON
                    || !self.contains(DVec2::new((adjacent[0].0 + adjacent[1].0) * 0.5, middle_y))
                {
                    continue;
                }
                let left = self.edges[adjacent[0].1];
                let right = self.edges[adjacent[1].1];
                let lo = [edge_x(left, row[0]), edge_x(right, row[0])];
                let hi = [edge_x(left, row[1]), edge_x(right, row[1])];
                let count = ((lo[1] - lo[0]).max(hi[1] - hi[0]) / max_step_m)
                    .ceil()
                    .max(1.0) as usize;
                if cell_count.saturating_add(count) > self.max_cells {
                    return None;
                }
                cell_count += count;
                visit(
                    [
                        DVec2::new(lo[0], row[0]),
                        DVec2::new(lo[1], row[0]),
                        DVec2::new(hi[1], row[1]),
                        DVec2::new(hi[0], row[1]),
                    ],
                    count,
                    (adjacent[0].1, adjacent[1].1),
                );
            }
        }
        Some(())
    }

    fn boundary(&self) -> Vec<[DVec2; 2]> {
        let mut result = Vec::new();
        for edge in &self.edges {
            let direction = edge[1] - edge[0];
            let mut cuts = vec![0.0, 1.0];
            for other in &self.edges {
                if let Some((t, _)) = intersection(*edge, *other) {
                    cuts.push(t);
                }
                for point in other {
                    let t = (*point - edge[0]).dot(direction) / direction.length_squared();
                    if (0.0..=1.0).contains(&t)
                        && edge[0].lerp(edge[1], t).distance_squared(*point)
                            < GEOMETRY_EPSILON * GEOMETRY_EPSILON
                    {
                        cuts.push(t);
                    }
                }
            }
            sorted_unique(&mut cuts);
            for cut in cuts.windows(2) {
                let a = edge[0].lerp(edge[1], cut[0]);
                let b = edge[0].lerp(edge[1], cut[1]);
                if a.distance_squared(b) <= GEOMETRY_EPSILON * GEOMETRY_EPSILON {
                    continue;
                }
                let midpoint = (a + b) * 0.5;
                let left = DVec2::new(-direction.y, direction.x).normalize() * 1.0e-4;
                let filled_left = self.contains(midpoint + left);
                if filled_left == self.contains(midpoint - left) {
                    continue;
                }
                let candidate = if filled_left { [a, b] } else { [b, a] };
                if !result.iter().any(|existing: &[DVec2; 2]| {
                    existing[0].distance_squared(candidate[0]) < GEOMETRY_EPSILON * GEOMETRY_EPSILON
                        && existing[1].distance_squared(candidate[1])
                            < GEOMETRY_EPSILON * GEOMETRY_EPSILON
                }) {
                    result.push(candidate);
                }
            }
        }
        result
    }
}

#[cfg(test)]
fn contains_or_boundary(rings: &Rings, point: DVec2) -> bool {
    contains(rings, point)
        || rings.iter().any(|ring| {
            (0..ring.len()).any(|i| {
                let a = DVec2::from_array(ring[i].map(f64::from));
                let b = DVec2::from_array(ring[(i + 1) % ring.len()].map(f64::from));
                let t =
                    ((point - a).dot(b - a) / a.distance_squared(b).max(1.0e-12)).clamp(0.0, 1.0);
                point.distance_squared(a.lerp(b, t)) < 1.0e-6
            })
        })
}

struct PolygonQuery {
    edges: Vec<[DVec2; 2]>,
    y_bounds: [f64; 2],
    rows: Vec<Vec<usize>>,
}
impl PolygonQuery {
    fn new(rings: &Rings) -> Self {
        let edges: Vec<_> = rings
            .iter()
            .flat_map(|ring| {
                (0..ring.len()).map(|i| {
                    [
                        DVec2::from_array(ring[i].map(f64::from)),
                        DVec2::from_array(ring[(i + 1) % ring.len()].map(f64::from)),
                    ]
                })
            })
            .collect();
        let y_bounds = edges
            .iter()
            .flatten()
            .fold([f64::INFINITY, f64::NEG_INFINITY], |[lo, hi], p| {
                [lo.min(p.y), hi.max(p.y)]
            });
        let mut query = Self {
            edges,
            y_bounds,
            rows: Vec::new(),
        };
        if query.edges.len() >= 24 && y_bounds[1] > y_bounds[0] {
            query.rows = vec![Vec::new(); 32];
            for (id, edge) in query.edges.iter().enumerate() {
                let lo = query.row(edge[0].y.min(edge[1].y));
                let hi = query.row(edge[0].y.max(edge[1].y));
                for row in &mut query.rows[lo..=hi] {
                    row.push(id);
                }
            }
        }
        query
    }

    fn any_edge_in_bounds(
        &self,
        bounds: (DVec2, DVec2),
        mut touches: impl FnMut([DVec2; 2]) -> bool,
    ) -> bool {
        if !self.rows.is_empty() && bounds.0.is_finite() && bounds.1.is_finite() {
            if bounds.0.y > self.y_bounds[1] || bounds.1.y < self.y_bounds[0] {
                return false;
            }
            let rows = &self.rows[self.row(bounds.0.y)..=self.row(bounds.1.y)];
            let count: usize = rows.iter().map(Vec::len).sum();
            if count < self.edges.len() {
                // Query bounds already include the exact overlap predicate's
                // tolerance. Inclusive edge bins retain horizontal edges and
                // touching endpoints; duplicates only repeat a pure predicate.
                return rows
                    .iter()
                    .flatten()
                    .any(|&index| touches(self.edges[index]));
            }
        }
        self.edges.iter().copied().any(touches)
    }

    fn row(&self, y: f64) -> usize {
        (((y - self.y_bounds[0]) / (self.y_bounds[1] - self.y_bounds[0]) * 32.0) as usize).min(31)
    }

    fn contains(&self, point: DVec2) -> bool {
        if point.y < self.y_bounds[0] || point.y > self.y_bounds[1] {
            return false;
        }
        let crosses = |&[a, b]: &[DVec2; 2]| {
            (a.y > point.y) != (b.y > point.y) && point.x < edge_x([a, b], point.y)
        };
        if self.rows.is_empty() {
            self.edges
                .iter()
                .fold(false, |inside, edge| inside ^ crosses(edge))
        } else {
            // Row candidates retain source ordering and the original even-odd
            // boundary predicate. Holes and self-crossings need no special case.
            self.rows[self.row(point.y)]
                .iter()
                .fold(false, |inside, &id| inside ^ crosses(&self.edges[id]))
        }
    }
}

#[cfg(test)]
fn contains(rings: &Rings, point: DVec2) -> bool {
    let mut inside = false;
    for ring in rings {
        for i in 0..ring.len() {
            let a = DVec2::from_array(ring[i].map(f64::from));
            let b = DVec2::from_array(ring[(i + 1) % ring.len()].map(f64::from));
            if (a.y > point.y) != (b.y > point.y) && point.x < edge_x([a, b], point.y) {
                inside = !inside;
            }
        }
    }
    inside
}

fn edge_x(edge: [DVec2; 2], y: f64) -> f64 {
    edge[0].x + (edge[1].x - edge[0].x) * (y - edge[0].y) / (edge[1].y - edge[0].y)
}

fn intersection(a: [DVec2; 2], b: [DVec2; 2]) -> Option<(f64, f64)> {
    let (r, s) = (a[1] - a[0], b[1] - b[0]);
    let cross = r.perp_dot(s);
    if cross.abs() < GEOMETRY_EPSILON {
        return None;
    }
    let delta = b[0] - a[0];
    let (t, u) = (delta.perp_dot(s) / cross, delta.perp_dot(r) / cross);
    ((0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)).then_some((t, u))
}

fn sorted_unique(values: &mut Vec<f64>) {
    values.sort_by(f64::total_cmp);
    values.dedup_by(|a, b| (*a - *b).abs() <= GEOMETRY_EPSILON);
}

#[cfg(test)]
mod tests {
    fn overlaps(a: &Rings, b: &Rings) -> bool {
        match (bounds(a), bounds(b)) {
            (Some((al, ah)), Some((bl, bh))) => {
                al.x < bh.x && bl.x < ah.x && al.y < bh.y && bl.y < ah.y
            }
            _ => false,
        }
    }

    use super::*;

    #[test]
    fn minified_arches_preserve_area_bounds_symmetry_and_silhouette_error() {
        for shape in [
            OpeningShape::SegmentalArch,
            OpeningShape::RoundArch,
            OpeningShape::PointedArch,
        ] {
            for width in [0.35, 0.8, 1.2, 2.4, 6.0] {
                for height in [0.45, 1.1, 3.0, 8.0] {
                    for offset in [0.0, 10_000.0] {
                        let x = [offset, offset + width];
                        let y = [offset + 100.0, offset + 100.0 + height];
                        let original = opening_outline(x, y, shape);
                        let reduced = minified_opening(&original, width.max(height)).unwrap();
                        assert_eq!(reduced.len(), 5);
                        assert_eq!(reduced[0], original[0]);
                        assert_eq!(reduced[1], original[1]);
                        assert_eq!(reduced[3], original[5]);
                        assert!(
                            (outline_area(&original) - outline_area(&reduced)).abs()
                                <= outline_area(&original).max(1.0) * 1.0e-10
                        );
                        assert_eq!(reduced[2].y, reduced[4].y);
                        assert!((reduced[2].x + reduced[4].x - 2.0 * reduced[3].x).abs() < 1.0e-9);
                        assert!(
                            reduced
                                .iter()
                                .all(|p| p.x >= x[0] && p.x <= x[1] && p.y >= y[0] && p.y <= y[1])
                        );
                        for i in 0..reduced.len() {
                            assert!(
                                (reduced[(i + 1) % 5] - reduced[i])
                                    .perp_dot(reduced[(i + 2) % 5] - reduced[(i + 1) % 5])
                                    > 0.0
                            );
                        }
                        let bound = outline_distance_bound(&original, &reduced);
                        assert!(bound > 0.0);
                        assert!(minified_opening(&original, bound * 0.999).is_none());
                        assert!(minified_opening(&original, bound * 1.001).is_some());
                    }
                }
            }
        }
        assert!(
            minified_opening(
                &opening_outline([0.0, 1.0], [0.0, 2.0], OpeningShape::Rectangular),
                1.0
            )
            .is_none()
        );
    }

    #[test]
    fn minified_glass_and_flat_surround_keep_materials_and_exact_near_geometry() {
        for level in [15, 20] {
            let frame = frame(level);
            let (lat, lon) = frame.column_latlon(0.5, 0.5);
            let projection = Projection {
                frame: &frame,
                lat,
                lon,
            };
            let plane = FacadePlane {
                shadow_units: None,
                projection: &projection,
                origin: DVec2::ZERO,
                direction: DVec2::X,
                outward: -DVec2::Y,
            };
            for shape in [
                OpeningShape::SegmentalArch,
                OpeningShape::RoundArch,
                OpeningShape::PointedArch,
            ] {
                for relief in [false, true] {
                    let x = [-0.5, 0.5];
                    let y = [100.0, 102.0];
                    let (inner, outer) = plane.opening_outlines(x, y, shape, 0.08, relief);
                    let outer = outer.unwrap();
                    let original = opening_outline(x, y, shape);
                    if relief || level == 20 {
                        assert_eq!(&*inner, &*original);
                        assert_eq!(
                            &*outer,
                            &*opening_outline([-0.58, 0.58], [99.92, 102.08], shape)
                        );
                    } else {
                        assert_eq!(inner.len(), 5);
                        assert_eq!(outer.len(), 5);
                        assert!(inner.iter().all(|&p| convex_outline_contains(&outer, p)));
                        for material in [crate::material::WINDOW, crate::material::LIT_WINDOW] {
                            let glass = SurfaceAppearance {
                                material,
                                tint: [81, 102, 116],
                            };
                            let trim = SurfaceAppearance {
                                material: crate::material::TIMBER_WALL,
                                tint: [134, 93, 48],
                            };
                            let mut mesh = MeshBuffers::default();
                            plane.opening(&mut mesh, x, y, 0.025, shape, 0.08, glass, trim, false);
                            assert_eq!(mesh.indices.len() / 3, 6);
                            assert_eq!(
                                mesh.vertices
                                    .iter()
                                    .filter(|v| ((v.face_ao >> 5) & 255) as u8 == material)
                                    .count(),
                                9
                            );
                            assert!(mesh.vertices.iter().all(|v| {
                                match ((v.face_ao >> 5) & 255) as u8 {
                                    m if m == material => v.rgba == [81, 102, 116, 255],
                                    m if m == crate::material::TIMBER_WALL => {
                                        v.rgba == [134, 93, 48, 255]
                                    }
                                    _ => false,
                                }
                            }));
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn minified_party_wall_proof_checks_the_selected_raised_spring() {
        let frame = frame(15);
        let (lat, lon) = frame.column_latlon(0.5, 0.5);
        let projection = Projection {
            frame: &frame,
            lat,
            lon,
        };
        let plane = FacadePlane {
            shadow_units: None,
            projection: &projection,
            origin: DVec2::ZERO,
            direction: DVec2::X,
            outward: DVec2::Y,
        };
        let x = [-0.5, 0.5];
        let y = [100.0, 102.0];
        let shape = OpeningShape::SegmentalArch;
        let original = opening_outline(x, y, shape);
        let (selected, _) = plane.opening_outlines(x, y, shape, 0.0, false);
        assert_eq!(selected.len(), 5);
        let edge = original[3] - original[2];
        let direction = DVec2::new(edge.y, -edge.x).normalize();
        let origin = plane.point(0.0, 100.0, 0.025);
        let axis_x = plane.point(1.0, 100.0, 0.025) - origin;
        let axis_y = plane.point(0.0, 101.0, 0.025) - origin;
        let normal = (axis_x * direction.x + axis_y * direction.y).normalize();
        let support = |outline: &OpeningOutline| {
            outline
                .iter()
                .map(|p| normal.dot(plane.point(p.x, p.y, 0.025).as_vec3().as_dvec3() - origin))
                .fold(f64::NEG_INFINITY, f64::max)
        };
        let original_support = support(&original);
        let selected_support = support(&selected);
        assert!(selected_support - original_support > 0.001);
        let rings = vec![square(-10.0, 10.0)];
        let wall = PartyWall {
            edge: [DVec2::new(-1.0, 0.0), DVec2::new(1.0, 0.0)],
            body: Rc::new(OpaqueBody {
                query: PolygonQuery::new(&rings),
                core: OpaqueCore {
                    planes: vec![OcclusionPlane {
                        anchor: origin + normal * (original_support + selected_support) * 0.5,
                        normal,
                    }],
                    floor_y: Some(origin.y - 10.0),
                    roof_y: Some(origin.y + 10.0),
                    invalid: false,
                },
                foundation: std::cell::RefCell::new(None),
                used: std::cell::Cell::new(false),
                base: 90.0,
                eave: 110.0,
            }),
        };
        assert!(wall.covers_opening(&plane, x, y, 0.025, &original));
        assert!(!wall.covers_opening(&plane, x, y, 0.025, &selected));
        let mut context = FacadeContext::from_sources(&rings, &[], &[], 100.0);
        context.party_walls.push(wall);
        assert!(!context.hidden_glazing(&plane, x, y, 0.025, shape, false));
        assert!(context.hidden_glazing(&plane, x, y, 0.025, shape, true));
    }

    #[test]
    fn facade_ledge_returns_and_opening_rims_face_outward() {
        let frame = frame(17);
        let (lat, lon) = frame.column_latlon(0.5, 0.5);
        let projection = Projection {
            frame: &frame,
            lat,
            lon,
        };
        let plane = FacadePlane {
            shadow_units: None,
            projection: &projection,
            origin: DVec2::ZERO,
            direction: DVec2::X,
            outward: -DVec2::Y,
        };
        let appearance = SurfaceAppearance {
            material: crate::material::WALL,
            tint: [180; 3],
        };
        let mut mesh = MeshBuffers::default();
        plane.structural_strip(
            &mut mesh,
            [-1.0, 1.0],
            [100.0, 101.0],
            [0.025, 0.8],
            appearance,
        );
        let center = plane.point(0.0, 100.5, 0.4125);
        assert_eq!(mesh.indices.len(), 30);
        for triangle in mesh.indices.chunks_exact(3) {
            let p: [DVec3; 3] = std::array::from_fn(|i| {
                DVec3::from_array(mesh.vertices[triangle[i] as usize].pos.map(f64::from))
            });
            let normal = (p[1] - p[0]).cross(p[2] - p[0]);
            assert!(normal.dot((p[0] + p[1] + p[2]) / 3.0 - center) > 0.0);
        }
        for shape in [
            OpeningShape::Rectangular,
            OpeningShape::SegmentalArch,
            OpeningShape::RoundArch,
            OpeningShape::PointedArch,
        ] {
            let mut mesh = MeshBuffers::default();
            plane.opening(
                &mut mesh,
                [-1.0, 1.0],
                [100.0, 103.0],
                0.06,
                shape,
                0.15,
                appearance,
                appearance,
                true,
            );
            let outward = plane.point(0.0, 101.0, 1.0) - plane.point(0.0, 101.0, 0.0);
            assert!(!mesh.indices.is_empty());
            for triangle in mesh.indices.chunks_exact(3) {
                let p: [DVec3; 3] = std::array::from_fn(|i| {
                    DVec3::from_array(mesh.vertices[triangle[i] as usize].pos.map(f64::from))
                });
                assert!(
                    (p[1] - p[0]).cross(p[2] - p[0]).dot(outward) > 0.0,
                    "{shape:?}"
                );
            }
        }
    }

    #[test]
    fn faceted_cones_keep_the_source_domain_peak_and_bounded_planar_work() {
        for reverse in [false, true] {
            let mut outer = vec![[-7.0, -6.0], [7.0, -6.0], [7.0, 6.0], [-7.0, 6.0]];
            if reverse {
                outer.reverse();
            }
            let source = vec![outer];
            let mut roof = RoofField::new(&source, Some(RoofShape::Pyramidal), 104.0, 108.0, 0.7);
            roof.conical = true;
            assert!((roof.height(DVec2::ZERO) - 108.0).abs() < 1.0e-10);
            assert!((roof.height(DVec2::new(7.0, 6.0)) - 104.0).abs() < 1.0e-10);
            for hole in [false, true] {
                let excluded = vec![square(-1.0, 1.0)];
                let domain =
                    Footprint::new(vec![&source], if hole { vec![&excluded] } else { vec![] })
                        .unwrap();
                let geometry = analytic_roof_geometry(
                    domain.roof_cells(4096.0).unwrap(),
                    &domain.boundary(),
                    &domain.roof_perimeter_samples(3.0).unwrap(),
                    &roof,
                    &roof.planes().unwrap(),
                    &|p, h| DVec3::new(p.x, h, p.y),
                )
                .expect("cones must use the bounded planar path");
                assert!(geometry.triangles.len() < 256);
                let area = roof_planar_area(&geometry);
                assert!((area - (168.0 - if hole { 4.0 } else { 0.0 })).abs() < 1.0e-7);
                for triangle in &geometry.triangles {
                    let center = triangle.points.iter().copied().sum::<DVec2>() / 3.0;
                    assert!(domain.contains(center));
                    assert!(
                        triangle
                            .heights
                            .iter()
                            .all(|&h| (103.99999..=108.00001).contains(&h))
                    );
                    assert!(
                        (roof.height(center) - triangle.heights.iter().sum::<f64>() / 3.0).abs()
                            < 1.0e-7
                    );
                }
            }
        }
    }

    #[test]
    fn convex_roof_planes_match_the_source_field() {
        let polygons = [
            vec![[-24.0, -12.0], [24.0, -12.0], [24.0, 12.0], [-24.0, 12.0]],
            vec![[-17.0, -8.0], [23.0, -12.0], [11.0, 19.0]],
            vec![
                [-21.0, -11.0],
                [15.0, -13.0],
                [24.0, 3.0],
                [9.0, 16.0],
                [-16.0, 9.0],
            ],
        ];
        for polygon in &polygons {
            for angle in [0.0_f64, 0.27, 1.81] {
                for reverse in [false, true] {
                    let mut ring: Vec<_> = polygon
                        .iter()
                        .map(|&p| {
                            let p = DVec2::from_array(p);
                            (DVec2::new(
                                p.x * angle.cos() - p.y * angle.sin(),
                                p.x * angle.sin() + p.y * angle.cos(),
                            ) + DVec2::new(731.25, -216.5))
                            .as_vec2()
                            .to_array()
                        })
                        .collect();
                    if reverse {
                        ring.reverse();
                    }
                    let rings = vec![ring];
                    let domain = Footprint::new(vec![&rings], vec![]).unwrap();
                    let (lo, hi) = bounds(&rings).unwrap();
                    for shape in [
                        RoofShape::Gabled,
                        RoofShape::Hipped,
                        RoofShape::Pyramidal,
                        RoofShape::Mansard,
                        RoofShape::Gambrel,
                        RoofShape::Skillion,
                    ] {
                        for rise in [1.5, 12.0] {
                            let roof =
                                RoofField::new(&rings, Some(shape), 100.0, 100.0 + rise, 0.7);
                            let planes = roof.planes().unwrap();
                            for y in 0..48 {
                                for x in 0..48 {
                                    let p = lo
                                        + (hi - lo)
                                            * DVec2::new(
                                                (x as f64 + 0.5) / 48.0,
                                                (y as f64 + 0.5) / 48.0,
                                            );
                                    if domain.contains(p) {
                                        let at = planes
                                            .iter()
                                            .map(|plane| plane.at(p))
                                            .fold(f64::INFINITY, f64::min);
                                        assert!(
                                            (at - roof.height(p)).abs() < 1.0e-7,
                                            "{shape:?} {angle} {reverse}: {p:?}"
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        let collinear = vec![vec![
            [-12.0, -8.0],
            [0.0, -8.0],
            [12.0, -8.0],
            [12.0, 8.0],
            [-12.0, 8.0],
            [-12.0, -8.0],
        ]];
        let roof = RoofField::new(&collinear, Some(RoofShape::Hipped), 10.0, 18.0, 0.6);
        assert_eq!(
            roof.planes().unwrap().len(),
            5,
            "collinear supports must not duplicate roof regions"
        );
    }

    #[test]
    fn analytic_broken_slope_roofs_keep_the_breakpoint_rise_and_ignore_hip_pitch() {
        let rings = vec![vec![
            [-24.0, -12.0],
            [24.0, -12.0],
            [24.0, 12.0],
            [-24.0, 12.0],
        ]];
        for shape in [RoofShape::Mansard, RoofShape::Gambrel, RoofShape::Skillion] {
            for pitch in [-1.0, 0.0, 0.7, 5.0, f32::NAN] {
                let roof = RoofField::new(&rings, Some(shape), 103.0, 111.0, pitch);
                let planes = roof.planes().unwrap();
                for q in [0.0, 1.0e-10, 0.28 - 1.0e-10, 0.28, 0.28 + 1.0e-10, 0.7, 1.0] {
                    let p = if shape == RoofShape::Skillion {
                        DVec2::new(0.0, -12.0 + q * 24.0)
                    } else {
                        DVec2::new(0.0, -12.0 + q * 12.0)
                    };
                    let height = planes
                        .iter()
                        .map(|plane| plane.at(p))
                        .fold(f64::INFINITY, f64::min);
                    assert!(
                        (height - roof.height(p)).abs() < 1.0e-10,
                        "{shape:?} q={q} pitch={pitch}"
                    );
                    let fraction = if shape == RoofShape::Skillion {
                        q
                    } else if q < 0.28 {
                        q / 0.28 * 0.78
                    } else {
                        0.78 + (q - 0.28) / 0.72 * 0.22
                    };
                    assert!((height - (103.0 + 8.0 * fraction)).abs() < 1.0e-10);
                }
                if shape != RoofShape::Skillion {
                    let expected = if shape == RoofShape::Mansard {
                        103.0
                    } else {
                        111.0
                    };
                    assert!((roof.height(DVec2::new(24.0, 0.0)) - expected).abs() < 1.0e-10);
                }
            }
        }
    }

    #[test]
    fn analytic_roof_keeps_holes_boundaries_and_the_original_perimeter_grid() {
        let sources = [
            vec![vec![
                [-24.0, -12.0],
                [24.0, -12.0],
                [24.0, 12.0],
                [-24.0, 12.0],
            ]],
            vec![vec![[-17.0, -8.0], [23.0, -12.0], [11.0, 19.0]]],
            vec![vec![
                [-21.0, -11.0],
                [15.0, -13.0],
                [24.0, 3.0],
                [9.0, 16.0],
                [-16.0, 9.0],
            ]],
            vec![vec![
                [-24.0, -12.0],
                [0.0, -12.0],
                [24.0, -12.0],
                [24.0, 12.0],
                [-24.0, 12.0],
                [-24.0, -12.0],
            ]],
        ];
        let exclusions = [
            vec![],
            vec![square(-3.0, 3.0)],
            vec![vec![[-2.0, -40.0], [2.0, -40.0], [2.0, 40.0], [-2.0, 40.0]]],
        ];
        for source in &sources {
            for exclusion in &exclusions {
                let excluded = (!exclusion.is_empty())
                    .then_some(exclusion)
                    .into_iter()
                    .collect();
                let domain = Footprint::new(vec![source], excluded).unwrap();
                let boundary = domain.boundary();
                let fine = domain.roof_cells(3.0).unwrap();
                let expected: Vec<_> = fine
                    .iter()
                    .flatten()
                    .copied()
                    .filter(|&p| boundary.iter().any(|&e| point_on_roof_edge(p, e)))
                    .collect();
                let perimeter = domain.roof_perimeter_samples(3.0).unwrap();
                for p in &expected {
                    assert!(
                        perimeter.iter().any(|q| p.distance_squared(*q) < 1.0e-20),
                        "missing perimeter point {p:?}"
                    );
                }
                for p in &perimeter {
                    assert!(
                        expected.iter().any(|q| p.distance_squared(*q) < 1.0e-20),
                        "extra perimeter point {p:?}"
                    );
                }
                for shape in [
                    RoofShape::Gabled,
                    RoofShape::Hipped,
                    RoofShape::Pyramidal,
                    RoofShape::Mansard,
                    RoofShape::Gambrel,
                    RoofShape::Skillion,
                ] {
                    let roof = RoofField::new(source, Some(shape), 100.0, 108.0, 0.7);
                    let geometry = analytic_roof_geometry(
                        domain.roof_cells(4096.0).unwrap(),
                        &boundary,
                        &perimeter,
                        &roof,
                        &roof.planes().unwrap(),
                        &|p, h| DVec3::new(p.x, h, p.y),
                    )
                    .unwrap();
                    let old =
                        roof_geometry(fine.clone(), &boundary, &|p| roof.height(p), true).unwrap();
                    let area = |triangles: &[RoofTriangle]| {
                        triangles
                            .iter()
                            .map(|t| {
                                (t.points[1] - t.points[0])
                                    .perp_dot(t.points[2] - t.points[0])
                                    .abs()
                            })
                            .sum::<f64>()
                    };
                    assert!((area(&old.triangles) - area(&geometry.triangles)).abs() < 1.0e-6);
                    for p in &perimeter {
                        assert!(
                            geometry
                                .triangles
                                .iter()
                                .flat_map(|t| t.points)
                                .any(|q| p.distance_squared(q) < 1.0e-16),
                            "collinear boundary knot lost: {p:?}"
                        );
                    }
                    for triangle in &geometry.triangles {
                        assert!(
                            domain.contains(triangle.points.iter().copied().sum::<DVec2>() / 3.0)
                        );
                        for a in 1..8 {
                            for b in 1..8 - a {
                                let weights =
                                    [a as f64 / 8.0, b as f64 / 8.0, (8 - a - b) as f64 / 8.0];
                                let p = (0..3).map(|i| triangle.points[i] * weights[i]).sum();
                                let height: f64 =
                                    (0..3).map(|i| triangle.heights[i] * weights[i]).sum();
                                assert!((height - roof.height(p)).abs() < 1.0e-5);
                            }
                        }
                    }
                    if source == &sources[0] && exclusion.is_empty() {
                        assert!(
                            geometry.triangles.len() * 3 < old.triangles.len(),
                            "{shape:?}: {} -> {} triangles",
                            old.triangles.len(),
                            geometry.triangles.len()
                        );
                        eprintln!(
                            "analytic roof {shape:?}: {} -> {} triangles",
                            old.triangles.len(),
                            geometry.triangles.len()
                        );
                    }
                }
            }
        }
        let bounded = Footprint::with_limits(vec![&sources[0]], vec![], 100, 5).unwrap();
        assert!(bounded.roof_cells(3.0).is_none());
        assert!(bounded.roof_perimeter_samples(3.0).is_none());
        assert!(bounded.roof_perimeter_samples(4096.0).is_some());
    }

    #[test]
    fn analytic_roof_preserves_partially_obstructed_eave_spans() {
        for shape in [
            RoofShape::Gabled,
            RoofShape::Mansard,
            RoofShape::Gambrel,
            RoofShape::Skillion,
        ] {
            let source = vec![vec![
                [-24.0, -12.0],
                [24.0, -12.0],
                [24.0, 12.0],
                [-24.0, 12.0],
            ]];
            let domain = Footprint::new(vec![&source], vec![]).unwrap();
            let roof = RoofField::new(&source, Some(shape), 112.0, 116.0, 0.7);
            let boundary = domain.boundary();
            let perimeter = domain.roof_perimeter_samples(3.0).unwrap();
            let old = roof_geometry(
                domain.roof_cells(3.0).unwrap(),
                &boundary,
                &|p| roof.height(p),
                true,
            )
            .unwrap();
            let geometry = analytic_roof_geometry(
                domain.roof_cells(4096.0).unwrap(),
                &boundary,
                &perimeter,
                &roof,
                &roof.planes().unwrap(),
                &|p, h| DVec3::new(p.x, h, p.y),
            )
            .unwrap();
            let mut neighbors = pack_with(
                vec![vec![[6.0, -13.0], [8.0, -13.0], [8.0, -11.5], [6.0, -11.5]]],
                25.0,
            )
            .buildings;
            neighbors[0].min_height_m = 0.0;
            let context = FacadeContext::from_sources(&source, &neighbors, &[0], 100.0);
            let frame = frame(17);
            let projection = Projection {
                frame: &frame,
                lat: 0.0,
                lon: 0.0,
            };
            let spans = |geometry: &RoofGeometry| {
                let mut result = Vec::new();
                for edge in &geometry.boundary {
                    if edge.points.iter().all(|p| (p.y + 12.0).abs() < 1.0e-8) {
                        let direction = (edge.points[1] - edge.points[0]).normalize();
                        let plane = FacadePlane {
                            shadow_units: None,
                            projection: &projection,
                            origin: edge.points[0],
                            direction,
                            outward: DVec2::new(direction.y, -direction.x),
                        };
                        if plane.clear(
                            &context,
                            [0.0, edge.points[0].distance(edge.points[1])],
                            [111.8, 116.01],
                            0.4,
                        ) {
                            result.push([edge.points[0].x, edge.points[1].x]);
                        }
                    }
                }
                result.sort_by(|a, b| a[0].total_cmp(&b[0]));
                result
            };
            let expected = spans(&old);
            assert!(!expected.is_empty());
            let length: f64 = expected.iter().map(|span| (span[1] - span[0]).abs()).sum();
            assert!(length > 30.0 && length < 48.0);
            assert_eq!(spans(&geometry), expected);
        }
    }

    #[test]
    fn analytic_roof_projection_and_shape_guards_preserve_sampling_fallbacks() {
        let rings = vec![square(-30.0, 30.0)];
        for shape in [RoofShape::Dome, RoofShape::Onion, RoofShape::Flat] {
            assert!(
                RoofField::new(&rings, Some(shape), 100.0, 106.0, 0.7)
                    .planes()
                    .is_none()
            );
        }
        for shape in [
            RoofShape::Hipped,
            RoofShape::Mansard,
            RoofShape::Gambrel,
            RoofShape::Skillion,
        ] {
            let courtyard = vec![square(-30.0, 30.0), square(-3.0, 3.0)];
            assert!(
                RoofField::new(&courtyard, Some(shape), 100.0, 106.0, 0.7)
                    .planes()
                    .is_none()
            );
            let concave = vec![vec![
                [0.0, 0.0],
                [20.0, 0.0],
                [20.0, 5.0],
                [5.0, 5.0],
                [5.0, 20.0],
                [0.0, 20.0],
            ]];
            assert!(
                RoofField::new(&concave, Some(shape), 100.0, 106.0, 0.7)
                    .planes()
                    .is_none()
            );
            let mut tiered = RoofField::new(&rings, Some(shape), 100.0, 106.0, 0.7);
            tiered.tiered = true;
            assert!(tiered.planes().is_none());
            let frame = frame(17);
            for (radius, latitude, accepted) in [
                (30.0, 52.0_f64, true),
                (30.0, 89.999, false),
                (2000.0, 0.0, false),
            ] {
                let rings = vec![square(-radius, radius)];
                let domain = Footprint::new(vec![&rings], vec![]).unwrap();
                let roof = RoofField::new(&rings, Some(shape), 100.0, 106.0, 0.7);
                let projection = Projection {
                    frame: &frame,
                    lat: latitude.to_radians(),
                    lon: 0.0,
                };
                assert_eq!(
                    analytic_roof(&domain, &domain.boundary(), &roof, false, 3.0, &projection)
                        .is_some(),
                    accepted,
                    "radius={radius}, latitude={latitude}"
                );
            }
        }
    }

    #[test]
    #[ignore = "local roof generation benchmark"]
    fn analytic_roof_generation_benchmark() {
        let frame = frame(17);
        let projection = Projection {
            frame: &frame,
            lat: 52.37_f64.to_radians(),
            lon: 4.90_f64.to_radians(),
        };
        for (name, rings) in [
            (
                "house",
                vec![vec![[-9.0, -6.0], [9.0, -6.0], [9.0, 6.0], [-9.0, 6.0]]],
            ),
            (
                "large-rectangle",
                vec![vec![
                    [-40.0, -24.0],
                    [40.0, -24.0],
                    [40.0, 24.0],
                    [-40.0, 24.0],
                ]],
            ),
            (
                "convex-24",
                vec![
                    (0..24)
                        .map(|i| {
                            let angle = i as f64 * std::f64::consts::TAU / 24.0;
                            [(angle.cos() * 36.0) as f32, (angle.sin() * 24.0) as f32]
                        })
                        .collect(),
                ],
            ),
        ] {
            let domain = Footprint::new(vec![&rings], vec![]).unwrap();
            let boundary = domain.boundary();
            for shape in [
                RoofShape::Hipped,
                RoofShape::Mansard,
                RoofShape::Gambrel,
                RoofShape::Skillion,
            ] {
                let roof = RoofField::new(&rings, Some(shape), 110.0, 116.0, 0.7);
                let run = |analytic: bool| {
                    let geometry = if analytic {
                        analytic_roof(&domain, &boundary, &roof, false, 3.0, &projection).unwrap()
                    } else {
                        roof_geometry(
                            domain.roof_cells(3.0).unwrap(),
                            &boundary,
                            &|p| roof.height(p),
                            true,
                        )
                        .unwrap()
                    };
                    let count = geometry.triangles.len();
                    let mut mesh = MeshBuffers::default();
                    emit_roof(
                        &projection,
                        &mut mesh,
                        &geometry,
                        SurfaceAppearance {
                            material: crate::material::TILE_ROOF,
                            tint: [100; 3],
                        },
                    );
                    std::hint::black_box(mesh);
                    count
                };
                let mut times = [Vec::new(), Vec::new()];
                let triangles = [run(false), run(true)];
                for round in 0..5 {
                    for mode in [round % 2, 1 - round % 2] {
                        let start = vearth_platform::time::Instant::now();
                        for _ in 0..10 {
                            std::hint::black_box(run(mode == 1));
                        }
                        times[mode].push(start.elapsed().as_secs_f64() * 1000.0 / 10.0);
                    }
                }
                for values in &mut times {
                    values.sort_by(f64::total_cmp);
                }
                eprintln!(
                    "{name} {shape:?}: {:.3} -> {:.3}ms ({:.2}x), {} -> {} triangles",
                    times[0][2],
                    times[1][2],
                    times[0][2] / times[1][2],
                    triangles[0],
                    triangles[1]
                );
            }
        }
    }

    #[test]
    fn source_overlap_sweep_preserves_ordered_ownership_candidates() {
        let mut rings: Vec<Rings> = (0..4096)
            .map(|i| {
                let x = ((i * 719) % 97) as f32 * 12.0;
                let y = ((i * 227) % 113) as f32 * 10.0;
                vec![vec![
                    [x, y],
                    [x + 10.0, y],
                    [x + 10.0, y + 12.0],
                    [x, y + 12.0],
                ]]
            })
            .collect();
        // Coincident, touching, containing, degenerate and absent footprints
        // exercise the strict broadphase boundary and original-index ordering.
        rings.extend([
            vec![square(0.0, 20.0), square(3.0, 8.0)],
            vec![square(0.0, 20.0)],
            vec![square(20.0, 30.0)],
            vec![square(0.0, 0.005)],
            vec![square(5.0, 10.0)],
            vec![],
        ]);
        let source_bounds: Vec<_> = rings.iter().map(bounds).collect();
        let started = vearth_platform::time::Instant::now();
        let expected: Vec<Vec<_>> = rings
            .iter()
            .enumerate()
            .map(|(i, a)| {
                rings
                    .iter()
                    .enumerate()
                    .filter_map(|(j, b)| (i != j && overlaps(a, b)).then_some(j))
                    .collect()
            })
            .collect();
        let reference_time = started.elapsed();
        let started = vearth_platform::time::Instant::now();
        let actual = overlapping_sources(&source_bounds);
        let actual_time = started.elapsed();
        assert_eq!(actual, expected);
        eprintln!("building overlap reference {reference_time:?}, sweep {actual_time:?}");
    }

    fn frame(level: u8) -> ChunkFrame {
        use vearth_core::{
            cell::{CellId, ChunkAddr},
            cubesphere::Face,
        };
        ChunkFrame::of(ChunkAddr {
            cell: CellId {
                face: Face(0),
                level,
                i: 1 << (level - 1),
                j: 1 << (level - 1),
            },
            k: 0,
        })
    }
    fn pack_with(rings: Rings, height: f32) -> FeaturePack {
        let mut pack = FeaturePack::empty();
        pack.buildings.push(Building {
            location_rad: None,
            rings,
            height_m: height,
            min_height_m: 0.0,
            seed: 42,
            kind: BuildingKind::Building,
            roof_shape: None,
            use_kind: Default::default(),
        });
        pack
    }
    fn generate_test(
        frame: &ChunkFrame,
        pack: &FeaturePack,
        ground: &dyn Fn(f64, f64, f32) -> Option<f32>,
    ) -> BuildingSurfaceData {
        let ov = tier0::build_overlay(
            frame.addr.cell,
            frame,
            pack,
            &|_, _| 100.0,
            0,
            None,
            None,
            Some(&|_| true),
        );
        let result = generate(frame, pack, &ov, &[u32::MAX; N], ground, &|_, _, _| {
            panic!("unmeasured building queried DSM")
        })
        .unwrap();
        crate::collision::SurfaceCollision::from_triangles(result.collision.clone())
            .validate()
            .unwrap();
        result
    }

    fn generate_resolved_facades<const PRUNE_HIDDEN_WALLS: bool>(
        frame: &ChunkFrame,
        pack: &FeaturePack,
        overlay: &CellOverlay,
        simple: bool,
    ) -> BuildingSurfaceData {
        let ground = |_, _, _| Some(100.0);
        let surface = |_, _, _| None;
        let mut plan =
            prepare_envelopes(frame, pack, overlay, &[u32::MAX; N], &ground, &surface).unwrap();
        // These fixtures compare source geometry, shading and occlusion while
        // all facade details are resolved; resolution thresholds have separate tests.
        plan.facade_resolution = FacadeResolution::FULL;
        generate_prepared_envelopes::<PRUNE_HIDDEN_WALLS>(
            frame,
            pack,
            overlay,
            &[u32::MAX; N],
            &ground,
            &surface,
            simple,
            usize::MAX,
            &plan,
        )
        .unwrap()
    }

    fn generate_shaded_facade_test(frame: &ChunkFrame, pack: &FeaturePack) -> BuildingSurfaceData {
        let mut overlay = tier0::build_overlay(
            frame.addr.cell,
            frame,
            pack,
            &|_, _| 100.0,
            0,
            None,
            None,
            Some(&|_| true),
        );
        for stamp in &mut overlay.source_buildings {
            // Broad sun shades remain optional metric projections after
            // individual window frames and dividers have been removed.
            stamp.design.detail = crate::regional_architecture::DetailFamily::Shaded;
            stamp.design.expression = 0;
            stamp.highrise = None;
        }
        let result = generate(
            frame,
            pack,
            &overlay,
            &[u32::MAX; N],
            &|_, _, _| Some(100.0),
            &|_, _, _| panic!("unmeasured building queried DSM"),
        )
        .unwrap();
        assert!(result.complete);
        crate::collision::SurfaceCollision::from_triangles(result.collision.clone())
            .validate()
            .unwrap();
        result
    }
    fn world_height(frame: &ChunkFrame, p: [f32; 3]) -> f64 {
        ellipsoid::ecef_to_geodetic(frame.local_to_ecef(DVec3::from_array(p.map(f64::from)))).2
    }
    fn facade_fixture(frame: &ChunkFrame, pack: &FeaturePack) -> BuildingStamp {
        let overlay = tier0::build_overlay(
            frame.addr.cell,
            frame,
            pack,
            &|_, _| 100.0,
            0,
            None,
            None,
            Some(&|_| true),
        );
        let mut stamp = overlay.source_buildings[0];
        stamp.design = crate::regional_architecture::BuildingDesign::default();
        stamp.facade_style = crate::tier0::FacadeStyle::LowRise;
        stamp.tall_style = None;
        stamp.highrise = None;
        stamp
    }

    fn glazing(mesh: &MeshBuffers) -> Vec<SurfaceVertex> {
        mesh.vertices
            .iter()
            .copied()
            .filter(|vertex| {
                matches!(
                    ((vertex.face_ao >> 5) & 255) as u8,
                    crate::material::WINDOW | crate::material::LIT_WINDOW
                )
            })
            .collect()
    }

    fn is_roof_vertex(vertex: &SurfaceVertex) -> bool {
        vertex.face_ao & 7 == 2
            && matches!(
                ((vertex.face_ao >> 5) & 255) as u8,
                crate::material::ROOF
                    | crate::material::TILE_ROOF
                    | crate::material::SLATE_ROOF
                    | crate::material::METAL_ROOF
                    | crate::material::SHINGLE_ROOF
                    | crate::material::THATCH_ROOF
                    | crate::material::FLAT_ROOF
            )
    }

    #[test]
    fn off_cell_source_id_fragment_does_not_poison_in_cell_completeness() {
        for level in [11, 14, 17] {
            let frame = frame(level);
            let mut baseline_pack = pack_with(vec![square(-6.0, 6.0)], 12.0);
            baseline_pack.buildings[0].roof_shape = Some(RoofShape::Flat);
            let prepared_cap = datum_cover(&frame, &baseline_pack);
            assert!(!prepared_cap.is_empty());
            let ground = |lat, lon, hint| {
                assert_eq!(hint, crate::datum_pitch_m(lat) as f32);
                assert!(
                    prepared_cap.contains(lat, lon),
                    "a noncontributing outside fragment requested unprepared ground"
                );
                Some(100.0)
            };
            let baseline = generate_test(&frame, &baseline_pack, &ground);
            assert!(baseline.complete);

            for outside_first in [false, true] {
                let mut pack = pack_with(vec![square(-6.0, 6.0)], 12.0);
                pack.buildings[0].roof_shape = Some(RoofShape::Flat);
                // Stable source IDs legitimately repeat across MVT fragments.
                // The distant fragment is valid, but cannot touch this cell or
                // affect the admitted fragment's overlap/architectural neighbors.
                let east = (frame.grid_scale.x * N as f64 * 1.5).max(200.0) as f32;
                let mut outside = pack_with(
                    vec![
                        square(-6.0, 6.0)
                            .into_iter()
                            .map(|[x, y]| [x + east, y])
                            .collect(),
                    ],
                    12.0,
                )
                .buildings
                .pop()
                .unwrap();
                outside.roof_shape = Some(RoofShape::Flat);
                assert_eq!(outside.seed, pack.buildings[0].seed);
                assert_eq!(outside.kind, pack.buildings[0].kind);
                if outside_first {
                    pack.buildings.insert(0, outside);
                } else {
                    pack.buildings.push(outside);
                }
                let overlay = tier0::build_overlay(
                    frame.addr.cell,
                    &frame,
                    &pack,
                    &|_, _| 100.0,
                    0,
                    None,
                    None,
                    Some(&|_| true),
                );
                assert_eq!(overlay.source_buildings.len(), 1);
                assert_eq!(
                    datum_cover(&frame, &pack).footprints(),
                    prepared_cap.footprints()
                );
                let actual = generate_test(&frame, &pack, &ground);
                assert_eq!(actual.mesh.vertices, baseline.mesh.vertices);
                assert_eq!(actual.mesh.indices, baseline.mesh.indices);
                assert_eq!(actual.collision, baseline.collision);
                assert!(
                    actual.complete,
                    "L{level}, outside_first={outside_first}: an outside fragment sharing \
                     the admitted source ID must not make the in-cell product incomplete"
                );
            }
        }
    }

    #[test]
    fn admitted_fragments_sharing_an_art_seed_keep_their_own_source_stamps() {
        let frame = frame(14);
        let mut pack = pack_with(
            vec![vec![[-16.0, -4.0], [-8.0, -4.0], [-8.0, 4.0], [-16.0, 4.0]]],
            12.0,
        );
        pack.buildings[0].roof_shape = Some(RoofShape::Flat);
        let mut other = pack_with(
            vec![vec![[8.0, -4.0], [16.0, -4.0], [16.0, 4.0], [8.0, 4.0]]],
            30.0,
        )
        .buildings
        .pop()
        .unwrap();
        other.roof_shape = Some(RoofShape::Gambrel);
        pack.buildings.push(other);
        let overlay = tier0::build_overlay(
            frame.addr.cell,
            &frame,
            &pack,
            &|_, _| 100.0,
            0,
            None,
            None,
            Some(&|_| true),
        );
        assert_eq!(overlay.source_buildings.len(), 2);
        let plan = prepare_envelopes(
            &frame,
            &pack,
            &overlay,
            &[u32::MAX; N],
            &|_, _, _| Some(100.0),
            &|_, _, _| None,
        )
        .unwrap();
        assert!(plan.complete);
        for (i, source) in pack.buildings.iter().enumerate() {
            let stamp = plan.stamps.get(&i).unwrap();
            assert_eq!(stamp.source_index, Some(i));
            assert_eq!(stamp.seed, 42);
            assert_eq!(stamp.height, source.height_m);
            assert_eq!(stamp.roof_shape, source.roof_shape);
            assert!(stamp.night_eligible);
        }
        assert!(generate_test(&frame, &pack, &|_, _, _| Some(100.0)).complete);
    }

    #[test]
    fn aggregate_source_components_keep_every_admitted_envelope_and_hole_across_cells() {
        run_aggregate_component_fixture(false);
    }

    #[test]
    #[ignore = "local aggregate-component geometry and generation diagnostic"]
    fn aggregate_component_generation_benchmark() {
        run_aggregate_component_fixture(true);
    }

    fn run_aggregate_component_fixture(report: bool) {
        for level in [11, 14] {
            let left = frame(level);
            let mut next = left.addr;
            next.cell.i += 1;
            let right = ChunkFrame::of(next);
            let (anchor_lat, anchor_lon) = left.column_latlon(1.0, 0.5);
            let mut seen = [0_usize; 256];
            for frame in [&left, &right] {
                let (lat0, lon0) = frame.column_latlon(0.5, 0.5);
                let project = |east: f64, north: f64| {
                    let lat = anchor_lat + north / MEAN_RADIUS;
                    let lon = anchor_lon + east / (MEAN_RADIUS * anchor_lat.cos());
                    [
                        ((lon - lon0) * MEAN_RADIUS * lat0.cos()) as f32,
                        ((lat - lat0) * MEAN_RADIUS) as f32,
                    ]
                };
                let mut pack = FeaturePack::empty();
                for component in 0..256 {
                    let east = ((component % 16) as f64 - 7.5) * 24.0;
                    let north = ((component / 16) as f64 - 7.5) * 24.0;
                    let ring = |radius: f32| {
                        square(-radius, radius)
                            .into_iter()
                            .map(|[x, y]| project(east + f64::from(x), north + f64::from(y)))
                            .collect()
                    };
                    let mut rings = vec![ring(4.0)];
                    if component % 5 == 0 {
                        rings.push(ring(1.0));
                    }
                    pack.buildings.push(Building {
                        location_rad: Some([
                            anchor_lat + north / MEAN_RADIUS,
                            anchor_lon + east / (MEAN_RADIUS * anchor_lat.cos()),
                        ]),
                        rings,
                        height_m: 8.0,
                        min_height_m: 0.0,
                        seed: 42,
                        kind: BuildingKind::Building,
                        roof_shape: Some(RoofShape::Flat),
                        use_kind: Default::default(),
                    });
                }
                let cap = datum_cover(frame, &pack);
                assert!(!cap.is_empty());
                let ground = |lat, lon, hint| {
                    assert_eq!(hint, crate::datum_pitch_m(lat) as f32);
                    assert!(cap.contains(lat, lon));
                    Some(100.0)
                };
                let overlay = tier0::build_overlay(
                    frame.addr.cell,
                    frame,
                    &pack,
                    &|_, _| 100.0,
                    0,
                    None,
                    None,
                    Some(&|_| true),
                );
                let admitted: std::collections::HashSet<_> = overlay
                    .source_buildings
                    .iter()
                    .map(|stamp| stamp.source_index.unwrap())
                    .collect();
                assert_eq!(admitted.len(), 128);
                let plan = prepare_envelopes(
                    frame,
                    &pack,
                    &overlay,
                    &[u32::MAX; N],
                    &ground,
                    &|_, _, _| None,
                )
                .unwrap();
                assert!(plan.complete);
                for i in 0..256 {
                    assert_eq!(plan.datums[i].is_some(), admitted.contains(&i));
                    assert_eq!(plan.stamps.contains_key(&i), admitted.contains(&i));
                }
                let started = vearth_platform::time::Instant::now();
                let output = generate(
                    frame,
                    &pack,
                    &overlay,
                    &[u32::MAX; N],
                    &ground,
                    &|_, _, _| None,
                )
                .unwrap();
                let generation_ms = started.elapsed().as_secs_f64() * 1000.0;
                assert!(
                    output.complete,
                    "L{level} component output must be cacheable"
                );
                let roof: Vec<[DVec2; 3]> = output
                    .mesh
                    .indices
                    .chunks_exact(3)
                    .filter(|triangle| {
                        triangle
                            .iter()
                            .all(|&i| is_roof_vertex(&output.mesh.vertices[i as usize]))
                    })
                    .map(|triangle| {
                        std::array::from_fn(|i| {
                            source_position(frame, output.mesh.vertices[triangle[i] as usize].pos)
                                .truncate()
                        })
                    })
                    .collect();
                if report {
                    let windows = output
                        .mesh
                        .indices
                        .chunks_exact(3)
                        .filter(|triangle| {
                            matches!(
                                ((output.mesh.vertices[triangle[0] as usize].face_ao >> 5) & 255)
                                    as u8,
                                crate::material::WINDOW | crate::material::LIT_WINDOW
                            )
                        })
                        .count();
                    eprintln!(
                        "aggregate L{level} i{}: {} admitted, {generation_ms:.3}ms, mandatory={}tris/{}bytes (roof={} window={windows}), optional={}tris/{}bytes, grid={:.3}m",
                        frame.addr.cell.i,
                        admitted.len(),
                        output.mesh.indices.len() / 3,
                        crate::built_surface::mesh_bytes(&output.mesh),
                        roof.len(),
                        output
                            .details
                            .iter()
                            .map(|detail| detail.mesh.indices.len() / 3)
                            .sum::<usize>(),
                        output
                            .details
                            .iter()
                            .map(|detail| crate::built_surface::mesh_bytes(&detail.mesh))
                            .sum::<usize>(),
                        frame.grid_scale.x.min(frame.grid_scale.z)
                    );
                    if level == 14 && frame.addr.cell.i == left.addr.cell.i {
                        // Keep this exact 128-house source/clip arrangement fixed
                        // while changing only its architectural resolution policy.
                        let mut control = prepare_envelopes(
                            frame,
                            &pack,
                            &overlay,
                            &[u32::MAX; N],
                            &ground,
                            &|_, _, _| None,
                        )
                        .unwrap();
                        control.facade_resolution = FacadeResolution::FULL;
                        let started = vearth_platform::time::Instant::now();
                        let full = generate_prepared_envelopes::<true>(
                            frame,
                            &pack,
                            &overlay,
                            &[u32::MAX; N],
                            &ground,
                            &|_, _, _| None,
                            false,
                            usize::MAX,
                            &control,
                        )
                        .unwrap();
                        let full_ms = started.elapsed().as_secs_f64() * 1000.0;
                        let full_detail: usize =
                            full.details.iter().map(|d| d.mesh.indices.len()).sum();
                        eprintln!(
                            "128-house fixed-source FULL: {full_ms:.3}ms mandatory={} optional={} triangles",
                            full.mesh.indices.len() / 3,
                            full_detail / 3
                        );
                        let mut previous = 0;
                        for policy_level in [11, 14, 15, 16, 17, 19] {
                            control.facade_resolution = FacadeResolution::for_frame(
                                &crate::building_surface::tests::frame(policy_level),
                            );
                            let started = vearth_platform::time::Instant::now();
                            let actual = generate_prepared_envelopes::<true>(
                                frame,
                                &pack,
                                &overlay,
                                &[u32::MAX; N],
                                &ground,
                                &|_, _, _| None,
                                false,
                                usize::MAX,
                                &control,
                            )
                            .unwrap();
                            let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
                            let detail: usize =
                                actual.details.iter().map(|d| d.mesh.indices.len()).sum();
                            let count = actual.mesh.indices.len() + detail;
                            assert!(actual.complete && full.complete);
                            assert_glazing_is_source_subsequence(&actual.mesh, &full.mesh);
                            assert_eq!(
                                material_triangles(&actual.mesh, is_roof_vertex),
                                material_triangles(&full.mesh, is_roof_vertex)
                            );
                            assert_eq!(actual.collision, full.collision);
                            assert!(
                                count >= previous && count <= full.mesh.indices.len() + full_detail
                            );
                            previous = count;
                            if policy_level >= 17 {
                                assert_eq!(actual.mesh.vertices, full.mesh.vertices);
                                assert_eq!(actual.mesh.indices, full.mesh.indices);
                            }
                            eprintln!(
                                "128-house fixed-source L{policy_level} policy: {elapsed_ms:.3}ms mandatory={} optional={} triangles, mesh={}bytes",
                                actual.mesh.indices.len() / 3,
                                detail / 3,
                                crate::built_surface::mesh_bytes(&actual.mesh)
                            );
                        }
                    }
                }
                let covered = |point: DVec2| {
                    roof.iter().any(|triangle| {
                        if (triangle[1] - triangle[0])
                            .perp_dot(triangle[2] - triangle[0])
                            .abs()
                            < 1.0e-8
                        {
                            return false;
                        }
                        let sides: [f64; 3] = std::array::from_fn(|i| {
                            (triangle[(i + 1) % 3] - triangle[i]).perp_dot(point - triangle[i])
                        });
                        sides.iter().all(|&side| side >= -1.0e-6)
                            || sides.iter().all(|&side| side <= 1.0e-6)
                    })
                };
                for &i in &admitted {
                    seen[i] += 1;
                    let (lo, hi) = bounds(&pack.buildings[i].rings).unwrap();
                    let center = (lo + hi) * 0.5;
                    assert!(
                        covered(center + DVec2::new(2.5, 0.0)),
                        "L{level}: component{i} lost its roof"
                    );
                    if i % 5 == 0 {
                        assert!(
                            !covered(center),
                            "L{level}: component{i} lost its courtyard"
                        );
                    }
                }
            }
            assert!(
                seen.iter().all(|&count| count == 1),
                "L{level}: a source component was dropped or duplicated across cell owners"
            );
        }
    }

    #[test]
    fn footprint_constructor_classifies_existing_rejection_boundaries() {
        let valid = vec![square(-1.0, 1.0)];
        let short = vec![vec![[0.0, 0.0], [1.0, 1.0]]];
        let nonfinite = vec![vec![[0.0, 0.0], [1.0, 1.0], [0.0, f32::NAN]]];
        assert!(matches!(
            Footprint::checked_with_limits(vec![&short], vec![], 512, 8192),
            Err(FootprintFailure::ShortRing)
        ));
        assert!(matches!(
            Footprint::checked_with_limits(vec![&nonfinite], vec![], 512, 8192),
            Err(FootprintFailure::Nonfinite)
        ));
        assert!(matches!(
            Footprint::checked_with_limits(vec![&valid], vec![], 3, 8192),
            Err(FootprintFailure::EdgeBudget { .. })
        ));
        assert!(matches!(
            Footprint::checked_with_limits(vec![], vec![], 512, 8192),
            Err(FootprintFailure::Empty)
        ));
        assert!(Footprint::checked_with_limits(vec![&valid], vec![], 4, 8192).is_ok());
    }

    #[test]
    fn detailed_building_outlines_beyond_the_general_edge_budget_stay_complete() {
        let pack = pack_with(
            vec![arrangement_circle(12.0, MAX_FOOTPRINT_EDGES + 88)],
            15.0,
        );
        assert!(Footprint::new(vec![&pack.buildings[0].rings], vec![]).is_none());
        assert!(datum_samples(&pack.buildings[0]).is_some());
        for level in [15, 16] {
            let result = generate_test(&frame(level), &pack, &|_, _, _| Some(100.0));
            assert!(result.complete, "L{level}");
            assert!(!result.mesh.vertices.is_empty(), "L{level}");
        }
    }

    fn material_triangles(
        mesh: &MeshBuffers,
        keep: impl Fn(&SurfaceVertex) -> bool,
    ) -> Vec<[SurfaceVertex; 3]> {
        mesh.indices
            .chunks_exact(3)
            .filter_map(|triangle| {
                let vertices = std::array::from_fn(|i| mesh.vertices[triangle[i] as usize]);
                keep(&vertices[0]).then_some(vertices)
            })
            .collect()
    }

    fn glazing_triangles(mesh: &MeshBuffers) -> Vec<[SurfaceVertex; 3]> {
        material_triangles(mesh, |vertex| {
            matches!(
                ((vertex.face_ao >> 5) & 255) as u8,
                crate::material::WINDOW | crate::material::LIT_WINDOW
            )
        })
    }

    fn assert_glazing_is_source_subsequence(mesh: &MeshBuffers, full: &MeshBuffers) {
        let mut source = glazing_triangles(full).into_iter();
        for pane_triangle in glazing_triangles(mesh) {
            assert!(
                source.by_ref().any(|triangle| triangle == pane_triangle),
                "retained glazing changed source position, tint, room state or ordering"
            );
        }
    }

    #[test]
    fn simple_apertures_keep_glass_and_night_without_raised_surrounds() {
        for level in [14, 17, 19] {
            let frame = frame(level);
            let (lat, lon) = frame.column_latlon(0.5, 0.5);
            let projection = Projection {
                frame: &frame,
                lat,
                lon,
            };
            let units = std::cell::RefCell::default();
            let plane = FacadePlane {
                shadow_units: Some(&units),
                projection: &projection,
                origin: DVec2::ZERO,
                direction: DVec2::X,
                outward: -DVec2::Y,
            };
            for shape in [
                OpeningShape::Rectangular,
                OpeningShape::SegmentalArch,
                OpeningShape::RoundArch,
                OpeningShape::PointedArch,
            ] {
                for material in [crate::material::WINDOW, crate::material::LIT_WINDOW] {
                    let glass = SurfaceAppearance {
                        material,
                        tint: [66, 82, 91],
                    };
                    let trim = SurfaceAppearance {
                        material: crate::material::WALL,
                        tint: [190; 3],
                    };
                    let mut old = MeshBuffers::default();
                    plane.opening(
                        &mut old,
                        [-0.55, 0.55],
                        [100.0, 102.0],
                        0.025,
                        shape,
                        0.10,
                        glass,
                        trim,
                        true,
                    );
                    let mut mesh = MeshBuffers::default();
                    units.borrow_mut().clear();
                    plane.aperture(
                        &mut mesh,
                        [-0.55, 0.55],
                        [100.0, 102.0],
                        0.025,
                        shape,
                        glass,
                        true,
                    );
                    assert_eq!(glazing_triangles(&mesh), glazing_triangles(&old));
                    assert!(mesh.indices.len() * 3 < old.indices.len(), "{shape:?}");
                    assert_eq!(*units.borrow(), vec![0..mesh.indices.len()]);
                    assert!(
                        mesh.vertices
                            .iter()
                            .all(|v| v.pos.iter().all(|p| p.is_finite()))
                    );
                    for (depth, appearance) in [(0.30, glass), (0.025, trim)] {
                        units.borrow_mut().clear();
                        plane.aperture(
                            &mut MeshBuffers::default(),
                            [-0.55, 0.55],
                            [100.0, 102.0],
                            depth,
                            shape,
                            appearance,
                            true,
                        );
                        assert!(units.borrow().is_empty(), "oriels and doors remain casters");
                    }
                }
            }
        }
    }

    #[test]
    fn simple_facades_share_daytime_glass_tone_and_keep_stable_lit_rooms() {
        let frame = frame(17);
        let pack = pack_with(vec![square(-12.0, 12.0)], 24.0);
        let rings = &pack.buildings[0].rings;
        let boundary = Footprint::new(vec![rings], Vec::new()).unwrap().boundary();
        let (lat, lon) = frame.column_latlon(0.5, 0.5);
        let projection = Projection {
            frame: &frame,
            lat,
            lon,
        };
        let context = FacadeContext::from_sources(rings, &[], &[], 100.0);
        for &style in crate::architecture_styles::ARCHITECTURE_STYLES {
            let mut stamp = facade_fixture(&frame, &pack);
            stamp.design.style = style;
            stamp.design.expression = 255;
            let tone = crate::facade_window_color(&stamp, stamp.seed);
            let render = |night_eligible| {
                let mut stamp = stamp;
                stamp.night_eligible = night_eligible;
                let mut mesh = MeshBuffers::default();
                for &edge in &boundary {
                    emit_wall_with_resolution(
                        &projection,
                        &mut mesh,
                        &mut MeshBuffers::default(),
                        edge,
                        99.8,
                        100.0,
                        124.0,
                        &stamp,
                        rings,
                        &context,
                        false,
                        FacadeResolution::FULL,
                    );
                }
                glazing_triangles(&mesh)
            };
            let lit = render(true);
            let unlit = render(false);
            assert!(!lit.is_empty());
            assert_eq!(lit.len(), unlit.len(), "{style:?}: room coverage changed");
            assert!(
                lit.iter()
                    .flatten()
                    .any(|v| (v.face_ao >> 5) & 255 == u32::from(crate::material::LIT_WINDOW))
            );
            for (lit, unlit) in lit.iter().flatten().zip(unlit.iter().flatten()) {
                assert_eq!(lit.pos, unlit.pos);
                assert_eq!(lit.rgba[..3], tone, "{style:?}: per-room daytime tint");
                assert_eq!(lit.rgba, unlit.rgba);
                assert_eq!(
                    (unlit.face_ao >> 5) & 255,
                    u32::from(crate::material::WINDOW)
                );
            }
            assert_eq!(
                lit,
                render(true),
                "{style:?}: night assignment changed on rebuild"
            );
        }
    }

    #[test]
    fn facade_resolution_scales_continuously_and_requires_two_pixel_panes() {
        let mut previous = f64::INFINITY;
        for level in 11..=19 {
            let resolution = FacadeResolution::for_frame(&frame(level));
            assert!(resolution.feature_m > 0.0 && resolution.feature_m < previous);
            previous = resolution.feature_m;
            assert!(!resolution.resolves(0.0));
            assert!(!resolution.resolves(resolution.feature_m));
            assert!(resolution.resolves(resolution.feature_m * 1.001));
            let pixel_m = resolution.feature_m;
            assert!(!resolution.resolves_glazing(pixel_m, pixel_m * 4.0));
            assert!(!resolution.resolves_glazing(pixel_m * 4.0, pixel_m));
            assert!(!resolution.resolves_glazing(pixel_m * 1.999, pixel_m * 2.0));
            assert!(!resolution.resolves_glazing(pixel_m * 2.0, pixel_m * 1.999));
            assert!(resolution.resolves_glazing(pixel_m * 2.0, pixel_m * 2.0));
            assert!(resolution.resolves_glazing(pixel_m * 2.001, pixel_m * 2.0));
        }
        let medium = FacadeResolution::for_frame(&frame(16));
        assert!(!medium.resolves(0.08));
        assert!(!medium.resolves(0.55));
        assert!(!medium.resolves(0.86));
        assert!(medium.resolves(2.0));
        assert!(!FacadeResolution::for_frame(&frame(17)).resolves_glazing(1.2, 1.5));
        for level in [18, 19] {
            let near = FacadeResolution::for_frame(&frame(level));
            assert!(near.resolves_glazing(1.2, 1.5));
            assert!(near.resolves(0.55));
        }
        assert!(FacadeResolution::FULL.resolves_glazing(0.001, 0.001));
    }

    #[test]
    fn glazing_resolution_keeps_large_shopfronts_shutters_and_projecting_bay_bodies() {
        let frame = frame(14);
        let pack = pack_with(vec![square(-12.0, 12.0)], 18.0);
        let rings = &pack.buildings[0].rings;
        let (lat, lon) = frame.column_latlon(0.5, 0.5);
        let projection = Projection {
            frame: &frame,
            lat,
            lon,
        };
        let plane = FacadePlane {
            shadow_units: None,
            projection: &projection,
            origin: DVec2::new(-12.0, -12.0),
            direction: DVec2::X,
            outward: -DVec2::Y,
        };
        let context = FacadeContext::from_sources(rings, &[], &[], 100.0);
        let mut stamp = facade_fixture(&frame, &pack);
        stamp.design.use_kind = vearth_data::vectors::BuildingUse::Commercial;
        let mut style = crate::architecture_styles::ArchitectureStyle::DutchGable.profile();
        style.ground_storey = GroundStorey::Shopfront;
        style.opening = OpeningShape::Rectangular;
        style.shutters = ShutterKind::None;
        style.projection = ProjectionKind::None;
        let glass = SurfaceAppearance {
            material: crate::material::LIT_WINDOW,
            tint: [40, 80, 120],
        };
        let resolution = FacadeResolution { feature_m: 1.0 };
        let render = |style, storey, resolution| {
            let mut mesh = MeshBuffers::default();
            let floor = 100.0 + f64::from(storey) * 3.2;
            emit_style_bay(
                &plane,
                &mut mesh,
                &context,
                [10.0, 11.2],
                [floor + 0.6, floor + 2.1],
                floor,
                3.2,
                24.0,
                4.0,
                storey,
                ((stamp.seed >> 9) & 1) as u32,
                true,
                style,
                &stamp,
                glass,
                facade_trim(&stamp),
                true,
                resolution,
            );
            mesh
        };
        assert!(glazing_triangles(&render(style, 1, resolution)).is_empty());
        let shop = render(style, 0, resolution);
        assert!(!glazing_triangles(&shop).is_empty());
        assert_eq!(
            glazing_triangles(&shop),
            glazing_triangles(&render(style, 0, FacadeResolution::FULL))
        );
        assert!(
            glazing_triangles(&render(
                style,
                1,
                FacadeResolution::for_frame(&crate::building_surface::tests::frame(17)),
            ))
            .is_empty()
        );
        for level in [18, 19] {
            let near = render(
                style,
                1,
                FacadeResolution::for_frame(&crate::building_surface::tests::frame(level)),
            );
            let full = render(style, 1, FacadeResolution::FULL);
            let panes = glazing_triangles(&near);
            assert!(!panes.is_empty());
            assert!(
                panes
                    .iter()
                    .flatten()
                    .all(|v| ((v.face_ao >> 5) & 255) == u32::from(crate::material::LIT_WINDOW))
            );
            assert_eq!(panes, glazing_triangles(&full));
        }

        style.shutters = ShutterKind::Panel;
        style.projection = ProjectionKind::TimberBay;
        style.projection_m = 0.6;
        let projected = render(style, 1, resolution);
        let full = render(style, 1, FacadeResolution::FULL);
        assert!(glazing_triangles(&projected).is_empty());
        assert!(!glazing_triangles(&full).is_empty());
        let opaque = |mesh: &MeshBuffers| {
            material_triangles(mesh, |v| {
                !matches!(
                    ((v.face_ao >> 5) & 255) as u8,
                    crate::material::WINDOW | crate::material::LIT_WINDOW
                )
            })
        };
        assert!(!opaque(&projected).is_empty());
        assert_eq!(opaque(&projected), opaque(&full));
    }

    #[test]
    fn facade_resolution_preserves_retained_catalog_glazing_and_monotonic_complexity() {
        let frame = frame(14);
        let pack = pack_with(vec![square(-12.0, 12.0), square(-3.0, 3.0)], 18.0);
        let rings = &pack.buildings[0].rings;
        let boundary = Footprint::new(vec![rings], Vec::new()).unwrap().boundary();
        let (lat, lon) = frame.column_latlon(0.5, 0.5);
        let projection = Projection {
            frame: &frame,
            lat,
            lon,
        };
        let mut context = FacadeContext::from_sources(rings, &[], &[], 100.0);
        context.entrance = entrance_on_source_boundary(rings, &boundary, 100.0, &context, None);
        let mut reduced_styles = 0;
        for &style in crate::architecture_styles::ARCHITECTURE_STYLES {
            let mut stamp = facade_fixture(&frame, &pack);
            stamp.design.style = style;
            stamp.design.expression = 255;
            stamp.design.joinery = [54, 87, 118];
            stamp.design.use_kind = vearth_data::vectors::BuildingUse::Commercial;
            let render = |resolution, simple| {
                let mut mesh = MeshBuffers::default();
                let mut detail = MeshBuffers::default();
                for &edge in &boundary {
                    emit_wall_with_resolution(
                        &projection,
                        &mut mesh,
                        &mut detail,
                        edge,
                        99.8,
                        100.0,
                        118.0,
                        &stamp,
                        rings,
                        &context,
                        simple,
                        resolution,
                    );
                }
                (mesh, detail)
            };
            for simple in [false, true] {
                let (full, full_detail) = render(FacadeResolution::FULL, simple);
                let glass = glazing_triangles(&full);
                assert!(!glass.is_empty(), "{style:?}: missing aperture fixture");
                let mut previous = 0;
                for level in [11, 14, 15, 16, 17, 19] {
                    let resolution =
                        FacadeResolution::for_frame(&crate::building_surface::tests::frame(level));
                    let (mesh, detail) = render(resolution, simple);
                    assert_glazing_is_source_subsequence(&mesh, &full);
                    if level == 11 {
                        assert!(
                            glazing_triangles(&mesh).is_empty(),
                            "{style:?}: subpixel panes"
                        );
                    }
                    let count = mesh.indices.len() + detail.indices.len();
                    assert!(
                        count >= previous,
                        "{style:?} L{level}: non-monotonic detail"
                    );
                    previous = count;
                    assert!(count <= full.indices.len() + full_detail.indices.len());
                    // Resolved entrances and joinery retain their authored
                    // color after thin frame geometry vanishes.
                    let has_joinery = |mesh: &MeshBuffers| {
                        mesh.vertices
                            .iter()
                            .any(|v| v.rgba[..3] == stamp.design.joinery)
                    };
                    if level >= 16 && has_joinery(&full) {
                        assert!(has_joinery(&mesh), "{style:?} L{level}: lost joinery tone");
                    }
                    if level == 14 && mesh.indices.len() < full.indices.len() {
                        reduced_styles += 1;
                    }
                }
            }
        }
        assert!(
            reduced_styles > 0,
            "coarse policy must still simplify major projections"
        );
    }

    #[test]
    fn facade_resolution_keeps_large_generic_shades_and_balconies() {
        let frame = frame(18);
        let pack = pack_with(vec![square(-12.0, 12.0)], 18.0);
        let rings = &pack.buildings[0].rings;
        let boundary = Footprint::new(vec![rings], Vec::new()).unwrap().boundary();
        let (lat, lon) = frame.column_latlon(0.5, 0.5);
        let projection = Projection {
            frame: &frame,
            lat,
            lon,
        };
        let context = FacadeContext::from_sources(rings, &[], &[], 100.0);
        for family in [
            crate::regional_architecture::DetailFamily::Shaded,
            crate::regional_architecture::DetailFamily::Balcony,
        ] {
            let mut stamp = facade_fixture(&frame, &pack);
            stamp.design.detail = family;
            stamp.design.expression = 0;
            let render = |resolution| {
                let mut mesh = MeshBuffers::default();
                let mut detail = MeshBuffers::default();
                for &edge in &boundary {
                    emit_wall_with_resolution(
                        &projection,
                        &mut mesh,
                        &mut detail,
                        edge,
                        99.8,
                        100.0,
                        118.0,
                        &stamp,
                        rings,
                        &context,
                        false,
                        resolution,
                    );
                }
                (mesh, detail)
            };
            let (full, full_detail) = render(FacadeResolution::FULL);
            let (mesh, detail) = render(FacadeResolution::for_frame(&frame));
            assert_glazing_is_source_subsequence(&mesh, &full);
            let reach = |mesh: &MeshBuffers| {
                mesh.vertices
                    .iter()
                    .map(|v| {
                        let p = source_position(&frame, v.pos);
                        p.x.abs().max(p.y.abs()) - 12.0
                    })
                    .max_by(f64::total_cmp)
                    .unwrap_or(0.0)
            };
            assert!(reach(&full_detail) >= 0.50);
            assert_eq!(
                reach(&detail).to_bits(),
                reach(&full_detail).to_bits(),
                "{family:?} lost resolvable projection"
            );
        }
    }

    #[test]
    fn facade_resolution_keeps_near_and_coarse_source_roofs_and_collision() {
        use crate::architecture_styles::ArchitectureStyle;
        for level in [14, 17, 19] {
            let frame = frame(level);
            let mut pack = pack_with(vec![square(-7.0, 7.0), square(-2.0, 2.0)], 18.0);
            pack.buildings[0].roof_shape = Some(RoofShape::Gabled);
            pack.buildings.push(Building {
                rings: vec![vec![[2.5, -6.0], [6.0, -6.0], [6.0, 6.0], [2.5, 6.0]]],
                location_rad: pack.buildings[0].location_rad,
                height_m: 22.0,
                min_height_m: 8.0,
                seed: 43,
                kind: BuildingKind::Part,
                roof_shape: Some(RoofShape::Flat),
                use_kind: pack.buildings[0].use_kind,
            });
            for style in [
                ArchitectureStyle::DutchGable,
                ArchitectureStyle::JaisalmerHaveli,
                ArchitectureStyle::SanaaniTower,
                ArchitectureStyle::StraitsShophouse,
                ArchitectureStyle::AlpineGallery,
                ArchitectureStyle::HejaziRoshan,
            ] {
                let mut overlay = tier0::build_overlay(
                    frame.addr.cell,
                    &frame,
                    &pack,
                    &|_, _| 100.0,
                    0,
                    None,
                    None,
                    Some(&|_| true),
                );
                overlay.source_buildings[0].design.style = style;
                overlay.source_buildings[0].design.expression = 255;
                let ground = |_, _, _| Some(100.0);
                let surface = |_, _, _| None;
                let eligible = [u32::MAX; N];
                let mut plan =
                    prepare_envelopes(&frame, &pack, &overlay, &eligible, &ground, &surface)
                        .unwrap();
                let actual = generate_prepared_envelopes::<true>(
                    &frame,
                    &pack,
                    &overlay,
                    &eligible,
                    &ground,
                    &surface,
                    false,
                    usize::MAX,
                    &plan,
                )
                .unwrap();
                plan.facade_resolution = FacadeResolution::FULL;
                let full = generate_prepared_envelopes::<true>(
                    &frame,
                    &pack,
                    &overlay,
                    &eligible,
                    &ground,
                    &surface,
                    false,
                    usize::MAX,
                    &plan,
                )
                .unwrap();
                assert!(actual.complete && full.complete);
                assert_eq!(actual.collision, full.collision);
                assert_eq!(
                    material_triangles(&actual.mesh, is_roof_vertex),
                    material_triangles(&full.mesh, is_roof_vertex),
                    "{style:?} L{level}: source roof changed"
                );
                assert_glazing_is_source_subsequence(&actual.mesh, &full.mesh);
                let indices = |surface: &BuildingSurfaceData| {
                    surface.mesh.indices.len()
                        + surface
                            .details
                            .iter()
                            .map(|detail| detail.mesh.indices.len())
                            .sum::<usize>()
                };
                assert!(indices(&actual) <= indices(&full));
            }
        }
    }

    #[test]
    fn architectural_openings_and_projections_survive_optional_detail_saturation() {
        use crate::architecture_styles::ArchitectureStyle;
        let frame = frame(17);
        let pack = pack_with(vec![square(-12.0, 12.0)], 15.0);
        let rings = &pack.buildings[0].rings;
        let boundary = Footprint::new(vec![rings], Vec::new()).unwrap().boundary();
        let (lat, lon) = frame.column_latlon(0.5, 0.5);
        let projection = Projection {
            frame: &frame,
            lat,
            lon,
        };
        let mut context = FacadeContext::from_sources(rings, &[], &[], 100.0);
        context.entrance = entrance_on_source_boundary(rings, &boundary, 100.0, &context, None);
        let mut silhouettes = std::collections::HashSet::new();
        for style in [
            ArchitectureStyle::DutchGable,
            ArchitectureStyle::JaisalmerHaveli,
            ArchitectureStyle::SanaaniTower,
            ArchitectureStyle::StraitsShophouse,
            ArchitectureStyle::AlpineGallery,
            ArchitectureStyle::HejaziRoshan,
        ] {
            let mut stamp = facade_fixture(&frame, &pack);
            stamp.design.style = style;
            stamp.design.expression = 255;
            let render = |saturated: bool, simple: bool| {
                let mut mesh = MeshBuffers::default();
                let mut details = MeshBuffers::default();
                if saturated {
                    details.vertices.resize(
                        MAX_FACADE_DETAIL_VERTICES,
                        SurfaceVertex {
                            pos: [0.0; 3],
                            face_ao: 0,
                            rgba: [0; 4],
                        },
                    );
                }
                for edge in &boundary {
                    emit_wall_with_resolution(
                        &projection,
                        &mut mesh,
                        &mut details,
                        *edge,
                        99.8,
                        100.0,
                        115.0,
                        &stamp,
                        rings,
                        &context,
                        simple,
                        FacadeResolution::FULL,
                    );
                }
                (mesh, details)
            };
            let (full, _) = render(false, false);
            let (saturated, _) = render(true, false);
            let (simple, detail) = render(false, true);
            assert_eq!(
                full.vertices, saturated.vertices,
                "{style:?} lost architecture at the detail cap"
            );
            assert_eq!(
                glazing(&full),
                glazing(&simple),
                "{style:?} changed source apertures in simple mode"
            );
            assert!(detail.vertices.is_empty());
            assert!(glazing(&full).len() > 40, "{style:?} has no openings");
            assert!(
                full.vertices.iter().all(|v| {
                    let h = world_height(&frame, v.pos);
                    v.pos.iter().all(|v| v.is_finite()) && (99.78..115.02).contains(&h)
                }),
                "{style:?} escaped the source storey bounds"
            );
            silhouettes.insert(
                full.vertices
                    .iter()
                    .map(|v| v.pos.map(f32::to_bits))
                    .collect::<Vec<_>>(),
            );
        }
        assert_eq!(
            silhouettes.len(),
            6,
            "styles must change geometry, not only tint"
        );
    }

    #[test]
    fn shaped_roofs_preserve_eave_and_reported_top_instead_of_becoming_flat() {
        let rings = vec![vec![[-6.0, -12.0], [6.0, -12.0], [6.0, 12.0], [-6.0, 12.0]]];
        for shape in [RoofShape::Mansard, RoofShape::Gambrel, RoofShape::Skillion] {
            let roof = RoofField::new(&rings, Some(shape), 110.0, 114.0, 0.8);
            let heights: Vec<_> = (0..=24)
                .map(|i| roof.height(DVec2::new(-6.0 + i as f64 * 0.5, 0.0)))
                .collect();
            assert!(heights.iter().all(|&h| (110.0..=114.0).contains(&h)));
            assert!(
                heights.iter().any(|&h| h <= 110.001),
                "{shape:?} lost its source eave"
            );
            assert!(
                heights.iter().any(|&h| h >= 113.999),
                "{shape:?} lost its reported top"
            );
            assert!(
                heights.iter().any(|&h| h > 110.5 && h < 113.5),
                "{shape:?} became a flat cap"
            );
        }
    }

    #[test]
    fn gabled_frontage_uses_the_short_exposed_end_and_avoids_courtyards() {
        let rings = vec![
            vec![[-4.0, -14.0], [4.0, -14.0], [4.0, 14.0], [-4.0, 14.0]],
            square(-2.0, 2.0),
        ];
        let domain = Footprint::new(vec![&rings], Vec::new()).unwrap();
        let context = FacadeContext::from_sources(&rings, &[], &[], 100.0);
        let entrance = entrance_on_source_boundary(
            &rings,
            &domain.boundary(),
            100.0,
            &context,
            Some(DVec2::Y),
        )
        .unwrap();
        assert!(
            entrance.0 == 0 || entrance.0 == 2,
            "short gable front expected, got {entrance:?}"
        );
    }

    #[test]
    fn projection_clearance_detects_a_thin_neighbor_between_sample_points() {
        let owner = vec![vec![[-4.0, 0.0], [4.0, 0.0], [4.0, 4.0], [-4.0, 4.0]]];
        let obstacle = vec![vec![
            [0.26, -0.55],
            [0.29, -0.55],
            [0.29, -0.50],
            [0.26, -0.50],
        ]];
        let frame = frame(17);
        let (lat, lon) = frame.column_latlon(0.5, 0.5);
        let projection = Projection {
            frame: &frame,
            lat,
            lon,
        };
        let plane = FacadePlane {
            shadow_units: None,
            projection: &projection,
            origin: DVec2::ZERO,
            direction: DVec2::X,
            outward: -DVec2::Y,
        };
        let mut context = FacadeContext {
            party_walls: Vec::new(),
            opaque_core: None,
            shadow_walls: std::cell::RefCell::default(),
            owner: owner.clone(),
            owner_bounds: bounds(&owner),
            owner_query: PolygonQuery::new(&owner),
            blockers: vec![FacadeBlocker {
                rings: obstacle.clone(),
                bounds: bounds(&obstacle),
                query: PolygonQuery::new(&obstacle),
                bottom: 103.0,
                top: 104.0,
            }],
            entrance: None,
        };
        assert!(!plane.clear(&context, [0.0, 2.0], [100.0, 110.0], 1.0));
        assert!(plane.clear(&context, [0.0, 2.0], [105.0, 110.0], 1.0));
        context.blockers.clear();
        assert!(plane.clear(&context, [0.0, 2.0], [100.0, 110.0], 1.0));
    }

    #[test]
    fn eyebrow_dormers_are_bounded_resolved_and_outward_for_both_windings() {
        let frame = frame(19);
        let (lat, lon) = frame.column_latlon(0.5, 0.5);
        let projection = Projection {
            frame: &frame,
            lat,
            lon,
        };
        for reverse in [false, true] {
            let mut ring = vec![[-9.0, -5.0], [9.0, -5.0], [9.0, 5.0], [-9.0, 5.0]];
            if reverse {
                ring.reverse();
            }
            let pack = pack_with(vec![ring], 8.0);
            let source = &pack.buildings[0];
            let domain = Footprint::new(vec![&source.rings], vec![]).unwrap();
            let roof = RoofField::new(&source.rings, Some(RoofShape::Hipped), 104.0, 108.0, 0.7);
            let mut stamp = facade_fixture(&frame, &pack);
            stamp.design.style = crate::architecture_styles::ArchitectureStyle::SibiuEyebrow;
            stamp.design.expression = 255;
            let covering = SurfaceAppearance {
                material: crate::material::TILE_ROOF,
                tint: [130, 80, 60],
            };
            let render = |resolution| {
                let mut details = MeshBuffers::default();
                emit_eyebrow_dormers(
                    &projection,
                    &mut details,
                    &roof,
                    &domain,
                    source,
                    &stamp,
                    covering,
                    resolution,
                );
                details
            };
            let details = render(FacadeResolution::FULL);
            assert!(!glazing(&details).is_empty());
            assert!(details.vertices.len() <= 4 * 96);
            assert!(
                render(FacadeResolution { feature_m: 0.4 })
                    .vertices
                    .is_empty()
            );
            for triangle in details.indices.chunks_exact(3) {
                let p: [DVec3; 3] = std::array::from_fn(|i| {
                    frame.local_to_ecef(DVec3::from_array(
                        details.vertices[triangle[i] as usize].pos.map(f64::from),
                    ))
                });
                for &i in triangle {
                    let h = world_height(&frame, details.vertices[i as usize].pos);
                    assert!((103.99..=108.01).contains(&h));
                }
                if ((details.vertices[triangle[0] as usize].face_ao >> 5) & 255)
                    == u32::from(crate::material::WINDOW)
                {
                    let center = p.iter().copied().sum::<DVec3>() / 3.0;
                    let (clat, clon, _) = ellipsoid::ecef_to_geodetic(center);
                    let outward = center - projection.world_point(DVec2::ZERO, 106.0);
                    let normal = (p[1] - p[0]).cross(p[2] - p[0]);
                    let up =
                        ellipsoid::geodetic_normal(ellipsoid::geodetic_to_ecef(clat, clon, 0.0));
                    assert!(normal.dot(outward - up * outward.dot(up)) > 0.0);
                }
            }
        }
    }

    #[test]
    fn wall_finish_variant_changes_only_envelope_payload_not_trim_or_geometry() {
        let frame = frame(19);
        let pack = pack_with(vec![square(-6.0, 6.0)], 8.0);
        let rings = &pack.buildings[0].rings;
        let (lat, lon) = frame.column_latlon(0.5, 0.5);
        let projection = Projection {
            frame: &frame,
            lat,
            lon,
        };
        let context = FacadeContext::from_sources(rings, &[], &[], 100.0);
        let mut stamp = facade_fixture(&frame, &pack);
        stamp.design.wall_material = crate::material::TIMBER_WALL;
        stamp.design.wall = [151, 42, 38];
        stamp.design.detail = crate::regional_architecture::DetailFamily::Shaded;
        let render = |variant| {
            let mut stamp = stamp;
            stamp.design.wall_variant = variant;
            let mut mesh = MeshBuffers::default();
            let mut details = MeshBuffers::default();
            emit_wall_with_resolution(
                &projection,
                &mut mesh,
                &mut details,
                [DVec2::new(-6.0, -6.0), DVec2::new(6.0, -6.0)],
                99.8,
                100.0,
                108.0,
                &stamp,
                rings,
                &context,
                false,
                FacadeResolution::FULL,
            );
            (mesh, details)
        };
        let (before, before_details) = render(0);
        let (after, after_details) = render(1);
        assert_eq!(before.indices, after.indices);
        assert_eq!(before.vertices.len(), after.vertices.len());
        let mut changed = 0;
        for (a, b) in before.vertices.iter().zip(&after.vertices) {
            assert_eq!(a.pos, b.pos);
            assert_eq!(a.rgba, b.rgba);
            assert_eq!(a.face_ao, b.face_ao & !(3 << 13));
            if a.face_ao != b.face_ao {
                assert_eq!(
                    (b.face_ao >> 5) & 255,
                    u32::from(crate::material::TIMBER_WALL)
                );
                changed += 1;
            }
        }
        assert!(changed >= 6, "main wall must carry the selected finish");
        assert!(
            !before_details.vertices.is_empty(),
            "fixture must exercise projected trim"
        );
        assert_eq!(before_details.vertices, after_details.vertices);
        assert_eq!(before_details.indices, after_details.indices);
    }

    #[test]
    fn namako_finish_is_confined_to_the_lower_source_wall_and_simplifies() {
        let frame = frame(19);
        let pack = pack_with(vec![square(-6.0, 6.0)], 8.0);
        let rings = &pack.buildings[0].rings;
        let (lat, lon) = frame.column_latlon(0.5, 0.5);
        let projection = Projection {
            frame: &frame,
            lat,
            lon,
        };
        let context = FacadeContext::from_sources(rings, &[], &[], 100.0);
        let mut stamp = facade_fixture(&frame, &pack);
        stamp.design.style = crate::architecture_styles::ArchitectureStyle::KurashikiBikan;
        stamp.design.expression = 255;
        for simple in [false, true] {
            let mut mesh = MeshBuffers::default();
            emit_wall_with_resolution(
                &projection,
                &mut mesh,
                &mut MeshBuffers::default(),
                [DVec2::new(-6.0, -6.0), DVec2::new(6.0, -6.0)],
                99.8,
                100.0,
                108.0,
                &stamp,
                rings,
                &context,
                simple,
                FacadeResolution::FULL,
            );
            let lower: Vec<_> = mesh
                .vertices
                .iter()
                .filter(|v| ((v.face_ao >> 5) & 255) == u32::from(crate::material::NAMAKO_WALL))
                .collect();
            assert_eq!(lower.is_empty(), simple);
            for v in lower {
                assert!((99.79..=101.21).contains(&world_height(&frame, v.pos)));
            }
        }
    }

    #[test]
    fn trulli_roof_surface_survives_facade_expression_fade() {
        use crate::architecture_styles::ArchitectureStyle;
        let frame = frame(17);
        let mut pack = pack_with(vec![square(-6.0, 6.0)], 13.5);
        let mut stamp = facade_fixture(&frame, &pack);
        stamp.design.style = ArchitectureStyle::AlberobelloTrulli;
        stamp.roof_shape = Some(RoofShape::Pyramidal);
        let sampled_roof = |source: &Building, stamp: &BuildingStamp| {
            let mut roof = RoofField::new(&source.rings, stamp.roof_shape, 109.0, 113.5, 0.7);
            configure_roof_style(&mut roof, source, stamp);
            let samples: Vec<_> = (-5..=5)
                .flat_map(|x| (-5..=5).map(move |y| DVec2::new(f64::from(x), f64::from(y))))
                .map(|p| roof.height(p).to_bits())
                .collect();
            (roof.conical, samples)
        };
        stamp.design.expression = 255;
        let expected = sampled_roof(&pack.buildings[0], &stamp);
        assert!(expected.0);
        for expression in [0, 64, 95, 96, 170] {
            stamp.design.expression = expression;
            assert_eq!(sampled_roof(&pack.buildings[0], &stamp), expected);
        }
        pack.buildings[0].roof_shape = Some(RoofShape::Pyramidal);
        let observed = sampled_roof(&pack.buildings[0], &stamp);
        assert!(!observed.0);
        assert_ne!(observed.1, expected.1);
        pack.buildings[0].roof_shape = None;
        pack.buildings[0].kind = BuildingKind::RoofOnly;
        assert!(!sampled_roof(&pack.buildings[0], &stamp).0);
        pack.buildings[0].kind = BuildingKind::Building;
        stamp.lidar = true;
        assert!(!sampled_roof(&pack.buildings[0], &stamp).0);
        stamp.lidar = false;
        stamp.roof_shape = Some(RoofShape::Flat);
        assert!(!sampled_roof(&pack.buildings[0], &stamp).0);
        stamp.roof_shape = Some(RoofShape::Pyramidal);
        pack.buildings[0].rings = vec![vec![
            [-6.0, -6.0],
            [6.0, -6.0],
            [6.0, 0.0],
            [0.0, 0.0],
            [0.0, 6.0],
            [-6.0, 6.0],
        ]];
        assert!(!sampled_roof(&pack.buildings[0], &stamp).0);
    }

    #[test]
    fn source_height_and_courtyard_survive_structural_roof_styles() {
        use crate::architecture_styles::ArchitectureStyle;
        let frame = frame(17);
        for style in [
            ArchitectureStyle::DutchGable,
            ArchitectureStyle::JaisalmerHaveli,
            ArchitectureStyle::ParisMansard,
            ArchitectureStyle::IndonesianTiered,
            ArchitectureStyle::AlberobelloTrulli,
            ArchitectureStyle::SibiuEyebrow,
            ArchitectureStyle::ShirakawaOgimachi,
            ArchitectureStyle::KurashikiBikan,
        ] {
            for courtyard in [false, true] {
                let mut rings = vec![square(-12.0, 12.0)];
                if courtyard {
                    rings.push(square(-4.0, 4.0));
                }
                let pack = pack_with(rings, 18.0);
                let mut overlay = tier0::build_overlay(
                    frame.addr.cell,
                    &frame,
                    &pack,
                    &|_, _| 100.0,
                    0,
                    None,
                    None,
                    Some(&|_| true),
                );
                let stamp = &mut overlay.source_buildings[0];
                stamp.design.style = style;
                stamp.design.expression = 255;
                stamp.roof_shape = style.profile().inferred_roof;
                stamp.roof_height_m = Some(if stamp.roof_shape == Some(RoofShape::Flat) {
                    0.0
                } else {
                    4.0
                });
                let result = generate_envelopes::<true>(
                    &frame,
                    &pack,
                    &overlay,
                    &[u32::MAX; N],
                    &|_, _, _| Some(100.0),
                    &|_, _, _| None,
                    false,
                    usize::MAX,
                )
                .unwrap();
                assert!(
                    result.complete,
                    "{style:?} courtyard={courtyard} lost source geometry"
                );
                let max = result
                    .mesh
                    .vertices
                    .iter()
                    .map(|v| world_height(&frame, v.pos))
                    .max_by(f64::total_cmp)
                    .unwrap();
                assert!(
                    max <= 118.02 && max >= 113.98,
                    "{style:?} courtyard={courtyard} escaped source roof height bounds: {max}"
                );
                if !courtyard {
                    assert!(
                        (max - 118.0).abs() < 0.02,
                        "{style:?} source peak became {max}"
                    );
                }
                if courtyard {
                    for triangle in result.mesh.indices.chunks_exact(3) {
                        let vertices = [triangle[0], triangle[1], triangle[2]]
                            .map(|i| result.mesh.vertices[i as usize]);
                        if !is_roof_vertex(&vertices[0]) {
                            continue;
                        }
                        let center = vertices
                            .map(|v| source_position(&frame, v.pos))
                            .into_iter()
                            .sum::<DVec3>()
                            / 3.0;
                        assert!(
                            center.x.abs() >= 3.6 || center.y.abs() >= 3.6,
                            "{style:?} filled courtyard at {center}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn facade_trim_cap_never_removes_large_building_glazing_or_entrance() {
        let frame = frame(14);
        let pack = pack_with(vec![square(-180.0, 180.0)], 120.0);
        let stamp = facade_fixture(&frame, &pack);
        let rings = &pack.buildings[0].rings;
        let boundary = Footprint::new(vec![rings], Vec::new()).unwrap().boundary();
        let (lat, lon) = frame.column_latlon(0.5, 0.5);
        let projection = Projection {
            frame: &frame,
            lat,
            lon,
        };
        let mut context = FacadeContext {
            party_walls: Vec::new(),
            opaque_core: None,
            shadow_walls: std::cell::RefCell::default(),
            owner: rings.clone(),
            owner_bounds: bounds(rings),
            owner_query: PolygonQuery::new(rings),
            blockers: Vec::new(),
            entrance: None,
        };
        context.entrance = entrance_on_source_boundary(rings, &boundary, 100.0, &context, None);
        let render = |saturated, simple| {
            let mut mesh = MeshBuffers::default();
            let mut details = MeshBuffers::default();
            if saturated {
                details.vertices.resize(
                    MAX_FACADE_DETAIL_VERTICES,
                    SurfaceVertex {
                        pos: [0.0; 3],
                        face_ao: 0,
                        rgba: [0; 4],
                    },
                );
            }
            for edge in &boundary {
                emit_wall_with_resolution(
                    &projection,
                    &mut mesh,
                    &mut details,
                    *edge,
                    99.6,
                    100.0,
                    220.0,
                    &stamp,
                    rings,
                    &context,
                    simple,
                    FacadeResolution { feature_m: 0.15 },
                );
            }
            (mesh, details)
        };
        let (full, trim) = render(false, false);
        let (capped, capped_trim) = render(true, false);
        let (simple, simple_trim) = render(false, true);
        assert!(
            glazing(&full).len() > 30_000,
            "fixture must exceed the old trim-limited opening count"
        );
        assert!(trim.vertices.len() <= MAX_FACADE_DETAIL_VERTICES);
        assert_eq!(capped_trim.vertices.len(), MAX_FACADE_DETAIL_VERTICES);
        assert!(simple_trim.vertices.is_empty());
        assert_eq!(full.vertices, capped.vertices);
        assert_eq!(full.indices, capped.indices);
        assert_eq!(glazing(&full), glazing(&simple));
        let doors = |mesh: &MeshBuffers| {
            mesh.vertices
                .iter()
                .copied()
                .filter(|vertex| {
                    ((vertex.face_ao >> 5) & 255) as u8 == crate::material::TIMBER_WALL
                })
                .collect::<Vec<_>>()
        };
        assert!(!doors(&full).is_empty());
        assert_eq!(doors(&full), doors(&simple));
    }

    #[test]
    fn source_neighbor_blocks_details_even_without_an_admitted_datum() {
        let frame = frame(18);
        let mut pack = pack_with(vec![square(-12.0, 12.0)], 12.0);
        // The first source edge lies on the south facade. An adjacent mapped
        // neighbor must block it even when only the owner is admitted here.
        pack.buildings.push(Building {
            location_rad: None,
            rings: vec![vec![
                [-12.0, -20.0],
                [12.0, -20.0],
                [12.0, -12.0],
                [-12.0, -12.0],
            ]],
            height_m: 15.0,
            min_height_m: 0.0,
            seed: 43,
            kind: BuildingKind::Building,
            roof_shape: None,
            use_kind: Default::default(),
        });
        let mut overlay = tier0::build_overlay(
            frame.addr.cell,
            &frame,
            &pack,
            &|_, _| 100.0,
            0,
            None,
            None,
            None,
        );
        assert_eq!(overlay.source_buildings.len(), 2);
        for stamp in &mut overlay.source_buildings {
            stamp.design.detail = crate::regional_architecture::DetailFamily::Shaded;
            stamp.design.expression = 0;
            stamp.highrise = None;
        }
        let full = generate(
            &frame,
            &pack,
            &overlay,
            &[u32::MAX; N],
            &|_, _, _| Some(100.0),
            &|_, _, _| None,
        )
        .unwrap();
        overlay
            .source_buildings
            .retain(|stamp| stamp.source_index == Some(0));
        let owner_only = generate(
            &frame,
            &pack,
            &overlay,
            &[u32::MAX; N],
            &|_, _, _| Some(100.0),
            &|_, _, _| None,
        )
        .unwrap();
        assert!(full.complete && owner_only.complete);
        assert!(!full.details.is_empty() && !owner_only.details.is_empty());
        assert_eq!(
            full.details[0].mesh.vertices,
            owner_only.details[0].mesh.vertices
        );
        assert_eq!(
            full.details[0].mesh.indices,
            owner_only.details[0].mesh.indices
        );
        let unblocked = generate(
            &frame,
            &pack_with(pack.buildings[0].rings.clone(), 12.0),
            &overlay,
            &[u32::MAX; N],
            &|_, _, _| Some(100.0),
            &|_, _, _| None,
        )
        .unwrap();
        assert!(unblocked.complete && !unblocked.details.is_empty());
        let south_shade = |data: &BuildingSurfaceData| {
            data.details
                .iter()
                .flat_map(|detail| &detail.mesh.vertices)
                .any(|vertex| {
                    let point = source_position(&frame, vertex.pos);
                    point.x.abs() < 11.0 && point.y < -12.4
                })
        };
        assert!(
            south_shade(&unblocked),
            "fixture needs a projecting south shade"
        );
        assert!(
            !south_shade(&owner_only),
            "unadmitted source neighbor failed to block the shade"
        );
        let rings = &pack.buildings[0].rings;
        let boundary = Footprint::new(vec![rings], Vec::new()).unwrap().boundary();
        let context = FacadeContext::from_sources(rings, &pack.buildings, &[1], 100.0);
        let entrance =
            entrance_on_source_boundary(rings, &boundary, 100.0, &context, None).unwrap();
        assert_ne!(
            entrance.0, 0,
            "unrendered source neighbour exposed a false entrance"
        );
    }

    #[test]
    fn entrances_reject_courtyards_shared_walls_and_elevated_parts() {
        let frame = frame(17);
        let pack = pack_with(vec![square(-12.0, 12.0), square(-6.0, 6.0)], 12.0);
        let rings = &pack.buildings[0].rings;
        let boundary = Footprint::new(vec![rings], Vec::new()).unwrap().boundary();
        let mut context = FacadeContext {
            party_walls: Vec::new(),
            opaque_core: None,
            shadow_walls: std::cell::RefCell::default(),
            owner: rings.clone(),
            owner_bounds: bounds(rings),
            owner_query: PolygonQuery::new(rings),
            blockers: Vec::new(),
            entrance: None,
        };
        let entrance =
            entrance_on_source_boundary(rings, &boundary, 100.0, &context, None).unwrap();
        assert!(entrance.0 < 4, "courtyard selected as the primary entrance");
        let edge = boundary
            .iter()
            .find(|edge| edge_facade(rings, (edge[0] + edge[1]) * 0.5).edge == entrance.0)
            .unwrap();
        let tangent = (edge[1] - edge[0]).normalize();
        let outward = DVec2::new(tangent.y, -tangent.x);
        let blocked_ring = vec![
            vec![
                edge[0] - tangent,
                edge[1] + tangent,
                edge[1] + tangent + outward,
                edge[0] - tangent + outward,
            ]
            .into_iter()
            .map(|p| p.as_vec2().to_array())
            .collect(),
        ];
        context.blockers.push(FacadeBlocker {
            rings: blocked_ring.clone(),
            bounds: bounds(&blocked_ring),
            query: PolygonQuery::new(&blocked_ring),
            bottom: 99.0,
            top: 115.0,
        });
        let clear = entrance_on_source_boundary(rings, &boundary, 100.0, &context, None).unwrap();
        assert_ne!(
            clear.0, entrance.0,
            "attached neighbor must block a doorway"
        );
        context.blockers.clear();
        context.entrance = Some(entrance);
        let (lat, lon) = frame.column_latlon(0.5, 0.5);
        let projection = Projection {
            frame: &frame,
            lat,
            lon,
        };
        let mut stamp = facade_fixture(&frame, &pack);
        stamp.min_h = 8.0;
        stamp.building_kind = BuildingKind::Part;
        let mut mesh = MeshBuffers::default();
        for edge in &boundary {
            emit_wall(
                &projection,
                &mut mesh,
                &mut MeshBuffers::default(),
                *edge,
                108.0,
                108.0,
                120.0,
                &stamp,
                rings,
                &context,
                false,
            );
        }
        assert!(
            mesh.vertices
                .iter()
                .all(|vertex| ((vertex.face_ao >> 5) & 255) as u8 != crate::material::TIMBER_WALL)
        );
    }

    #[test]
    fn projection_clearance_covers_retained_balcony_decks_and_shades() {
        use crate::regional_architecture::DetailFamily;
        let frame = frame(17);
        let pack = pack_with(vec![square(-12.0, 12.0)], 24.0);
        let rings = &pack.buildings[0].rings;
        let barrier = vec![vec![
            [-13.0, -13.0],
            [13.0, -13.0],
            [13.0, -12.0],
            [-13.0, -12.0],
        ]];
        let (lat, lon) = frame.column_latlon(0.5, 0.5);
        let projection = Projection {
            frame: &frame,
            lat,
            lon,
        };
        // Each obstruction touches the outer extent of a retained projection:
        // a balcony deck underside or the top of a sun shade.
        let stamp = facade_fixture(&frame, &pack);
        let grammar = crate::facade_grammar(&stamp);
        let floor_count = (24.0 / grammar.floor_m).round() as u32;
        let floor_height = 24.0 / f64::from(floor_count);
        let (_, band) = crate::facade_floor_bands(grammar, 2, floor_count);
        let window_top = 100.0 + (2.0 + band.1) * floor_height;
        for (family, contact_height) in [
            (DetailFamily::Balcony, 105.95),
            (DetailFamily::Shaded, window_top + 0.09),
        ] {
            let mut stamp = facade_fixture(&frame, &pack);
            stamp.design.detail = family;
            let render = |blocked| {
                let context = FacadeContext {
                    party_walls: Vec::new(),
                    opaque_core: None,
                    shadow_walls: std::cell::RefCell::default(),
                    owner: rings.clone(),
                    owner_bounds: bounds(rings),
                    owner_query: PolygonQuery::new(rings),
                    blockers: if blocked {
                        vec![FacadeBlocker {
                            rings: barrier.clone(),
                            bounds: bounds(&barrier),
                            query: PolygonQuery::new(&barrier),
                            bottom: contact_height - 0.03,
                            top: contact_height + 0.03,
                        }]
                    } else {
                        Vec::new()
                    },
                    entrance: None,
                };
                let mut mesh = MeshBuffers::default();
                let mut details = MeshBuffers::default();
                emit_wall_with_resolution(
                    &projection,
                    &mut mesh,
                    &mut details,
                    [DVec2::new(-12.0, -12.0), DVec2::new(12.0, -12.0)],
                    99.6,
                    100.0,
                    124.0,
                    &stamp,
                    rings,
                    &context,
                    false,
                    FacadeResolution::FULL,
                );
                (mesh, details)
            };
            let (base, free) = render(false);
            let (blocked_base, blocked) = render(true);
            let at_contact = |mesh: &MeshBuffers| {
                mesh.vertices
                    .iter()
                    .any(|vertex| (world_height(&frame, vertex.pos) - contact_height).abs() < 0.01)
            };
            assert!(
                at_contact(&free),
                "{family:?} fixture has no contacting projection"
            );
            assert!(
                !at_contact(&blocked),
                "{family:?} projection crossed a source blocker"
            );
            assert_eq!(
                base.vertices, blocked_base.vertices,
                "blocking trim changed the envelope"
            );
            assert_eq!(base.indices, blocked_base.indices);
        }
    }

    #[test]
    fn source_cut_walls_never_gain_fabricated_openings_or_balconies() {
        let frame = frame(17);
        let pack = pack_with(vec![square(-12.0, 12.0)], 24.0);
        let rings = &pack.buildings[0].rings;
        let mut stamp = facade_fixture(&frame, &pack);
        stamp.design.detail = crate::regional_architecture::DetailFamily::Balcony;
        let context = FacadeContext {
            party_walls: Vec::new(),
            opaque_core: None,
            shadow_walls: std::cell::RefCell::default(),
            owner: rings.clone(),
            owner_bounds: bounds(rings),
            owner_query: PolygonQuery::new(rings),
            blockers: Vec::new(),
            entrance: None,
        };
        let (lat, lon) = frame.column_latlon(0.5, 0.5);
        let projection = Projection {
            frame: &frame,
            lat,
            lon,
        };
        let mut mesh = MeshBuffers::default();
        let mut details = MeshBuffers::default();
        emit_wall(
            &projection,
            &mut mesh,
            &mut details,
            [DVec2::new(0.0, -8.0), DVec2::new(0.0, 8.0)],
            99.6,
            100.0,
            124.0,
            &stamp,
            rings,
            &context,
            false,
        );
        assert!(!mesh.indices.is_empty());
        assert!(glazing(&mesh).is_empty());
        assert!(details.indices.is_empty());
    }

    #[test]
    fn facade_families_keep_major_projections_and_source_collision() {
        use crate::regional_architecture::DetailFamily;
        let frame = frame(17);
        let pack = pack_with(vec![square(-12.0, 12.0)], 18.0);
        let mut overlay = tier0::build_overlay(
            frame.addr.cell,
            &frame,
            &pack,
            &|_, _| 100.0,
            0,
            None,
            None,
            Some(&|_| true),
        );
        let original = overlay.source_buildings[0];
        let mut collisions = None;
        let mut detailed_geometry = Vec::new();
        for family in [
            DetailFamily::Plain,
            DetailFamily::Classical,
            DetailFamily::Timber,
            DetailFamily::Shaded,
            DetailFamily::Balcony,
            DetailFamily::Modern,
            DetailFamily::Industrial,
        ] {
            overlay.source_buildings[0] = BuildingStamp {
                design: crate::regional_architecture::BuildingDesign {
                    detail: family,
                    expression: 0,
                    ..original.design
                },
                ..original
            };
            let result = generate_resolved_facades::<true>(&frame, &pack, &overlay, false);
            assert!(result.complete);
            if let Some(expected) = &collisions {
                assert_eq!(expected, &result.collision);
            } else {
                collisions = Some(result.collision.clone());
            }
            let vertices = result
                .details
                .iter()
                .flat_map(|detail| &detail.mesh.vertices)
                .map(|vertex| vertex.pos.map(f32::to_bits))
                .collect::<Vec<_>>();
            assert!(!vertices.is_empty());
            if !detailed_geometry.contains(&vertices) {
                detailed_geometry.push(vertices);
            }
        }
        assert!(
            detailed_geometry.len() >= 3,
            "bands, shades and balconies must retain distinct larger geometry"
        );
    }

    #[test]
    fn missing_required_datum_keeps_empty_output_incomplete() {
        let frame = frame(19);
        let pack = pack_with(vec![square(-12.0, 12.0)], 36.0);
        for missing in [None, Some(f32::NAN)] {
            let result = generate_test(&frame, &pack, &|_, _, _| missing);
            assert!(!result.complete);
            assert!(result.mesh.indices.is_empty());
            assert!(result.collision.is_empty());
        }
    }

    #[test]
    fn unsupported_required_footprint_keeps_empty_output_incomplete() {
        let frame = frame(11);
        let pack = pack_with(vec![square(-2_500.0, 2_500.0)], 36.0);
        let result = generate_test(&frame, &pack, &|_, _, _| Some(100.0));
        assert!(!result.complete);
        assert!(result.mesh.indices.is_empty());
    }

    #[test]
    fn coarse_source_building_omits_subpixel_panes_and_keeps_its_envelope() {
        let frame = frame(tier0::BUILDINGS_MIN_LEVEL);
        let pack = pack_with(vec![square(-240.0, 240.0)], 180.0);
        let result = generate_test(&frame, &pack, &|_, _, _| Some(100.0));
        assert!(result.complete);
        assert!(glazing(&result.mesh).is_empty());
        assert!(!result.mesh.indices.is_empty());
        assert!(!result.collision.is_empty());
    }

    fn party_wall_pair() -> (ChunkFrame, FeaturePack, CellOverlay) {
        let frame = frame(17);
        let rectangle = |lo, hi| vec![[lo, -14.0], [hi, -14.0], [hi, 14.0], [lo, 14.0]];
        let mut pack = pack_with(vec![rectangle(-14.0, 0.0)], 18.0);
        pack.buildings.push(Building {
            location_rad: None,
            rings: vec![rectangle(0.0, 14.0)],
            height_m: 27.0,
            min_height_m: 0.0,
            seed: 43,
            kind: BuildingKind::Building,
            roof_shape: None,
            use_kind: Default::default(),
        });
        pack.buildings[1].rings[0].rotate_left(2);
        let mut overlay = tier0::build_overlay(
            frame.addr.cell,
            &frame,
            &pack,
            &|_, _| 100.0,
            0,
            None,
            None,
            Some(&|_| true),
        );
        for stamp in &mut overlay.source_buildings {
            stamp.roof_shape = Some(RoofShape::Flat);
            stamp.roof_height_m = Some(0.0);
            stamp.design.style = crate::architecture_styles::ArchitectureStyle::DutchGable;
            stamp.design.expression = 255;
        }
        (frame, pack, overlay)
    }

    fn shadow_fixture(
        frame: &ChunkFrame,
        x: [f64; 2],
        y: [f64; 2],
        depth_m: f64,
    ) -> (MeshBuffers, Vec<ShadowWall>) {
        let mut mesh = MeshBuffers::default();
        emit_quad(
            &mut mesh,
            [
                DVec3::new(5.0, 0.0, 16.0),
                DVec3::new(25.0, 0.0, 16.0),
                DVec3::new(25.0, 10.0, 16.0),
                DVec3::new(5.0, 10.0, 16.0),
            ],
            SurfaceAppearance {
                material: crate::material::WALL,
                tint: [100; 3],
            },
            false,
        );
        let backing = 0..mesh.indices.len();
        let z = 16.0 + depth_m / frame.grid_scale.z;
        let start = mesh.indices.len();
        emit_quad(
            &mut mesh,
            [
                DVec3::new(x[0], y[0], z),
                DVec3::new(x[1], y[0], z),
                DVec3::new(x[1], y[1], z),
                DVec3::new(x[0], y[1], z),
            ],
            SurfaceAppearance {
                material: crate::material::WINDOW,
                tint: [150; 3],
            },
            false,
        );
        let end = mesh.indices.len();
        (
            mesh,
            vec![ShadowWall {
                backing,
                units: vec![start..end],
            }],
        )
    }

    #[test]
    fn shadow_omission_proves_actual_metric_backing_and_rejects_diagonals_or_relief() {
        for level in [12, 17, 21] {
            let frame = frame(level);
            for depth in [0.018, 0.025, -0.008] {
                let (mesh, walls) = shadow_fixture(&frame, [8.0, 10.0], [5.0, 7.0], depth);
                let omissions = prove_shadow_omissions(&mesh, &frame, &walls);
                assert_eq!(omissions.len(), 1);
                assert_eq!(omissions[0].first_index as usize, walls[0].units[0].start);
                assert_eq!(omissions[0].index_count as usize, walls[0].units[0].len());
                let a = DVec3::from_array(mesh.vertices[0].pos.map(f64::from));
                let b = DVec3::from_array(mesh.vertices[1].pos.map(f64::from));
                let c = DVec3::from_array(mesh.vertices[2].pos.map(f64::from));
                let normal = metric_delta(&frame, b - a)
                    .cross(metric_delta(&frame, c - a))
                    .normalize();
                for &index in &mesh.indices[walls[0].units[0].clone()] {
                    let p = DVec3::from_array(mesh.vertices[index as usize].pos.map(f64::from));
                    let actual = normal.dot(metric_delta(&frame, p - a)).abs();
                    assert!(f64::from(omissions[0].max_deviation_m) >= actual);
                }
                assert!(omissions[0].max_deviation_m <= 0.05);
            }
            for (x, y, depth) in [
                ([14.0, 16.0], [4.0, 6.0], 0.025),
                ([8.0, 10.0], [5.0, 7.0], 0.20),
                ([26.0, 28.0], [5.0, 7.0], 0.025),
            ] {
                let (mesh, walls) = shadow_fixture(&frame, x, y, depth);
                assert!(prove_shadow_omissions(&mesh, &frame, &walls).is_empty());
            }
        }
    }

    #[test]
    fn shadow_omission_final_clipping_cannot_use_a_removed_backing_wall() {
        let frame = frame(20);
        let mut mesh = MeshBuffers::default();
        let quad = |x| {
            [
                DVec3::new(x, 0.0, 8.0),
                DVec3::new(x, 0.0, 20.0),
                DVec3::new(x, 10.0, 20.0),
                DVec3::new(x, 10.0, 8.0),
            ]
        };
        emit_quad(
            &mut mesh,
            quad(11.999),
            SurfaceAppearance {
                material: crate::material::WALL,
                tint: [100; 3],
            },
            false,
        );
        let backing = 0..mesh.indices.len();
        let start = mesh.indices.len();
        emit_quad(
            &mut mesh,
            [
                DVec3::new(12.001, 5.0, 10.0),
                DVec3::new(12.001, 5.0, 12.0),
                DVec3::new(12.001, 7.0, 12.0),
                DVec3::new(12.001, 7.0, 10.0),
            ],
            SurfaceAppearance {
                material: crate::material::WINDOW,
                tint: [150; 3],
            },
            false,
        );
        let mut walls = vec![ShadowWall {
            backing,
            units: vec![start..mesh.indices.len()],
        }];
        assert_eq!(prove_shadow_omissions(&mesh, &frame, &walls).len(), 1);
        let eligible = [u32::MAX & !(1 << 11); N];
        let mesh = clip_to_eligible_recorded(mesh, &eligible, Some(&mut walls));
        assert!(
            !mesh.indices.is_empty(),
            "offset pane survives in the neighboring eligible column"
        );
        assert!(walls.is_empty());
        assert!(prove_shadow_omissions(&mesh, &frame, &walls).is_empty());
    }

    #[test]
    fn shadow_omission_clipping_append_and_compaction_keep_index_ownership() {
        let frame = frame(17);
        let (mesh, mut walls) = shadow_fixture(&frame, [8.0, 10.0], [5.0, 7.0], 0.025);
        let mut eligible = [u32::MAX; N];
        eligible[16] &= !(1 << 20);
        let reference = clip_to_eligible(mesh.clone(), &eligible);
        let mesh = clip_to_eligible_recorded(mesh, &eligible, Some(&mut walls));
        assert_eq!(mesh.vertices, reference.vertices);
        assert_eq!(mesh.indices, reference.indices);
        let omissions = prove_shadow_omissions(&mesh, &frame, &walls);
        assert!(!omissions.is_empty());
        let first = omissions[0].first_index;
        let (mut target, _) = shadow_fixture(&frame, [8.0, 10.0], [5.0, 7.0], 0.25);
        let base = target.indices.len() as u32;
        let mut result = Vec::new();
        append_mesh_with_shadows(&mut target, &mut result, mesh, omissions);
        assert_eq!(result[0].first_index, base + first);
        let attributes = |mesh: &MeshBuffers| {
            mesh.indices
                .iter()
                .map(|&i| mesh.vertices[i as usize])
                .collect::<Vec<_>>()
        };
        let before = attributes(&target);
        crate::built_surface::compact_mesh(&mut target);
        assert_eq!(attributes(&target), before);
        for span in &result {
            assert!(
                target.indices
                    [span.first_index as usize..(span.first_index + span.index_count) as usize]
                    .iter()
                    .all(
                        |&index| ((target.vertices[index as usize].face_ao >> 5) & 255) as u8
                            == crate::material::WINDOW
                    )
            );
        }
    }

    #[test]
    fn shadow_omission_opening_registration_excludes_relief_doors_and_oriels() {
        let frame = frame(17);
        let (lat, lon) = frame.column_latlon(0.5, 0.5);
        let projection = Projection {
            frame: &frame,
            lat,
            lon,
        };
        let units = std::cell::RefCell::default();
        let plane = FacadePlane {
            shadow_units: Some(&units),
            projection: &projection,
            origin: DVec2::ZERO,
            direction: DVec2::X,
            outward: DVec2::Y,
        };
        let glass = SurfaceAppearance {
            material: crate::material::WINDOW,
            tint: [150; 3],
        };
        let trim = SurfaceAppearance {
            material: crate::material::WALL,
            tint: [100; 3],
        };
        let mut mesh = MeshBuffers::default();
        for (relief, depth, material) in [
            (false, 0.025, glass),
            (true, 0.025, glass),
            (false, 0.5, glass),
            (false, 0.025, trim),
        ] {
            plane.opening(
                &mut mesh,
                [-1.0, 1.0],
                [101.0, 103.0],
                depth,
                OpeningShape::RoundArch,
                0.1,
                material,
                trim,
                relief,
            );
        }
        assert_eq!(units.borrow().len(), 1);
        assert!(units.borrow()[0].end < mesh.indices.len());
    }

    #[test]
    fn shadow_omission_source_buildings_retain_all_meshes_and_pruned_party_wall_safety() {
        let (frame, pack, overlay) = party_wall_pair();
        for simple in [false, true] {
            for prune in [false, true] {
                let result = if prune {
                    generate_resolved_facades::<true>(&frame, &pack, &overlay, simple)
                } else {
                    generate_resolved_facades::<false>(&frame, &pack, &overlay, simple)
                };
                assert!(!result.shadow_omissions.is_empty());
                let mut end = 0;
                for span in &result.shadow_omissions {
                    assert!(span.first_index >= end);
                    assert!(span.index_count > 0 && span.index_count.is_multiple_of(3));
                    end = span.first_index + span.index_count;
                    assert!(end as usize <= result.mesh.indices.len());
                    assert!(span.max_deviation_m.is_finite() && span.max_deviation_m <= 0.05);
                }
                assert!(
                    result
                        .shadow_omissions
                        .iter()
                        .map(|span| span.index_count as usize)
                        .sum::<usize>()
                        < result.mesh.indices.len()
                );
            }
        }
    }

    #[test]
    fn emitted_party_wall_pruning_retains_every_exposed_triangle_and_collision() {
        let (frame, pack, overlay) = party_wall_pair();
        for simple in [false, true] {
            let original = generate_resolved_facades::<false>(&frame, &pack, &overlay, simple);
            let optimized = generate_resolved_facades::<true>(&frame, &pack, &overlay, simple);
            assert!(original.complete && optimized.complete);
            assert!(optimized.mesh.indices.len() + 30 < original.mesh.indices.len());
            assert!(glazing(&optimized.mesh).len() < glazing(&original.mesh).len());
            assert_eq!(optimized.collision, original.collision);
            assert_eq!(optimized.details.len(), original.details.len());
            for (a, b) in optimized.details.iter().zip(&original.details) {
                assert_eq!(a.mesh.vertices, b.mesh.vertices);
                assert_eq!(a.mesh.indices, b.mesh.indices);
            }
            let triangle_key = |mesh: &MeshBuffers, triangle: &[u32]| {
                std::array::from_fn::<_, 3, _>(|i| {
                    let vertex = mesh.vertices[triangle[i] as usize];
                    (vertex.pos.map(f32::to_bits), vertex.face_ao, vertex.rgba)
                })
            };
            let retained: std::collections::HashSet<_> = optimized
                .mesh
                .indices
                .chunks_exact(3)
                .map(|triangle| triangle_key(&optimized.mesh, triangle))
                .collect();
            let reference: std::collections::HashSet<_> = original
                .mesh
                .indices
                .chunks_exact(3)
                .map(|triangle| triangle_key(&original.mesh, triangle))
                .collect();
            let mut added = 0;
            for triangle in optimized.mesh.indices.chunks_exact(3) {
                if reference.contains(&triangle_key(&optimized.mesh, triangle)) {
                    continue;
                }
                added += 1;
                let points: [DVec3; 3] = std::array::from_fn(|i| {
                    let vertex = optimized.mesh.vertices[triangle[i] as usize];
                    assert!((world_height(&frame, vertex.pos) - 99.6).abs() < 0.002);
                    DVec3::from_array(vertex.pos.map(f64::from))
                });
                assert!(
                    (points[1] - points[0]).cross(points[2] - points[0]).y < 0.0,
                    "the only new triangles are outward foundation undersides"
                );
            }
            assert!(added >= 2);
            let metric = crate::collision::MetricFrame::of(&frame);
            let (lat, lon) = frame.column_latlon(0.5, 0.5);
            let projection = Projection {
                frame: &frame,
                lat,
                lon,
            };
            let point = |height| {
                DVec3::from_array(
                    metric.point_from_ecef(projection.world_point(DVec2::new(-7.0, 0.0), height)),
                )
            };
            let origin = point(90.0);
            let direction = (point(110.0) - origin).normalize();
            let rendered = crate::collision::SurfaceCollision::from_triangles(
                crate::collision::triangles_from_mesh(&optimized.mesh, &frame),
            );
            let hit = rendered
                .raycast(origin.to_array(), direction.to_array(), 12.0)
                .expect("the rendered foundation must close the underside view");
            assert!((hit - 9.6).abs() < 0.01);
            let prior = crate::collision::SurfaceCollision::from_triangles(
                crate::collision::triangles_from_mesh(&original.mesh, &frame),
            );
            assert!(
                prior
                    .raycast(origin.to_array(), direction.to_array(), 12.0)
                    .is_none()
            );
            let mut removed = 0;
            for triangle in original.mesh.indices.chunks_exact(3) {
                if retained.contains(&triangle_key(&original.mesh, triangle)) {
                    continue;
                }
                removed += 1;
                for &index in triangle {
                    let point = source_position(&frame, original.mesh.vertices[index as usize].pos);
                    assert!(
                        point.x >= -0.027 && point.x <= 0.002,
                        "removed exposed side {point:?}"
                    );
                    assert!(point.y >= -14.002 && point.y <= 14.002);
                    assert!(
                        point.z >= 99.598 && point.z <= 118.002,
                        "removed exposed height {point:?}"
                    );
                }
            }
            assert!(removed > 10);
        }
    }

    fn opaque_test_core(
        projection: &Projection<'_>,
        boundary: &[[DVec2; 2]],
        base: f64,
        eave: f64,
    ) -> OpaqueCore {
        let mut core = OpaqueCore {
            roof_y: boundary
                .iter()
                .flatten()
                .map(|&point| f64::from(projection.point(point, eave).as_vec3().y))
                .min_by(f64::total_cmp),
            ..OpaqueCore::default()
        };
        for &edge in boundary {
            let midpoint = (edge[0] + edge[1]) * 0.5;
            let tangent = (edge[1] - edge[0]).normalize();
            let outward = projection
                .point(midpoint + DVec2::new(tangent.y, -tangent.x) * 0.1, base)
                - projection.point(midpoint, base);
            core.wall(
                [
                    projection.point(edge[0], base),
                    projection.point(edge[1], base),
                    projection.point(edge[1], eave),
                    projection.point(edge[0], eave),
                ],
                outward,
                true,
            );
        }
        core
    }

    #[test]
    fn party_wall_index_rejects_gaps_partial_edges_same_side_and_height_exposure() {
        let rings = vec![vec![
            [-10.0, -10.0],
            [0.0, -10.0],
            [0.0, 10.0],
            [-10.0, 10.0],
        ]];
        let boundary = Footprint::new(vec![&rings], vec![]).unwrap().boundary();
        let frame = frame(17);
        let (lat, lon) = frame.column_latlon(0.5, 0.5);
        let projection = Projection {
            frame: &frame,
            lat,
            lon,
        };
        let mut index = OpaqueWallIndex::default();
        let edge = [DVec2::new(0.0, 10.0), DVec2::new(0.0, -10.0)];
        assert!(index.facing(&[edge]).is_empty());
        index.insert(
            &boundary,
            &rings,
            105.0,
            118.0,
            opaque_test_core(&projection, &boundary, 105.0, 118.0),
            MeshBuffers::default(),
        );
        let facing = index.facing(&[edge]);
        assert_eq!(facing.len(), 1);
        assert!(facing[0].covers_heights(105.0, 118.0));
        assert!(!facing[0].covers_heights(104.99, 118.0));
        assert!(!facing[0].covers_heights(105.0, 118.01));
        assert!(index.facing(&[[edge[1], edge[0]]]).is_empty());
        assert!(index.facing(&[[edge[0], DVec2::ZERO]]).is_empty());
        for gap in [f64::EPSILON, 0.00001, 0.01, 1.0] {
            assert!(index.facing(&[edge.map(|p| p + DVec2::X * gap)]).is_empty());
        }
        let plane = FacadePlane {
            shadow_units: None,
            projection: &projection,
            origin: edge[0],
            direction: -DVec2::Y,
            outward: -DVec2::X,
        };
        assert!(facing[0].covers_glazing(
            &plane,
            [2.0, 18.0],
            [106.0, 117.0],
            0.025,
            OpeningShape::Rectangular
        ));
        assert!(!facing[0].covers_glazing(
            &plane,
            [2.0, 18.0],
            [106.0, 119.0],
            0.025,
            OpeningShape::Rectangular
        ));
        assert!(!facing[0].covers_glazing(
            &plane,
            [2.0, 21.0],
            [106.0, 117.0],
            0.025,
            OpeningShape::Rectangular
        ));
        assert!(!facing[0].covers_glazing(
            &plane,
            [2.0, 18.0],
            [106.0, 117.0],
            10.1,
            OpeningShape::Rectangular
        ));
        let courtyard = vec![
            rings[0].clone(),
            vec![[-0.04, -2.0], [-0.01, -2.0], [-0.01, 2.0], [-0.04, 2.0]],
        ];
        let wall = PartyWall {
            edge,
            body: Rc::new(OpaqueBody {
                query: PolygonQuery::new(&courtyard),
                core: facing[0].body.core.clone(),
                foundation: std::cell::RefCell::new(None),
                used: std::cell::Cell::new(false),
                base: 105.0,
                eave: 118.0,
            }),
        };
        assert!(
            !wall.covers_glazing(
                &plane,
                [2.0, 18.0],
                [106.0, 117.0],
                0.025,
                OpeningShape::Rectangular
            ),
            "a hole between the endpoints must remain exposed"
        );
    }

    #[test]
    fn party_wall_glazing_uses_rendered_polar_chords_not_only_source_containment() {
        use vearth_core::cell::{CellId, ChunkAddr};
        let cell = CellId::from_dir(
            ellipsoid::geodetic_to_ecef(80.0_f64.to_radians(), 0.0, 0.0).normalize(),
            11,
        );
        let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
        let (lat, lon) = frame.column_latlon(0.5, 0.5);
        let projection = Projection {
            frame: &frame,
            lat,
            lon,
        };
        for half_width in [3.0_f32, 1500.0] {
            let rings = vec![vec![
                [-half_width, 0.0],
                [half_width, 0.0],
                [half_width, 20.0],
                [-half_width, 20.0],
            ]];
            let boundary = Footprint::new(vec![&rings], vec![]).unwrap().boundary();
            let edge = [
                DVec2::new(f64::from(half_width), 0.0),
                DVec2::new(-f64::from(half_width), 0.0),
            ];
            let mut index = OpaqueWallIndex::default();
            index.insert(
                &boundary,
                &rings,
                100.0,
                120.0,
                opaque_test_core(&projection, &boundary, 100.0, 120.0),
                MeshBuffers::default(),
            );
            let wall = index.facing(&[edge]).pop().unwrap();
            let plane = FacadePlane {
                shadow_units: None,
                projection: &projection,
                origin: edge[0],
                direction: -DVec2::X,
                outward: DVec2::Y,
            };
            let x = [f64::from(half_width) - 1.0, f64::from(half_width) + 1.0];
            assert!(x.iter().all(|&x| {
                wall.body
                    .query
                    .contains(plane.origin + plane.direction * x + plane.outward * 0.025)
            }));
            assert_eq!(
                wall.covers_glazing(
                    &plane,
                    x,
                    [110.0, 115.0],
                    0.025,
                    OpeningShape::SegmentalArch
                ),
                half_width < 10.0,
                "a long latitude edge's straight rendered wall can expose source-interior glazing"
            );
        }
    }

    #[test]
    fn party_wall_pruning_requires_a_whole_emitted_opaque_cell_owner() {
        for case in 0..7 {
            let (frame, mut pack, mut overlay) = party_wall_pair();
            let mut eligible = [u32::MAX; N];
            match case {
                0 => overlay
                    .source_buildings
                    .retain(|stamp| stamp.seed != pack.buildings[0].seed),
                1 => {
                    pack.buildings[0].kind = BuildingKind::RoofOnly;
                    overlay.source_buildings[0].building_kind = BuildingKind::RoofOnly;
                }
                2 => {
                    pack.buildings[0].rings[0][0][0] = -500.0;
                    pack.buildings[0].rings[0][3][0] = -500.0;
                }
                3 => {
                    pack.buildings[0].rings.push(square(-8.0, -4.0));
                }
                4 => eligible[N / 2] &= !(1 << (N / 2 - 1)),
                5 => {
                    pack.buildings[0].min_height_m = 4.0;
                    overlay.source_buildings[0].min_h = 4.0;
                }
                6 => overlay.source_buildings[0].lidar = true,
                _ => unreachable!(),
            }
            let original = generate_envelopes::<false>(
                &frame,
                &pack,
                &overlay,
                &eligible,
                &|_, _, _| Some(100.0),
                &|_, _, _| None,
                false,
                usize::MAX,
            )
            .unwrap();
            let optimized = generate_envelopes::<true>(
                &frame,
                &pack,
                &overlay,
                &eligible,
                &|_, _, _| Some(100.0),
                &|_, _, _| None,
                false,
                usize::MAX,
            )
            .unwrap();
            assert_eq!(
                optimized.mesh.vertices, original.mesh.vertices,
                "case {case}"
            );
            assert_eq!(optimized.mesh.indices, original.mesh.indices, "case {case}");
        }
    }

    #[test]
    fn compaction_cannot_hide_owners_after_a_budget_stop() {
        let frame = frame(17);
        let mut pack = pack_with(vec![square(-20.0, -10.0)], 36.0);
        let first = generate_test(&frame, &pack, &|_, _, _| Some(100.0));
        let budget = crate::built_surface::mesh_bytes(&first.mesh) - 1;
        let mut compact = first.mesh;
        crate::built_surface::compact_mesh(&mut compact);
        assert!(crate::built_surface::mesh_bytes(&compact) < budget);
        pack.buildings.push(Building {
            location_rad: None,
            rings: vec![square(10.0, 20.0)],
            seed: 43,
            height_m: 36.0,
            min_height_m: 0.0,
            kind: BuildingKind::Building,
            roof_shape: None,
            use_kind: Default::default(),
        });
        let overlay = tier0::build_overlay(
            frame.addr.cell,
            &frame,
            &pack,
            &|_, _| 100.0,
            0,
            None,
            None,
            Some(&|_| true),
        );
        let ground = |_, _, _| Some(100.0);
        let surface = |_, _, _| None;
        let expected = generate_envelopes::<true>(
            &frame,
            &pack,
            &overlay,
            &[u32::MAX; N],
            &ground,
            &surface,
            true,
            usize::MAX,
        )
        .unwrap();
        let result = generate_bounded(
            &frame,
            &pack,
            &overlay,
            &[u32::MAX; N],
            &ground,
            &surface,
            budget,
        )
        .unwrap();
        assert!(result.complete);
        assert_eq!(result.collision, expected.collision);
    }

    fn source_position(frame: &ChunkFrame, point: [f32; 3]) -> DVec3 {
        let (lat0, lon0) = frame.column_latlon(0.5, 0.5);
        let (lat, lon, height) = ellipsoid::ecef_to_geodetic(
            frame.local_to_ecef(DVec3::from_array(point.map(f64::from))),
        );
        DVec3::new(
            (lon - lon0) * MEAN_RADIUS * lat0.cos(),
            (lat - lat0) * MEAN_RADIUS,
            height,
        )
    }

    #[test]
    fn flat_terrace_parapets_keep_top_holes_inward_returns_and_simple_budget() {
        let frame = frame(18);
        let mut pack = pack_with(vec![square(-12.0, 12.0), square(-4.0, 4.0)], 12.0);
        pack.buildings[0].roof_shape = Some(RoofShape::Flat);
        let overlay = tier0::build_overlay(
            frame.addr.cell,
            &frame,
            &pack,
            &|_, _| 100.0,
            0,
            None,
            None,
            None,
        );
        let roof = FlatRoof::for_source(
            &pack.buildings[0],
            &overlay.source_buildings[0],
            100.0,
            112.0,
        )
        .unwrap();
        let render = |simple| {
            generate_envelopes::<true>(
                &frame,
                &pack,
                &overlay,
                &[u32::MAX; N],
                &|_, _, _| Some(100.0),
                &|_, _, _| None,
                simple,
                usize::MAX,
            )
            .unwrap()
        };
        let detailed = render(false);
        let simple = render(true);
        let upward = |mesh: &MeshBuffers| {
            mesh.indices
                .chunks_exact(3)
                .filter(|tri| mesh.vertices[tri[0] as usize].face_ao & 7 == 2)
                .map(|tri| std::array::from_fn::<_, 3, _>(|i| mesh.vertices[tri[i] as usize]))
                .collect::<Vec<_>>()
        };
        assert_eq!(upward(&detailed.mesh), upward(&simple.mesh));
        assert_eq!(detailed.collision, simple.collision);
        let budget = crate::built_surface::mesh_bytes(&detailed.mesh) - 1;
        let bounded = generate_bounded(
            &frame,
            &pack,
            &overlay,
            &[u32::MAX; N],
            &|_, _, _| Some(100.0),
            &|_, _, _| None,
            budget,
        )
        .unwrap();
        assert_eq!(upward(&detailed.mesh), upward(&bounded.mesh));
        let mut decks = 0;
        let mut caps = 0;
        let mut inward_return = false;
        for tri in detailed.mesh.indices.chunks_exact(3) {
            let p = std::array::from_fn::<_, 3, _>(|i| {
                source_position(&frame, detailed.mesh.vertices[tri[i] as usize].pos)
            });
            if p.iter().all(|p| p.z > roof.deck - 0.01) {
                for p in p {
                    assert!(p.z <= roof.top + 0.01, "parapet exceeded reported top");
                    assert!(
                        contains_or_boundary(&pack.buildings[0].rings, p.truncate()),
                        "parapet crossed source footprint/hole"
                    );
                }
                let normal = (p[1] - p[0]).cross(p[2] - p[0]);
                let center = (p[0] + p[1] + p[2]) / 3.0;
                if (-11.99..-11.7).contains(&center.x)
                    && center.y.abs() < 11.0
                    && normal.x > 0.01
                    && normal.z.abs() < normal.x * 0.05
                {
                    inward_return = true;
                }
            }
        }
        for tri in upward(&detailed.mesh) {
            let height = tri
                .map(|v| world_height(&frame, v.pos))
                .into_iter()
                .sum::<f64>()
                / 3.0;
            if height < roof.deck - 0.01 {
                continue;
            }
            if (height - roof.deck).abs() < 0.01 {
                decks += 1;
            } else if (height - roof.top).abs() < 0.01 {
                caps += 1;
            } else {
                panic!("flat terrace has unexpected height {height}");
            }
        }
        assert!(decks > 0 && caps > 0 && inward_return);
        let metric = crate::collision::MetricFrame::of(&frame);
        let collision_top = detailed
            .collision
            .iter()
            .flat_map(|t| t.vertices)
            .map(|p| ellipsoid::ecef_to_geodetic(metric.point_to_ecef(p.map(f64::from))).2)
            .max_by(f64::total_cmp)
            .unwrap();
        assert!(
            (collision_top - 112.0).abs() < 0.01,
            "source collision cap changed"
        );
    }

    #[test]
    fn unresolved_flat_rim_requires_both_dimensions_and_preserves_stepped_crowns() {
        let roof = FlatRoof {
            deck: 9.5,
            top: 10.0,
            thickness: 0.15,
            stepped: false,
        };
        let resolved = |roof: FlatRoof| [roof.deck, roof.top, roof.thickness];
        assert_eq!(resolved(roof.with_resolution(0.7)), [10.0, 10.0, 0.0]);
        for pixel in [0.1, 0.5, 0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(resolved(roof.with_resolution(pixel)), resolved(roof));
        }
        for retained in [
            FlatRoof {
                thickness: 0.8,
                ..roof
            },
            FlatRoof { deck: 9.2, ..roof },
            FlatRoof {
                stepped: true,
                ..roof
            },
        ] {
            assert_eq!(resolved(retained.with_resolution(0.7)), resolved(retained));
        }
        let a = frame(17);
        let mut address = a.addr;
        address.cell.i += 1;
        let b = ChunkFrame::of(address);
        assert_eq!(
            resolved(roof.with_resolution(a.addr.cell.sample_pitch_m() / 3.0)),
            resolved(roof.with_resolution(b.addr.cell.sample_pitch_m() / 3.0))
        );
    }

    #[test]
    fn unresolved_flat_rim_keeps_closed_top_source_hole_and_no_replacement_eave() {
        let frame = frame(17);
        let mut pack = pack_with(vec![square(-12.0, 12.0), square(-4.0, 4.0)], 12.0);
        pack.buildings[0].roof_shape = Some(RoofShape::Flat);
        let mut overlay = tier0::build_overlay(
            frame.addr.cell,
            &frame,
            &pack,
            &|_, _| 100.0,
            0,
            None,
            None,
            None,
        );
        overlay.source_buildings[0].design.style =
            crate::architecture_styles::ArchitectureStyle::Contemporary;
        overlay.source_buildings[0].design.expression = 255;
        let source_roof = FlatRoof::for_source(
            &pack.buildings[0],
            &overlay.source_buildings[0],
            100.0,
            112.0,
        )
        .unwrap();
        let roof = source_roof.with_resolution(frame.addr.cell.sample_pitch_m() / 3.0);
        assert!(!roof.stepped && roof.deck == roof.top && roof.thickness == 0.0);
        assert!(roof.deck - source_roof.deck < frame.addr.cell.sample_pitch_m() / 3.0);
        let result = generate(
            &frame,
            &pack,
            &overlay,
            &[u32::MAX; N],
            &|_, _, _| Some(100.0),
            &|_, _, _| None,
        )
        .unwrap();
        assert!(result.complete);
        let mut roof_area = 0.0;
        for triangle in result.mesh.indices.chunks_exact(3) {
            let vertices =
                [triangle[0], triangle[1], triangle[2]].map(|i| result.mesh.vertices[i as usize]);
            let points = vertices.map(|vertex| source_position(&frame, vertex.pos));
            if vertices.iter().all(is_roof_vertex) {
                assert!(points.iter().all(|p| (p.z - 112.0).abs() < 0.01));
                let center = (points[0] + points[1] + points[2]) / 3.0;
                assert!(contains_or_boundary(
                    &pack.buildings[0].rings,
                    center.truncate()
                ));
                roof_area += ((points[1] - points[0])
                    .truncate()
                    .perp_dot((points[2] - points[0]).truncate()))
                .abs()
                    * 0.5;
            }
            for p in points.into_iter().filter(|p| p.z > source_roof.deck + 0.01) {
                assert!(p.z <= 112.01);
                assert!(
                    p.x.abs() <= 12.01 && p.y.abs() <= 12.01,
                    "unresolved rim enabled an outward eave"
                );
                assert!(
                    p.x.abs() >= 3.99 || p.y.abs() >= 3.99,
                    "source hole was filled"
                );
            }
        }
        assert!(
            (roof_area - (24.0 * 24.0 - 8.0 * 8.0)).abs() < 0.05,
            "closed roof area {roof_area}"
        );
        let metric = crate::collision::MetricFrame::of(&frame);
        let collision_top = result
            .collision
            .iter()
            .flat_map(|triangle| triangle.vertices)
            .map(|p| ellipsoid::ecef_to_geodetic(metric.point_to_ecef(p.map(f64::from))).2)
            .max_by(f64::total_cmp)
            .unwrap();
        assert!((collision_top - 112.0).abs() < 0.01);
    }

    #[test]
    fn flat_terrace_ownership_cuts_stop_at_deck_without_internal_fences() {
        let frame = frame(18);
        for cut_x in [-3.0f32, -11.995] {
            let mut pack = pack_with(vec![square(-12.0, 12.0)], 12.0);
            pack.buildings[0].roof_shape = Some(RoofShape::Flat);
            pack.buildings.push(Building {
                rings: vec![vec![
                    [cut_x, -12.0],
                    [12.0, -12.0],
                    [12.0, 12.0],
                    [cut_x, 12.0],
                ]],
                location_rad: None,
                height_m: 18.0,
                min_height_m: 0.0,
                seed: 77,
                kind: BuildingKind::Part,
                roof_shape: Some(RoofShape::Flat),
                use_kind: Default::default(),
            });
            let mut overlay = tier0::build_overlay(
                frame.addr.cell,
                &frame,
                &pack,
                &|_, _| 100.0,
                0,
                None,
                None,
                None,
            );
            overlay.source_buildings.retain(|stamp| stamp.seed == 42);
            let roof = FlatRoof::for_source(
                &pack.buildings[0],
                &overlay.source_buildings[0],
                100.0,
                112.0,
            )
            .unwrap();
            let result = generate(
                &frame,
                &pack,
                &overlay,
                &[u32::MAX; N],
                &|_, _, _| Some(100.0),
                &|_, _, _| None,
            )
            .unwrap();
            let mut cut_triangles = 0;
            for tri in result.mesh.indices.chunks_exact(3) {
                let p = std::array::from_fn::<_, 3, _>(|i| {
                    source_position(&frame, result.mesh.vertices[tri[i] as usize].pos)
                });
                if p.iter().all(|p| (p.x - f64::from(cut_x)).abs() < 0.0001)
                    && p.iter().map(|p| p.y).sum::<f64>().abs() / 3.0 < 10.0
                {
                    assert!(
                        p.iter().all(|p| p.z <= roof.deck + 0.01),
                        "derived ownership cut became a parapet fence"
                    );
                    cut_triangles += 1;
                }
            }
            assert!(cut_triangles > 0);
        }
    }

    #[test]
    fn flat_parent_terrace_continues_under_a_floating_source_part() {
        let frame = frame(18);
        let mut pack = pack_with(vec![square(-12.0, 12.0)], 12.0);
        pack.buildings[0].roof_shape = Some(RoofShape::Flat);
        pack.buildings.push(Building {
            rings: vec![square(-5.0, 5.0)],
            location_rad: None,
            height_m: 28.0,
            min_height_m: 20.0,
            seed: 77,
            kind: BuildingKind::Part,
            roof_shape: Some(RoofShape::Flat),
            use_kind: Default::default(),
        });
        let result = generate_test(&frame, &pack, &|_, _, _| Some(100.0));
        let deck = 112.0 - (0.45 + ((pack.buildings[0].seed >> 17) & 3) as f64 * (0.20 / 3.0));
        let mut under_part = 0;
        for vertex in &result.mesh.vertices {
            let p = source_position(&frame, vertex.pos);
            if vertex.face_ao & 7 == 2 && p.x.abs() < 3.0 && p.y.abs() < 3.0 && p.z < 115.0 {
                assert!(
                    (p.z - deck).abs() < 0.01,
                    "floating part changed parent terrace height"
                );
                under_part += 1;
            }
        }
        assert!(
            under_part > 0,
            "parent terrace has a hole below the floating part"
        );
    }

    #[test]
    fn flat_terraces_do_not_reinterpret_measured_roofs_or_thin_roof_only_parts() {
        let frame = frame(17);
        let mut pack = pack_with(vec![square(-12.0, 12.0)], 12.0);
        pack.buildings[0].roof_shape = Some(RoofShape::Flat);
        let mut stamp = facade_fixture(&frame, &pack);
        assert!(FlatRoof::for_source(&pack.buildings[0], &stamp, 100.0, 112.0).is_some());
        stamp.lidar = true;
        assert!(FlatRoof::for_source(&pack.buildings[0], &stamp, 100.0, 112.0).is_none());
        stamp.lidar = false;
        pack.buildings[0].kind = BuildingKind::RoofOnly;
        assert!(FlatRoof::for_source(&pack.buildings[0], &stamp, 100.0, 112.0).is_none());
        pack.buildings[0].kind = BuildingKind::Part;
        assert!(FlatRoof::for_source(&pack.buildings[0], &stamp, 111.0, 112.0).is_none());
    }

    #[test]
    fn service_envelopes_have_no_windows_or_night_panels_at_any_resolution() {
        use vearth_data::vectors::BuildingUse;
        for level in [15, 17, 19] {
            for explicit in [false, true] {
                let frame = frame(level);
                let mut pack = pack_with(vec![square(-3.0, 3.0)], 180.0);
                let source = &mut pack.buildings[0];
                source.kind = BuildingKind::Part;
                source.min_height_m = 150.0;
                source.use_kind = if explicit {
                    BuildingUse::Utility
                } else {
                    BuildingUse::Unknown
                };
                let overlay = tier0::build_overlay(
                    frame.addr.cell,
                    &frame,
                    &pack,
                    &|_, _| 100.0,
                    0,
                    None,
                    None,
                    Some(&|_| true),
                );
                let stamp = overlay.source_buildings[0];
                assert!(!stamp.facade_openings);
                assert!(!stamp.night_eligible);
                assert!(stamp.highrise.is_none());
                for simple in [false, true] {
                    let result = generate_resolved_facades::<true>(&frame, &pack, &overlay, simple);
                    assert!(result.complete);
                    assert!(!result.collision.is_empty());
                    assert!(
                        result
                            .mesh
                            .vertices
                            .iter()
                            .chain(result.details.iter().flat_map(|d| &d.mesh.vertices))
                            .all(|v| {
                                let material = ((v.face_ao >> 5) & 255) as u8;
                                material != crate::material::WINDOW
                                    && !(18..=21).contains(&material)
                                    && !crate::highrise::is_facade(material)
                            })
                    );
                    let top = result
                        .mesh
                        .vertices
                        .iter()
                        .map(|v| world_height(&frame, v.pos))
                        .max_by(f64::total_cmp)
                        .unwrap();
                    assert!((top - 280.0).abs() < 0.03);
                }
                let mut sampled = stamp;
                sampled.facade = FacadeStamp {
                    boundary: true,
                    corner: false,
                    edge: 0,
                    along_m: 2.0,
                    length_m: 6.0,
                    edge_distance_m: 0.0,
                };
                for h in 151..179 {
                    assert_eq!(
                        crate::building_appearance(
                            f64::from(h),
                            150.0,
                            180.0,
                            &sampled,
                            None,
                            0.01
                        )
                        .material,
                        stamp.design.wall_material
                    );
                }
            }
        }
    }

    #[test]
    fn corrected_outline_height_agrees_in_overlay_surface_and_collision() {
        for level in [15, 17, 19] {
            let frame = frame(level);
            let mut pack = pack_with(vec![square(-20.0, 20.0)], 160.0);
            pack.buildings[0].roof_shape = Some(RoofShape::Flat);
            pack.buildings.push(Building {
                location_rad: None,
                rings: vec![vec![
                    [-20.0, -20.0],
                    [19.8, -20.0],
                    [19.8, 20.0],
                    [-20.0, 20.0],
                ]],
                height_m: 60.0,
                min_height_m: 0.0,
                seed: 43,
                kind: BuildingKind::Part,
                roof_shape: Some(RoofShape::Flat),
                use_kind: Default::default(),
            });
            let overlay = tier0::build_overlay(
                frame.addr.cell,
                &frame,
                &pack,
                &|_, _| 100.0,
                0,
                None,
                None,
                None,
            );
            assert_eq!(overlay.source_buildings[0].height, 60.0);
            assert_eq!(overlay.source_buildings[0].source_total_height, 160.0);
            let result = generate_test(&frame, &pack, &|_, _, _| Some(100.0));
            assert!(result.complete);
            let render_top = result
                .mesh
                .vertices
                .iter()
                .map(|v| world_height(&frame, v.pos))
                .max_by(f64::total_cmp)
                .unwrap();
            let metric = crate::collision::MetricFrame::of(&frame);
            let collision_top = result
                .collision
                .iter()
                .flat_map(|t| t.vertices)
                .map(|p| ellipsoid::ecef_to_geodetic(metric.point_to_ecef(p.map(f64::from))).2)
                .max_by(f64::total_cmp)
                .unwrap();
            // A fine cell may contain only the recessed deck, with the parapet outside.
            assert!(
                (158.0..=160.03).contains(&render_top),
                "render top {render_top}"
            );
            assert!(
                (collision_top - 160.0).abs() < 0.03,
                "collision top {collision_top}"
            );
        }
    }

    #[test]
    fn highrise_panels_preserve_collision_and_height_across_resolution_and_use() {
        use vearth_data::vectors::BuildingUse;
        for level in [15, 16, 18, 19] {
            let frame = frame(level);
            for usage in [BuildingUse::Commercial, BuildingUse::Residential] {
                let mut pack = pack_with(vec![square(-12.0, 12.0), square(-2.0, 2.0)], 180.0);
                pack.buildings[0].use_kind = usage;
                pack.buildings[0].roof_shape = Some(RoofShape::Flat);
                let mut overlay = tier0::build_overlay(
                    frame.addr.cell,
                    &frame,
                    &pack,
                    &|_, _| 100.0,
                    0,
                    None,
                    None,
                    Some(&|_| true),
                );
                let render = |overlay: &CellOverlay| {
                    generate(
                        &frame,
                        &pack,
                        overlay,
                        &[u32::MAX; N],
                        &|_, _, _| Some(100.0),
                        &|_, _, _| None,
                    )
                    .unwrap()
                };
                let actual = render(&overlay);
                assert!(actual.complete);
                assert!(
                    actual
                        .mesh
                        .vertices
                        .iter()
                        .any(|v| crate::highrise::is_facade(((v.face_ao >> 5) & 255) as u8))
                );
                for detail in &actual.details {
                    assert!(detail.mesh.vertices.len() <= 4096);
                    assert!(
                        detail
                            .mesh
                            .vertices
                            .iter()
                            .all(|v| world_height(&frame, v.pos) <= 280.02)
                    );
                }
                for stamp in &mut overlay.source_buildings {
                    stamp.highrise = None;
                    // Disable the resolved grammar too for the ordinary-window control.
                    stamp.design.facade = None;
                }
                let old = render(&overlay);
                assert_eq!(
                    actual.collision, old.collision,
                    "collision changed at L{level}/{usage:?}"
                );
                assert!(
                    actual
                        .mesh
                        .vertices
                        .iter()
                        .all(|v| v.pos.iter().all(|x| x.is_finite()))
                );
                assert!(
                    actual
                        .mesh
                        .vertices
                        .iter()
                        .all(|v| world_height(&frame, v.pos) <= 280.02)
                );
                if level >= 18 {
                    assert!(
                        actual.mesh.indices.len() < old.mesh.indices.len(),
                        "L{level}/{usage:?}: panels={}, ordinary={}",
                        actual.mesh.indices.len(),
                        old.mesh.indices.len()
                    );
                }
                if level == 15 {
                    assert!(actual.mesh.indices.len() <= old.mesh.indices.len());
                }
            }
        }
    }

    #[test]
    fn highrise_panels_continue_through_small_clipped_edge_fragments() {
        let frame = frame(18);
        let (lat, lon) = frame.column_latlon(0.5, 0.5);
        let projection = Projection {
            frame: &frame,
            lat,
            lon,
        };
        let pack = pack_with(vec![square(-12.0, 12.0)], 180.0);
        let stamp = facade_fixture(&frame, &pack);
        let system = crate::highrise::System::ClearOffice;
        let split = DVec2::new(-11.3, -12.0);
        let expected = projection
            .point(split + DVec2::new(0.0, -0.018), 104.0)
            .as_vec3();
        for edge in [
            [DVec2::new(-12.0, -12.0), split],
            [split, DVec2::new(12.0, -12.0)],
        ] {
            let mut mesh = MeshBuffers::default();
            emit_highrise_panels(
                &projection,
                &mut mesh,
                edge,
                100.0,
                280.0,
                &stamp,
                &pack.buildings[0].rings,
                system,
            );
            assert!(
                mesh.vertices
                    .iter()
                    .any(|v| { glam::Vec3::from_array(v.pos).abs_diff_eq(expected, 0.001) }),
                "clipped panel left a gap at the source-edge split"
            );
        }
    }

    #[test]
    fn all_highrise_panels_keep_the_resolved_opaque_pigment() {
        let frame = frame(18);
        let (lat, lon) = frame.column_latlon(0.5, 0.5);
        let projection = Projection {
            frame: &frame,
            lat,
            lon,
        };
        let pack = pack_with(vec![square(-12.0, 12.0)], 180.0);
        let mut stamp = facade_fixture(&frame, &pack);
        stamp.seed = 4 << 23; // Unit shade isolates ownership of the pigment.
        stamp.design.wall_material = crate::material::PLASTER_WALL;
        for wall in [[224, 156, 64], [98, 157, 198]] {
            stamp.design.wall = wall;
            let expected =
                crate::art_direction::architecture_surface(stamp.design.wall_material, wall);
            for system in crate::highrise::System::ALL {
                stamp.highrise = Some(system);
                stamp.design.facade = Some(system);
                stamp.design.joinery = [31, 83, 147];
                assert_eq!(facade_trim(&stamp).tint, stamp.design.joinery);
                let mut mesh = MeshBuffers::default();
                emit_highrise_panels(
                    &projection,
                    &mut mesh,
                    [DVec2::new(-12.0, -12.0), DVec2::new(12.0, -12.0)],
                    100.0,
                    280.0,
                    &stamp,
                    &pack.buildings[0].rings,
                    system,
                );
                assert!(!mesh.vertices.is_empty());
                assert!(
                    mesh.vertices
                        .iter()
                        .all(|v| v.rgba[..3] == expected && (v.face_ao >> 13) & 3 == 0)
                );
            }
        }
    }

    #[test]
    fn rooftop_service_box_respects_courtyards_exclusions_and_source_top() {
        let frame = frame(19);
        let (lat, lon) = frame.column_latlon(0.5, 0.5);
        let projection = Projection {
            frame: &frame,
            lat,
            lon,
        };
        let outer = vec![square(-12.0, 12.0)];
        let hole = vec![square(-0.5, 0.5)];
        let pack = pack_with(outer.clone(), 180.0);
        let mut stamp = facade_fixture(&frame, &pack);
        stamp.highrise = Some(crate::highrise::System::ClearOffice);
        let roof = FlatRoof {
            deck: 278.4,
            top: 280.0,
            thickness: 0.16,
            stepped: false,
        };
        let clear = Footprint::new(vec![&outer], vec![]).unwrap();
        let excluded = Footprint::new(vec![&outer], vec![&hole]).unwrap();
        for (domain, expected) in [(&clear, true), (&excluded, false)] {
            let mut details = MeshBuffers::default();
            emit_highrise_roof_plant(
                &projection,
                &mut details,
                domain,
                &pack.buildings[0],
                roof,
                &stamp,
            );
            assert_eq!(!details.indices.is_empty(), expected);
            assert!(details.vertices.len() <= 30);
            assert!(
                details
                    .vertices
                    .iter()
                    .all(|v| (278.39..280.0).contains(&world_height(&frame, v.pos)))
            );
        }
    }

    #[test]
    fn source_envelope_has_level_roof_deck_on_sloped_ground_and_clips_to_cell() {
        let frame = frame(19);
        let pack = pack_with(vec![square(-20.0, 20.0)], 36.0);
        let (lat, lon) = frame.column_latlon(0.5, 0.5);
        let result = generate_test(&frame, &pack, &|lt, lng, hint| {
            assert_eq!(hint, crate::datum_pitch_m(lt) as f32);
            Some((100.0 + (lng - lon) * MEAN_RADIUS * lat.cos() * 0.2) as f32)
        });
        assert!(!result.collision.is_empty());
        assert!(result.mesh.vertices.iter().all(|v| v.pos[0] >= 0.0
            && v.pos[0] <= N as f32
            && v.pos[2] >= 0.0
            && v.pos[2] <= N as f32));
        let roofs: Vec<_> = result
            .mesh
            .vertices
            .iter()
            .filter(|v| (v.face_ao & 7) == 2)
            .map(|v| world_height(&frame, v.pos))
            .collect();
        assert!(!roofs.is_empty());
        let deck = 136.0 - (1.6 + ((pack.buildings[0].seed >> 17) & 3) as f64 * (0.20 / 3.0));
        for height in roofs {
            assert!(
                (height - deck).abs() < 0.01 || (height - 136.0).abs() < 0.01,
                "roof height {height}"
            );
        }
    }
    #[test]
    fn subcolumn_building_has_direct_geometry_and_wall_collision() {
        let frame = frame(19);
        let pack = pack_with(vec![square(-0.08, 0.08)], 4.0);
        let overlay = tier0::build_overlay(
            frame.addr.cell,
            &frame,
            &pack,
            &|_, _| 100.0,
            0,
            None,
            None,
            None,
        );
        assert!(
            overlay
                .cols
                .iter()
                .flatten()
                .all(|column| column.building.is_none())
        );
        assert_eq!(overlay.source_buildings.len(), 1);
        let result = generate(
            &frame,
            &pack,
            &overlay,
            &[u32::MAX; N],
            &|_, _, _| Some(100.0),
            &|_, _, _| None,
        )
        .unwrap();
        assert!(!result.mesh.indices.is_empty());
        let metric = crate::collision::MetricFrame::of(&frame);
        let (lat, lon) = frame.column_latlon(0.5, 0.5);
        let projection = Projection {
            frame: &frame,
            lat,
            lon,
        };
        let point = |en, h| metric.point_from_ecef(frame.local_to_ecef(projection.point(en, h)));
        let collision = crate::collision::SurfaceCollision::from_triangles(result.collision);
        let center = point(DVec2::ZERO, 102.0);
        assert!(collision.contains(center[0], center[1], center[2]));
        let origin = DVec3::from_array(point(DVec2::new(-1.0, 0.0), 102.0));
        let direction = (DVec3::from_array(center) - origin).normalize();
        let hit = collision
            .raycast(origin.to_array(), direction.to_array(), 2.0)
            .unwrap();
        assert!((hit - 0.92).abs() < 0.01, "wall hit {hit}");
    }

    #[test]
    fn collision_prisms_do_not_duplicate_refined_visual_roofs() {
        let frame = frame(16);
        let mut pack = pack_with(vec![square(-35.0, 35.0), square(-8.0, 8.0)], 24.0);
        pack.buildings[0].roof_shape = Some(RoofShape::Gabled);
        let result = generate_test(&frame, &pack, &|_, _, _| Some(100.0));
        assert!(
            result.collision.len() <= 64,
            "simple source courtyard must keep a bounded collider: {}",
            result.collision.len()
        );
        assert!(result.mesh.indices.len() / 3 > result.collision.len() * 2);
    }

    #[test]
    fn continuous_collision_keeps_courtyard_and_raised_part_open() {
        let frame = frame(18);
        let mut pack = pack_with(vec![square(-12.0, 12.0), square(-4.0, 4.0)], 24.0);
        pack.buildings[0].kind = BuildingKind::Part;
        pack.buildings[0].min_height_m = 10.0;
        let result = generate_test(&frame, &pack, &|_, _, _| Some(100.0));
        let collision = crate::collision::SurfaceCollision::from_triangles(result.collision);
        let metric = crate::collision::MetricFrame::of(&frame);
        let (lat, lon) = frame.column_latlon(0.5, 0.5);
        let projection = Projection {
            frame: &frame,
            lat,
            lon,
        };
        let inside = |en, h| {
            let p = metric.point_from_ecef(frame.local_to_ecef(projection.point(en, h)));
            collision.contains(p[0], p[1], p[2])
        };
        assert!(!inside(DVec2::ZERO, 116.0), "courtyard filled");
        assert!(
            !inside(DVec2::new(8.0, 0.0), 108.0),
            "raised underside filled"
        );
        assert!(inside(DVec2::new(8.0, 0.0), 116.0));
        assert!(!inside(DVec2::new(8.0, 0.0), 126.0));
    }

    #[test]
    fn coalesced_source_slabs_preserve_crossings_holes_and_exact_source_segments() {
        let mut lake = vec![square(-12.0, 12.0)];
        for i in 0..16 {
            let x = -10.0 + (i % 4) as f32 * 6.0;
            let y = -10.0 + (i / 4) as f32 * 6.0 + i as f32 * 0.015;
            lake.push(vec![[x, y], [x + 2.0, y], [x + 2.0, y + 2.0], [x, y + 2.0]]);
        }
        let cases = [
            lake,
            vec![square(-10.0, 3.0), square(-3.0, 10.0)],
            vec![vec![[-8.0, -7.0], [7.0, 9.0], [-7.0, 9.0], [8.0, -7.0]]],
            vec![square(-9.0, 9.0), square(-9.0, 9.0)],
        ];
        for (case, rings) in cases.iter().enumerate() {
            for step in [1.5, 1.0e9] {
                let old = source_polygon_cells_clipped(
                    rings,
                    DVec2::splat(-11.0),
                    DVec2::splat(11.0),
                    step,
                )
                .unwrap();
                let new = source_polygon_cells_clipped_coalesced(
                    rings,
                    DVec2::splat(-11.0),
                    DVec2::splat(11.0),
                    step,
                )
                .unwrap();
                assert!((area(&old) - area(&new)).abs() < 1.0e-8);
                assert!(new.len() <= old.len());
                if case == 0 && step > 100.0 {
                    assert!(new.len() < old.len());
                }
                let cover = |cells: &[[DVec2; 4]], point: DVec2| {
                    cells
                        .iter()
                        .filter(|cell| {
                            (0..4).all(|i| {
                                let a = cell[i];
                                let b = cell[(i + 1) % 4];
                                (b - a).perp_dot(point - a) >= 0.0
                            })
                        })
                        .count()
                };
                for i in 0..4096 {
                    let point = DVec2::new(
                        ((i * 719 + 3) % 10007) as f64 / 10007.0,
                        ((i * 313 + 7) % 9973) as f64 / 9973.0,
                    ) * 22.0
                        - DVec2::splat(11.0);
                    assert_eq!(cover(&old, point), cover(&new, point));
                    assert_eq!(cover(&new, point) > 0, contains(rings, point));
                }
                for cell in &new {
                    assert!(cell[2].y - cell[0].y <= step + 1.0e-9);
                    assert!((cell[1].x - cell[0].x).max(cell[2].x - cell[3].x) <= step + 1.0e-9);
                }
            }
        }
    }

    #[test]
    fn coalesced_source_slabs_keep_original_clipping_failure_contract() {
        for rings in [
            vec![vec![[0.0, 0.0]; 3]],
            vec![vec![[0.0, 0.0], [1.0, 1.0]]],
            vec![vec![[f32::NAN, 0.0], [0.0, 1.0], [1.0, 0.0]]],
            vec![vec![[1.0, 1.0], [2.0, 1.0], [1.0, 2.0]]],
            vec![square(-0.5, 0.5)],
        ] {
            for step in [0.0, f64::NAN, 1.0e9] {
                let old = source_polygon_cells_clipped(
                    &rings,
                    DVec2::splat(-1.0),
                    DVec2::splat(1.0),
                    step,
                );
                let new = source_polygon_cells_clipped_coalesced(
                    &rings,
                    DVec2::splat(-1.0),
                    DVec2::splat(1.0),
                    step,
                );
                assert_eq!(old.is_some(), new.is_some());
                if let (Some(old), Some(new)) = (old, new) {
                    assert!((area(&old) - area(&new)).abs() < 1.0e-10);
                }
            }
        }
    }

    #[test]
    fn large_clipped_polygon_keeps_hole_and_source_area() {
        let mut outer = Vec::new();
        for i in 0..2048 {
            let angle = i as f32 / 2048.0 * std::f32::consts::TAU;
            outer.push([angle.cos() * 1000.0, angle.sin() * 1000.0]);
        }
        let rings = vec![outer, square(-4.0, 4.0)];
        let cells =
            source_polygon_cells_clipped(&rings, DVec2::splat(-16.0), DVec2::splat(16.0), 1e9)
                .unwrap();
        assert!((area(&cells) - (32.0 * 32.0 - 8.0 * 8.0)).abs() < 0.001);
        assert!(
            cells
                .iter()
                .flatten()
                .all(|p| p.abs().cmple(DVec2::splat(16.0)).all())
        );
    }

    fn assert_same_building_surface(actual: &BuildingSurfaceData, expected: &BuildingSurfaceData) {
        assert_eq!(actual.complete, expected.complete);
        assert_eq!(actual.shadow_omissions, expected.shadow_omissions);
        assert_eq!(actual.mesh.vertices, expected.mesh.vertices);
        assert_eq!(actual.mesh.indices, expected.mesh.indices);
        assert_eq!(actual.collision, expected.collision);
        assert_eq!(actual.details.len(), expected.details.len());
        for (actual, expected) in actual.details.iter().zip(&expected.details) {
            assert_eq!(actual.mesh.vertices, expected.mesh.vertices);
            assert_eq!(actual.mesh.indices, expected.mesh.indices);
            assert_eq!(actual.bound_center, expected.bound_center);
            assert_eq!(actual.radius_m, expected.radius_m);
            assert_eq!(actual.min_screen_px, expected.min_screen_px);
        }
    }

    #[test]
    fn shared_envelope_plan_keeps_exact_retry_meshes_and_samples_datums_once() {
        let frame = frame(17);
        for courtyard in [false, true] {
            let mut pack = if courtyard {
                pack_with(vec![square(-35.0, 35.0), square(-9.0, 9.0)], 24.0)
            } else {
                let mut pack = FeaturePack::empty();
                for i in 0..8 {
                    let x = -40.0 + (i % 4) as f32 * 12.0;
                    let z = -20.0 + (i / 4) as f32 * 24.0;
                    let mut owner = pack_with(
                        vec![vec![
                            [x, z],
                            [x + 12.0, z],
                            [x + 12.0, z + 14.0],
                            [x, z + 14.0],
                        ]],
                        15.0,
                    );
                    owner.buildings[0].seed = 100 + i;
                    pack.buildings.extend(owner.buildings);
                }
                pack
            };
            if courtyard {
                let mut part = pack_with(
                    vec![vec![
                        [-32.0, -30.0],
                        [-12.0, -30.0],
                        [-12.0, 28.0],
                        [-32.0, 28.0],
                    ]],
                    32.0,
                );
                part.buildings[0].kind = BuildingKind::Part;
                part.buildings[0].min_height_m = 6.0;
                part.buildings[0].seed = 999;
                pack.buildings.extend(part.buildings);
            }
            let overlay = tier0::build_overlay(
                frame.addr.cell,
                &frame,
                &pack,
                &|_, _| 100.0,
                0,
                None,
                None,
                Some(&|_| true),
            );
            for budget in [1, usize::MAX] {
                let calls = std::cell::Cell::new(0usize);
                let elevation = |_, _, _| {
                    calls.set(calls.get() + 1);
                    Some(100.0)
                };
                let surface = |_, _, _| None;
                let actual = generate_bounded(
                    &frame,
                    &pack,
                    &overlay,
                    &[u32::MAX; N],
                    &elevation,
                    &surface,
                    budget,
                )
                .unwrap();
                let shared_calls = calls.replace(0);
                let mut expected = generate_envelopes::<true>(
                    &frame,
                    &pack,
                    &overlay,
                    &[u32::MAX; N],
                    &elevation,
                    &surface,
                    false,
                    budget,
                )
                .unwrap();
                let retry = crate::built_surface::mesh_bytes(&expected.mesh) > budget;
                assert_eq!(
                    retry,
                    budget == 1,
                    "fixture must exercise both budget paths"
                );
                if retry {
                    expected = generate_envelopes::<true>(
                        &frame,
                        &pack,
                        &overlay,
                        &[u32::MAX; N],
                        &elevation,
                        &surface,
                        true,
                        budget,
                    )
                    .unwrap();
                    crate::built_surface::compact_mesh(&mut expected.mesh);
                }
                assert_same_building_surface(&actual, &expected);
                assert!(shared_calls > 0);
                assert_eq!(calls.get(), shared_calls * if retry { 2 } else { 1 });
            }
        }
    }

    #[test]
    fn envelope_plan_retains_missing_data_and_lazy_source_boundaries() {
        let frame = frame(17);
        let pack = pack_with(vec![square(-12.0, 12.0)], 18.0);
        let overlay = tier0::build_overlay(
            frame.addr.cell,
            &frame,
            &pack,
            &|_, _| 100.0,
            0,
            None,
            None,
            Some(&|_| true),
        );
        let missing = prepare_envelopes(
            &frame,
            &pack,
            &overlay,
            &[u32::MAX; N],
            &|_, _, _| None,
            &|_, _, _| None,
        )
        .unwrap();
        assert!(!missing.complete);
        assert!(missing.datums.iter().all(Option::is_none));
        assert!(
            missing
                .domains
                .iter()
                .flatten()
                .all(|domain| domain.boundary.get().is_none())
        );
        let result = generate_prepared_envelopes::<true>(
            &frame,
            &pack,
            &overlay,
            &[u32::MAX; N],
            &|_, _, _| None,
            &|_, _, _| None,
            false,
            usize::MAX,
            &missing,
        )
        .unwrap();
        assert!(!result.complete);
        assert!(result.mesh.vertices.is_empty());
        let plan = prepare_envelopes(
            &frame,
            &pack,
            &overlay,
            &[u32::MAX; N],
            &|_, _, _| Some(100.0),
            &|_, _, _| None,
        )
        .unwrap();
        let owner = plan.domains[0].as_ref().unwrap();
        assert!(owner.boundary.get().is_none());
        for simple in [false, true] {
            let result = generate_prepared_envelopes::<true>(
                &frame,
                &pack,
                &overlay,
                &[u32::MAX; N],
                &|_, _, _| Some(100.0),
                &|_, _, _| None,
                simple,
                usize::MAX,
                &plan,
            )
            .unwrap();
            assert!(result.complete);
            assert_eq!(owner.boundary.get().unwrap(), &owner.footprint.boundary());
        }
    }

    #[test]
    fn budget_retry_keeps_a_continuous_courtyard_envelope() {
        let frame = frame(18);
        let pack = pack_with(vec![square(-40.0, 40.0), square(-12.0, 12.0)], 36.0);
        let overlay = tier0::build_overlay(
            frame.addr.cell,
            &frame,
            &pack,
            &|_, _| 100.0,
            0,
            None,
            None,
            Some(&|_| true),
        );
        let elevation = |_, _, _| Some(100.0);
        let surface = |_, _, _| None;
        let mut simple = generate_envelopes::<true>(
            &frame,
            &pack,
            &overlay,
            &[u32::MAX; N],
            &elevation,
            &surface,
            true,
            usize::MAX,
        )
        .unwrap();
        crate::built_surface::compact_mesh(&mut simple.mesh);
        let budget = crate::built_surface::mesh_bytes(&simple.mesh);
        let detailed = generate(
            &frame,
            &pack,
            &overlay,
            &[u32::MAX; N],
            &elevation,
            &surface,
        )
        .unwrap();
        assert!(crate::built_surface::mesh_bytes(&detailed.mesh) > budget);
        let result = generate_bounded(
            &frame,
            &pack,
            &overlay,
            &[u32::MAX; N],
            &elevation,
            &surface,
            budget,
        )
        .unwrap();
        assert!(crate::built_surface::mesh_bytes(&result.mesh) <= budget);
        assert!(!result.mesh.indices.is_empty());
        assert!(result.details.is_empty());
        let (lat, lon) = frame.column_latlon(0.5, 0.5);
        let projection = Projection {
            frame: &frame,
            lat,
            lon,
        };
        let courtyard = projection.point(DVec2::ZERO, 136.0);
        for indices in result.mesh.indices.chunks_exact(3) {
            let points = indices
                .iter()
                .map(|&i| result.mesh.vertices[i as usize].pos)
                .collect::<Vec<_>>();
            let center = points
                .iter()
                .map(|p| DVec2::new(f64::from(p[0]), f64::from(p[2])))
                .sum::<DVec2>()
                / 3.0;
            let radius = 8.0 / frame.grid_scale.x.max(frame.grid_scale.z);
            assert!(center.distance(DVec2::new(courtyard.x, courtyard.z)) > radius);
        }
    }

    #[test]
    fn shadowed_pack_buildings_do_not_repeat_source_datum_queries() {
        let frame = frame(18);
        let mut pack = pack_with(vec![square(-60.0, 60.0)], 36.0);
        let baseline_calls = std::cell::Cell::new(0);
        let baseline = generate_test(&frame, &pack, &|_, _, _| {
            baseline_calls.set(baseline_calls.get() + 1);
            Some(100.0)
        });
        for i in 0..128 {
            let x = (i % 16) as f32 * 6.0 - 47.0;
            let y = (i / 16) as f32 * 6.0 - 23.0;
            pack.buildings.push(Building {
                location_rad: None,
                rings: vec![vec![[x, y], [x + 2.0, y], [x + 2.0, y + 2.0], [x, y + 2.0]]],
                height_m: 4.0,
                min_height_m: 0.0,
                seed: 1000 + i,
                kind: BuildingKind::Building,
                roof_shape: None,
                use_kind: Default::default(),
            });
        }
        let expanded_calls = std::cell::Cell::new(0);
        let expanded = generate_test(&frame, &pack, &|_, _, _| {
            expanded_calls.set(expanded_calls.get() + 1);
            Some(100.0)
        });
        assert!(baseline.complete && expanded.complete);
        assert!(baseline_calls.get() > 0);
        assert_eq!(
            expanded_calls.get(),
            baseline_calls.get(),
            "nonwinning pack records must not cause extra raster sampling"
        );
        assert_eq!(expanded.mesh.vertices, baseline.mesh.vertices);
        assert_eq!(expanded.mesh.indices, baseline.mesh.indices);
        assert_eq!(expanded.collision, baseline.collision);
    }

    #[test]
    fn a_cell_without_building_owners_does_not_sample_pack_datums() {
        let frame = frame(18);
        let pack = pack_with(vec![square(-60.0, 60.0)], 36.0);
        assert!(
            generate(
                &frame,
                &pack,
                &CellOverlay::empty(),
                &[u32::MAX; N],
                &|_, _, _| panic!("unowned source queried elevation"),
                &|_, _, _| panic!("unowned source queried surface"),
            )
            .is_none()
        );
    }

    #[test]
    fn fully_curated_cells_do_not_sample_hidden_building_datums() {
        let frame = frame(18);
        let pack = pack_with(vec![square(-60.0, 60.0)], 36.0);
        let overlay = tier0::build_overlay(
            frame.addr.cell,
            &frame,
            &pack,
            &|_, _| 100.0,
            0,
            None,
            None,
            None,
        );
        assert!(
            overlay
                .cols
                .iter()
                .flatten()
                .any(|column| column.building.is_some())
        );
        assert!(
            generate(
                &frame,
                &pack,
                &overlay,
                &[0; N],
                &|_, _, _| panic!("fully hidden source queried elevation"),
                &|_, _, _| panic!("fully hidden source queried surface"),
            )
            .is_none()
        );
    }

    #[test]
    fn full_source_facades_use_metric_details_before_finest_terrain_level() {
        let frame = frame(18);
        let pack = pack_with(vec![square(-12.0, 12.0)], 36.0);
        let result = generate_shaded_facade_test(&frame, &pack);
        assert!(frame.addr.cell.level < vearth_core::cell::MAX_LEVEL);
        assert!(frame.grid_scale.y > 1.0);
        assert!(!result.details.is_empty());
        assert!(result.mesh.vertices.iter().any(|v| matches!(
            ((v.face_ao >> 5) & 255) as u8,
            crate::material::WINDOW | crate::material::LIT_WINDOW
        )));
        assert!(
            result
                .details
                .iter()
                .all(|d| d.radius_m > 10.0 && d.min_screen_px > 0.0)
        );
    }
    #[test]
    fn continuous_source_roofs_do_not_fill_courtyards() {
        let frame = frame(18);
        let pack = pack_with(vec![square(-12.0, 12.0), square(-5.0, 5.0)], 36.0);
        let result = generate_test(&frame, &pack, &|_, _, _| Some(100.0));
        let (lat, lon) = frame.column_latlon(0.5, 0.5);
        for triangle in result.mesh.indices.chunks_exact(3) {
            let vertices = [triangle[0], triangle[1], triangle[2]]
                .map(|index| result.mesh.vertices[index as usize]);
            if !is_roof_vertex(&vertices[0]) {
                continue;
            }
            let p = vertices
                .iter()
                .map(|v| DVec3::from_array(v.pos.map(f64::from)))
                .sum::<DVec3>()
                / 3.0;
            let (p_lat, p_lon, _) = ellipsoid::ecef_to_geodetic(frame.local_to_ecef(p));
            let en = DVec2::new(
                (p_lon - lon) * MEAN_RADIUS * lat.cos(),
                (p_lat - lat) * MEAN_RADIUS,
            );
            assert!(
                en.x.abs() > 4.5 || en.y.abs() > 4.5,
                "courtyard roof at {en}"
            );
        }
    }
    #[test]
    fn gabled_roof_uses_footprint_axis_and_keeps_the_gable_end_ridge() {
        let rings = vec![vec![[-10.0, -4.0], [10.0, -4.0], [10.0, 4.0], [-10.0, 4.0]]];
        let roof = RoofField::new(&rings, Some(RoofShape::Gabled), 8.0, 10.0, 0.6);
        assert!((roof.height(DVec2::ZERO) - 10.0).abs() < 0.01);
        assert!((roof.height(DVec2::new(10.0, 0.0)) - 10.0).abs() < 0.01);
        assert!((roof.height(DVec2::new(0.0, 4.0)) - 8.0).abs() < 0.01);
        let angle = 0.63f64;
        let rotate = |p: [f32; 2]| {
            let p = DVec2::from_array(p.map(f64::from));
            [
                (p.x * angle.cos() - p.y * angle.sin()) as f32,
                (p.x * angle.sin() + p.y * angle.cos()) as f32,
            ]
        };
        let rotated: Rings = rings
            .iter()
            .map(|r| r.iter().copied().map(rotate).collect())
            .collect();
        let rotated = RoofField::new(&rotated, Some(RoofShape::Gabled), 8.0, 10.0, 0.6);
        for p in [[0.0, 0.0], [10.0, 0.0], [0.0, 3.0]] {
            assert!(
                (roof.height(DVec2::from_array(p.map(f64::from)))
                    - rotated.height(DVec2::from_array(rotate(p).map(f64::from))))
                .abs()
                    < 0.01
            );
        }
    }
    #[test]
    fn prepared_building_cap_contains_all_datum_and_roof_boundary_queries() {
        let frame = frame(19);
        let pack = pack_with(
            vec![vec![
                [-800.0, -10.0],
                [800.0, -10.0],
                [800.0, 10.0],
                [-800.0, 10.0],
            ]],
            36.0,
        );
        let cap = datum_cover(&frame, &pack);
        assert!(!cap.is_empty());
        let result = generate_test(&frame, &pack, &|lat, lon, hint| {
            assert!(cap.contains(lat, lon));
            assert_eq!(hint, crate::datum_pitch_m(lat) as f32);
            Some(100.0)
        });
        assert!(!result.mesh.vertices.is_empty());
    }
    #[test]
    fn mapped_parts_share_parent_datum_and_replace_parent_roof_only_in_their_footprint() {
        let frame = frame(17);
        let mut pack = pack_with(vec![square(-20.0, 20.0)], 36.0);
        pack.buildings[0].roof_shape = Some(RoofShape::Flat);
        pack.buildings.push(Building {
            location_rad: None,
            rings: vec![vec![[2.0, -4.0], [10.0, -4.0], [10.0, 4.0], [2.0, 4.0]]],
            height_m: 48.0,
            min_height_m: 12.0,
            seed: 77,
            kind: BuildingKind::Part,
            roof_shape: Some(RoofShape::Flat),
            use_kind: Default::default(),
        });
        let (lat, lon) = frame.column_latlon(0.5, 0.5);
        let result = generate_test(&frame, &pack, &|_, lng, _| {
            Some((100.0 + (lng - lon) * MEAN_RADIUS * lat.cos() * 0.2) as f32)
        });
        let overlay = tier0::build_overlay(
            frame.addr.cell,
            &frame,
            &pack,
            &|_, _| 100.0,
            0,
            None,
            None,
            None,
        );
        let part_stamp = overlay
            .source_buildings
            .iter()
            .find(|stamp| stamp.seed == 77)
            .unwrap();
        let part_deck = FlatRoof::for_source(&pack.buildings[1], part_stamp, 112.0, 148.0)
            .unwrap()
            .with_resolution(frame.addr.cell.sample_pitch_m() / 3.0)
            .deck;
        let mut part_roofs = 0;
        for triangle in result.mesh.indices.chunks_exact(3) {
            let vertices =
                [triangle[0], triangle[1], triangle[2]].map(|i| result.mesh.vertices[i as usize]);
            if vertices[0].face_ao & 7 != 2 {
                continue;
            }
            let point = vertices
                .iter()
                .map(|v| DVec3::from_array(v.pos.map(f64::from)))
                .sum::<DVec3>()
                / 3.0;
            let (p_lat, p_lon, height) = ellipsoid::ecef_to_geodetic(frame.local_to_ecef(point));
            let en = DVec2::new(
                (p_lon - lon) * MEAN_RADIUS * lat.cos(),
                (p_lat - lat) * MEAN_RADIUS,
            );
            if en.x > 3.0 && en.x < 9.0 && en.y.abs() < 3.0 {
                assert!(
                    (height - part_deck).abs() < 0.02,
                    "unsupported parent roof or shifted part datum {height}"
                );
                part_roofs += 1;
            }
        }
        assert!(part_roofs > 0);
    }
    #[test]
    fn lidar_envelope_preserves_measured_absolute_roof_and_source_outline() {
        let frame = frame(18);
        let pack = pack_with(vec![square(-8.0, 8.0)], 36.0);
        let mut ov = tier0::build_overlay(
            frame.addr.cell,
            &frame,
            &pack,
            &|_, _| 100.0,
            0,
            None,
            None,
            Some(&|_| true),
        );
        for stamp in &mut ov.source_buildings {
            stamp.lidar = true;
            stamp.top_geo = 112.0;
            stamp.height = 12.0;
        }
        let (lat, lon) = frame.column_latlon(0.5, 0.5);
        let cap = whole_source_cap(&frame, &pack).unwrap();
        let measured = |lt, lng, _| {
            assert!(cap.contains(lt, lng));
            Some((112.0 + (lng - lon) * MEAN_RADIUS * lat.cos() * 0.2) as f32)
        };
        let result = generate(
            &frame,
            &pack,
            &ov,
            &[u32::MAX; N],
            &|_, _, _| Some(100.0),
            &measured,
        )
        .unwrap();
        let roofs: Vec<_> = result
            .mesh
            .vertices
            .iter()
            .filter(|v| is_roof_vertex(v))
            .collect();
        assert!(!roofs.is_empty());
        for vertex in roofs {
            let (_, lng, h) = ellipsoid::ecef_to_geodetic(
                frame.local_to_ecef(DVec3::from_array(vertex.pos.map(f64::from))),
            );
            let expected = 112.0 + (lng - lon) * MEAN_RADIUS * lat.cos() * 0.2;
            assert!(
                (h - expected).abs() < 0.02,
                "measured roof {h} expected {expected}"
            );
        }
    }
    #[test]
    fn curated_ownership_cuts_source_surfaces_and_collision() {
        let frame = frame(18);
        let pack = pack_with(vec![square(-8.0, 8.0)], 36.0);
        let ov = tier0::build_overlay(
            frame.addr.cell,
            &frame,
            &pack,
            &|_, _| 100.0,
            0,
            None,
            None,
            None,
        );
        let mut allowed = [u32::MAX; N];
        for row in &mut allowed {
            *row &= !(1 << 16);
        }
        let result = generate(
            &frame,
            &pack,
            &ov,
            &allowed,
            &|_, _, _| Some(100.0),
            &|_, _, _| None,
        )
        .unwrap();
        for mesh in std::iter::once(&result.mesh).chain(result.details.iter().map(|d| &d.mesh)) {
            for triangle in mesh.indices.chunks_exact(3) {
                let x = triangle
                    .iter()
                    .map(|i| mesh.vertices[*i as usize].pos[0])
                    .sum::<f32>()
                    / 3.0;
                assert!(x <= 16.0001 || x >= 16.9999, "curated overlap at {x}");
            }
        }
    }
    #[test]
    fn retained_building_foundation_follows_curated_osaka_plaza() {
        use vearth_core::cell::{CellId, ChunkAddr};
        let lm = crate::landmarks::LANDMARKS
            .iter()
            .find(|lm| lm.name == "osaka-castle")
            .unwrap();
        let (latitude, longitude) = (lm.lat_deg.to_radians(), lm.lon_deg.to_radians());
        let (sh, ch) = lm.heading_deg.to_radians().sin_cos();
        let latitude = latitude + (-40.0 * sh - 30.0 * ch) / MEAN_RADIUS;
        let longitude = longitude + (40.0 * ch - 30.0 * sh) / (MEAN_RADIUS * latitude.cos());
        let frame = ChunkFrame::of(ChunkAddr {
            cell: CellId::from_dir(
                ellipsoid::geodetic_to_ecef(latitude, longitude, 0.0).normalize(),
                18,
            ),
            k: 0,
        });
        let (center_lat, center_lon) = frame.column_latlon(0.5, 0.5);
        let offset = DVec2::new(
            (longitude - center_lon) * MEAN_RADIUS * center_lat.cos(),
            (latitude - center_lat) * MEAN_RADIUS,
        );
        let ring = square(-3.0, 3.0)
            .into_iter()
            .map(|p| {
                (DVec2::from_array(p.map(f64::from)) + offset)
                    .as_vec2()
                    .to_array()
            })
            .collect();
        let pack = pack_with(vec![ring], 10.0);
        let result = generate_test(&frame, &pack, &|lat, lon, _| {
            Some(crate::landmarks::corrected_source_building_ground(
                lat, lon, 55.0,
            ))
        });
        let bottom = result
            .mesh
            .vertices
            .iter()
            .map(|v| world_height(&frame, v.pos))
            .min_by(f64::total_cmp)
            .unwrap();
        assert!(
            (bottom - 30.85).abs() < 0.03,
            "unsupported plaza foundation {bottom}"
        );
        assert!(result.complete);
    }

    #[test]
    fn foundation_reaches_downhill_source_boundary_without_tilting_floor_datum() {
        let frame = frame(17);
        let pack = pack_with(vec![square(-20.0, 20.0)], 36.0);
        let (lat, lon) = frame.column_latlon(0.5, 0.5);
        let result = generate_test(&frame, &pack, &|_, lng, _| {
            Some((100.0 + (lng - lon) * MEAN_RADIUS * lat.cos() * 0.2) as f32)
        });
        let bottom = result
            .mesh
            .vertices
            .iter()
            .map(|v| world_height(&frame, v.pos))
            .min_by(f64::total_cmp)
            .unwrap();
        assert!(
            (bottom - 95.6).abs() < 0.02,
            "unsupported foundation {bottom}"
        );
        let datum = datum_cover(&frame, &pack).footprints();
        assert!(!datum.is_empty());
        let pitch = crate::datum_pitch_m(frame.column_latlon(0.5, 0.5).0);
        assert!(datum.iter().all(|cap| cap.sample_hint_m == pitch));
        assert_eq!(
            roof_source_footprint(&frame, &pack).unwrap().sample_hint_m,
            1.0
        );
        assert!(roof_source_footprint(&super::tests::frame(15), &pack).is_none());
    }

    #[test]
    fn neighboring_cell_reprojections_share_foundation_eave_and_storey_datums() {
        let a = frame(18);
        let mut address = a.addr;
        address.cell.i += 1;
        let b = ChunkFrame::of(address);
        let (alat, alon) = a.column_latlon(0.5, 0.5);
        let (blat, blon) = b.column_latlon(0.5, 0.5);
        let (latitude, longitude) = a.column_latlon(1.0, 0.5);
        let center = DVec2::new(
            (longitude - alon) * MEAN_RADIUS * alat.cos(),
            (latitude - alat) * MEAN_RADIUS,
        );
        let rings = vec![
            square(-14.0, 14.0)
                .into_iter()
                .map(|p| {
                    (center + DVec2::from_array(p.map(f64::from)))
                        .as_vec2()
                        .to_array()
                })
                .collect(),
        ];
        let pa = pack_with(rings, 32.0);
        let mut pb = pack_with(pa.buildings[0].rings.clone(), 32.0);
        pb.buildings[0].rings = pa.buildings[0]
            .rings
            .iter()
            .map(|ring| {
                ring.iter()
                    .map(|p| {
                        let (lat, lon) = sample_latlon(&a, *p);
                        [
                            ((lon - blon) * MEAN_RADIUS * blat.cos()) as f32,
                            ((lat - blat) * MEAN_RADIUS) as f32,
                        ]
                    })
                    .collect()
            })
            .collect();
        let elevation = |lat, lon, _| {
            Some(
                (100.0
                    + (lon - longitude) * MEAN_RADIUS * latitude.cos() * 0.1
                    + (lat - latitude) * MEAN_RADIUS * 0.05) as f32,
            )
        };
        let ma = generate_test(&a, &pa, &elevation);
        let mb = generate_test(&b, &pb, &elevation);
        let entrance = |pack: &FeaturePack| {
            let rings = &pack.buildings[0].rings;
            let context = FacadeContext {
                party_walls: Vec::new(),
                opaque_core: None,
                shadow_walls: std::cell::RefCell::default(),
                owner: rings.clone(),
                owner_bounds: bounds(rings),
                owner_query: PolygonQuery::new(rings),
                blockers: Vec::new(),
                entrance: None,
            };
            let boundary = Footprint::new(vec![rings], Vec::new()).unwrap().boundary();
            entrance_on_source_boundary(rings, &boundary, 100.0, &context, None).unwrap()
        };
        let (edge_a, along_a) = entrance(&pa);
        let (edge_b, along_b) = entrance(&pb);
        assert_eq!(
            edge_a, edge_b,
            "neighboring cells selected different entrance edges"
        );
        assert!((along_a - along_b).abs() < 0.01);
        let heights = |fr: &ChunkFrame, mesh: &BuildingSurfaceData| {
            let bottom = mesh
                .mesh
                .vertices
                .iter()
                .map(|v| world_height(fr, v.pos))
                .min_by(f64::total_cmp)
                .unwrap();
            let roof = mesh
                .mesh
                .vertices
                .iter()
                .filter(|v| v.face_ao & 7 == 2)
                .map(|v| world_height(fr, v.pos))
                .max_by(f64::total_cmp)
                .unwrap();
            let deck = mesh
                .mesh
                .vertices
                .iter()
                .filter(|v| is_roof_vertex(v))
                .map(|v| world_height(fr, v.pos))
                .min_by(f64::total_cmp)
                .unwrap();
            [bottom, roof, deck]
        };
        for (a, b) in heights(&a, &ma).into_iter().zip(heights(&b, &mb)) {
            assert!((a - b).abs() < 0.01, "neighbor datum {a} versus {b}");
        }
        let storey_heights = |frame: &ChunkFrame, data: &BuildingSurfaceData| {
            let mut heights: Vec<_> = glazing(&data.mesh)
                .iter()
                // Clip intersections can lie anywhere along an arched pane;
                // compare the surviving source sill/crown knots instead.
                .filter(|vertex| {
                    [vertex.pos[0], vertex.pos[2]]
                        .into_iter()
                        .all(|coordinate| coordinate > 0.001 && coordinate < N as f32 - 0.001)
                })
                .map(|vertex| world_height(frame, vertex.pos))
                .collect();
            heights.sort_by(f64::total_cmp);
            heights.dedup_by(|a, b| (*a - *b).abs() < 0.001);
            heights
        };
        let storeys_a = storey_heights(&a, &ma);
        let storeys_b = storey_heights(&b, &mb);
        assert!(
            storeys_a.len() >= 8,
            "fixture must span several source storeys"
        );
        assert_eq!(storeys_a.len(), storeys_b.len());
        for (a, b) in storeys_a.into_iter().zip(storeys_b) {
            assert!((a - b).abs() < 0.01, "glazing/storey datum {a} versus {b}");
        }
        for (fr, pack, mesh) in [(&a, &pa, &ma), (&b, &pb, &mb)] {
            let deck = heights(fr, mesh)[2];
            for vertex in &mesh.mesh.vertices {
                if (vertex.pos[0].abs() < 1.0e-4 || (vertex.pos[0] - N as f32).abs() < 1.0e-4)
                    && world_height(fr, vertex.pos) > deck + 0.03
                {
                    let point = source_position(fr, vertex.pos).truncate();
                    assert!(
                        edge_facade(&pack.buildings[0].rings, point).edge_distance_m < 0.3,
                        "chunk clip created an internal parapet"
                    );
                }
            }
        }
    }
    #[test]
    fn roof_beneath_a_floating_mapped_part_remains_closed_and_keeps_its_profile() {
        for shape in [
            RoofShape::Gabled,
            RoofShape::Mansard,
            RoofShape::Gambrel,
            RoofShape::Skillion,
        ] {
            let frame = frame(17);
            let mut pack = pack_with(vec![square(-12.0, 12.0)], 12.0);
            pack.buildings[0].roof_shape = Some(shape);
            pack.buildings.push(Building {
                location_rad: None,
                rings: vec![square(-5.0, 5.0)],
                height_m: 28.0,
                min_height_m: 20.0,
                seed: 77,
                kind: BuildingKind::Part,
                roof_shape: None,
                use_kind: Default::default(),
            });
            let ov = tier0::build_overlay(
                frame.addr.cell,
                &frame,
                &pack,
                &|_, _| 100.0,
                0,
                None,
                None,
                None,
            );
            let stamp = ov
                .cols
                .iter()
                .flatten()
                .filter_map(|c| c.building)
                .find(|s| s.seed == 42)
                .unwrap();
            let (shape, rise) = (stamp.roof_shape, stamp.roof_height_m);
            let roof = RoofField::new(
                &pack.buildings[0].rings,
                shape,
                112.0 - f64::from(rise.unwrap_or(0.0)),
                112.0,
                architecture_facade::production_hip_pitch(&stamp.design),
            );
            let mesh = generate_test(&frame, &pack, &|_, _, _| Some(100.0));
            let (lat, lon) = frame.column_latlon(0.5, 0.5);
            let mut cap_triangles = 0;
            for triangle in mesh.mesh.indices.chunks_exact(3) {
                if mesh.mesh.vertices[triangle[0] as usize].face_ao & 7 != 2 {
                    continue;
                }
                let center = triangle
                    .iter()
                    .map(|&i| DVec3::from_array(mesh.mesh.vertices[i as usize].pos.map(f64::from)))
                    .sum::<DVec3>()
                    / 3.0;
                let (p_lat, p_lon, height) =
                    ellipsoid::ecef_to_geodetic(frame.local_to_ecef(center));
                let en = DVec2::new(
                    (p_lon - lon) * MEAN_RADIUS * lat.cos(),
                    (p_lat - lat) * MEAN_RADIUS,
                );
                if en.x.abs() < 3.0 && en.y.abs() < 3.0 && height < 115.0 {
                    assert!((height - roof.height(en)).abs() < 0.03);
                    cap_triangles += 1;
                }
            }
            assert!(
                cap_triangles > 0,
                "mapped parent roof disappeared beneath an elevated part"
            );
        }
    }

    #[test]
    fn lit_glazing_survives_the_projected_ornament_detail_cutoff() {
        let frame = frame(18);
        let pack = pack_with(vec![square(-12.0, 12.0)], 36.0);
        let result = generate_shaded_facade_test(&frame, &pack);
        assert!(
            result
                .mesh
                .vertices
                .iter()
                .any(|v| ((v.face_ao >> 5) & 255) as u8 == crate::material::LIT_WINDOW)
        );
        assert!(!result.details.is_empty());
        assert!(
            result
                .details
                .iter()
                .flat_map(|d| &d.mesh.vertices)
                .all(|v| !matches!(
                    ((v.face_ao >> 5) & 255) as u8,
                    crate::material::WINDOW | crate::material::LIT_WINDOW
                ))
        );
    }

    #[test]
    fn coarse_roof_cap_gate_preserves_near_authored_measured_and_signature_shapes() {
        for level in 11..=19 {
            let frame = frame(level);
            let (lat, lon) = frame.column_latlon(0.5, 0.5);
            let projection = Projection {
                frame: &frame,
                lat,
                lon,
            };
            let mut pack = pack_with(vec![square(-2.0, 2.0)], 12.0);
            let mut stamp = facade_fixture(&frame, &pack);
            let policy = RoofResolution::for_frame(&frame);
            let allowance = frame.addr.cell.sample_pitch_m() / 12.0;
            let mut roof = RoofField::new(
                &pack.buildings[0].rings,
                Some(RoofShape::Gabled),
                0.0,
                allowance,
                0.7,
            );
            for shape in [
                RoofShape::Gabled,
                RoofShape::Hipped,
                RoofShape::Pyramidal,
                RoofShape::Mansard,
                RoofShape::Gambrel,
                RoofShape::Skillion,
            ] {
                roof.shape = Some(shape);
                assert_eq!(
                    policy
                        .contained_cap_top(&projection, &pack.buildings[0], &stamp, &roof)
                        .is_some(),
                    level < 17
                );
            }
            roof.top = allowance.next_up();
            assert!(
                policy
                    .contained_cap_top(&projection, &pack.buildings[0], &stamp, &roof)
                    .is_none()
            );
            roof.top = allowance * 0.5;
            for shape in [
                None,
                Some(RoofShape::Flat),
                Some(RoofShape::Dome),
                Some(RoofShape::Onion),
                Some(RoofShape::Sawtooth),
            ] {
                roof.shape = shape;
                assert!(
                    policy
                        .contained_cap_top(&projection, &pack.buildings[0], &stamp, &roof)
                        .is_none()
                );
            }
            roof.shape = Some(RoofShape::Gabled);
            roof.tiered = true;
            assert!(
                policy
                    .contained_cap_top(&projection, &pack.buildings[0], &stamp, &roof)
                    .is_none()
            );
            roof.tiered = false;
            stamp.lidar = true;
            assert!(
                policy
                    .contained_cap_top(&projection, &pack.buildings[0], &stamp, &roof)
                    .is_none()
            );
            stamp.lidar = false;
            pack.buildings[0].roof_shape = Some(RoofShape::Gabled);
            assert!(
                policy
                    .contained_cap_top(&projection, &pack.buildings[0], &stamp, &roof)
                    .is_none()
            );
            pack.buildings[0].roof_shape = None;
            pack.buildings[0].kind = BuildingKind::RoofOnly;
            assert!(
                policy
                    .contained_cap_top(&projection, &pack.buildings[0], &stamp, &roof)
                    .is_none()
            );
            pack.buildings[0].kind = BuildingKind::Building;
            for top in [0.0, -1.0, f64::NAN, f64::INFINITY] {
                roof.top = top;
                assert!(
                    policy
                        .contained_cap_top(&projection, &pack.buildings[0], &stamp, &roof)
                        .is_none()
                );
            }
        }
    }

    #[test]
    fn coarse_roof_cap_sphere_encloses_curved_skewed_and_dateline_geometry() {
        use vearth_core::{
            cell::{CellId, ChunkAddr},
            cubesphere::Face,
        };
        let mut admitted = 0;
        for face in 0..6 {
            for level in [11, 13, 16] {
                let n = 1u32 << level;
                for (i, j) in [(1, 1), (n / 2, n / 2), (n - 2, n - 2)] {
                    let frame = ChunkFrame::of(ChunkAddr {
                        cell: CellId {
                            face: Face(face),
                            level,
                            i,
                            j,
                        },
                        k: -2,
                    });
                    let (lat, lon) = frame.column_latlon(0.5, 0.5);
                    let projection = Projection {
                        frame: &frame,
                        lat,
                        lon,
                    };
                    for heights in [[-500.0, -498.0], [100.0, 102.0], [8998.0, 9000.0]] {
                        let rectangle = [DVec2::new(-22.0, -17.0), DVec2::new(27.0, 23.0)];
                        let Some((center, radius)) =
                            roof_envelope_sphere(&projection, rectangle, heights)
                        else {
                            continue;
                        };
                        let inside = roof_sphere_inside_owner(&frame, center, radius);
                        admitted += usize::from(inside);
                        for x in 0..=8 {
                            for y in 0..=8 {
                                let p = DVec2::new(
                                    rectangle[0].x
                                        + (rectangle[1].x - rectangle[0].x) * x as f64 / 8.0,
                                    rectangle[0].y
                                        + (rectangle[1].y - rectangle[0].y) * y as f64 / 8.0,
                                );
                                for h in [heights[0], (heights[0] + heights[1]) * 0.5, heights[1]] {
                                    let world = projection.world_point(p, h);
                                    let local = frame.ecef_to_local(world).as_vec3().as_dvec3();
                                    let rounded = frame.local_to_ecef(local);
                                    assert!(
                                        world.distance(center) <= radius
                                            && rounded.distance(center) <= radius
                                    );
                                    if inside {
                                        assert!(
                                            local.x > 0.0
                                                && local.z > 0.0
                                                && local.x < N as f64
                                                && local.z < N as f64
                                        );
                                        let (actual_face, u, v) =
                                            vearth_core::cubesphere::dir_to_face_uv(rounded);
                                        let (u0, v0, u1, v1) = frame.addr.cell.uv_bounds();
                                        assert_eq!(actual_face, frame.addr.cell.face);
                                        assert!(u > u0 && u < u1 && v > v0 && v < v1);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        assert!(admitted > 100);
        // The EN interval straddles the wrapped longitude discontinuity.
        let frame = frame(13);
        let projection = Projection {
            frame: &frame,
            lat: 0.3,
            lon: std::f64::consts::PI - 1.0e-7,
        };
        let rectangle = [DVec2::splat(-30.0), DVec2::splat(30.0)];
        let (center, radius) =
            roof_envelope_sphere(&projection, rectangle, [100.0, 102.0]).unwrap();
        for p in [rectangle[0], rectangle[1], DVec2::ZERO] {
            assert!(projection.world_point(p, 102.0).distance(center) <= radius);
        }
        assert!(roof_envelope_sphere(&projection, rectangle, [f64::NAN, 1.0]).is_none());
        let polar = Projection {
            lat: std::f64::consts::FRAC_PI_2,
            ..projection
        };
        assert!(roof_envelope_sphere(&polar, rectangle, [0.0, 1.0]).is_none());
    }

    #[test]
    fn coarse_roof_cap_requires_both_logical_and_render_owner_containment() {
        use vearth_core::{
            cell::{CellId, ChunkAddr},
            cubesphere::Face,
        };
        let mut prism_only = 0;
        for face in 0..6 {
            let frame = ChunkFrame::of(ChunkAddr {
                cell: CellId {
                    face: Face(face),
                    level: 11,
                    i: 187,
                    j: 1761,
                },
                k: 0,
            });
            for x in [0.0001, N as f64 - 0.0001] {
                for z in [0.0001, 16.0, N as f64 - 0.0001] {
                    for y in [-10.0, 0.0, 100.0] {
                        let center = frame.local_to_ecef(DVec3::new(x, y, z));
                        let (f, u, v) = vearth_core::cubesphere::dir_to_face_uv(center);
                        let (u0, v0, u1, v1) = frame.addr.cell.uv_bounds();
                        if f != frame.addr.cell.face || u <= u0 || u >= u1 || v <= v0 || v >= v1 {
                            prism_only += 1;
                            assert!(!roof_sphere_inside_owner(&frame, center, 1.0e-6));
                        }
                    }
                }
            }
            let center = frame.local_to_ecef(DVec3::new(16.0, 0.0, 16.0));
            assert!(roof_sphere_inside_owner(&frame, center, 1.0));
            assert!(!roof_sphere_inside_owner(
                &frame,
                center,
                frame.grid_scale.x * N as f64
            ));
            assert!(!roof_sphere_inside_owner(
                &frame,
                DVec3::splat(f64::NAN),
                1.0
            ));
        }
        assert!(
            prism_only > 0,
            "fixture must distinguish the two owner definitions"
        );
    }

    #[test]
    fn coarse_roof_cap_keeps_holes_exclusions_top_and_shared_closures() {
        let frame = frame(13);
        let (lat, lon) = frame.column_latlon(0.5, 0.5);
        let projection = Projection {
            frame: &frame,
            lat,
            lon,
        };
        let rings = vec![
            vec![
                [-120.0, -90.0],
                [120.0, -90.0],
                [120.0, 20.0],
                [30.0, 20.0],
                [30.0, 90.0],
                [-120.0, 90.0],
            ],
            square(-15.0, 15.0),
        ];
        let excluded = vec![vec![
            [-120.0, 30.0],
            [-80.0, 30.0],
            [-80.0, 70.0],
            [-120.0, 70.0],
        ]];
        let domain = Footprint::new(vec![&rings], vec![&excluded]).unwrap();
        let boundary = domain.boundary();
        let roof = RoofField::new(&rings, Some(RoofShape::Hipped), 100.0, 102.0, 0.7);
        let mut stats = crate::contracts::RoofWorkCounters::default();
        let cap = coarse_roof_cap(&domain, &boundary, 102.0, Some(&mut stats)).unwrap();
        assert!(cap.triangles.len() < 100 && stats.plane_attempts == 0);
        assert_eq!(
            stats.fallback_budget + stats.fallback_depth + stats.fallback_invalid,
            0
        );
        let reference_area: f64 = domain
            .roof_cells(4096.0)
            .unwrap()
            .iter()
            .map(|q| {
                ((q[1] - q[0]).perp_dot(q[2] - q[0]).abs()
                    + (q[2] - q[0]).perp_dot(q[3] - q[0]).abs())
                    * 0.5
            })
            .sum();
        assert!((roof_planar_area(&cap) - reference_area).abs() < 1.0e-8);
        let mut roof_mesh = MeshBuffers::default();
        let mut closures = MeshBuffers::default();
        let appearance = SurfaceAppearance::default();
        emit_roof(&projection, &mut roof_mesh, &cap, appearance);
        for edge in &cap.boundary {
            assert_eq!(edge.heights, [102.0; 2]);
            emit_roof_closure(&projection, &mut closures, edge, 100.0, appearance);
            for p in edge.points {
                let expected = projection.point(p, 102.0).as_vec3().to_array();
                assert!(roof_mesh.vertices.iter().any(|v| v.pos == expected));
                assert!(closures.vertices.iter().any(|v| v.pos == expected));
            }
        }
        assert!(!closures.indices.is_empty());
        let cap_length: f64 = cap
            .boundary
            .iter()
            .map(|e| e.points[0].distance(e.points[1]))
            .sum();
        let source_length: f64 = boundary.iter().map(|e| e[0].distance(e[1])).sum();
        assert!((cap_length - source_length).abs() < 1.0e-8);
        let error = roof_policy_error(&cap, &roof, 8);
        assert!(error.0 <= 2.0 + 1.0e-10 && error.1 <= 2.0 + 1.0e-10);
        for tri in &cap.triangles {
            assert_eq!(tri.heights, [102.0; 3]);
            assert!(domain.contains(tri.points.iter().copied().sum::<DVec2>() / 3.0));
        }
        for x in (-119..120).step_by(7) {
            for y in (-89..90).step_by(7) {
                let p = DVec2::new(x as f64 + 0.123, y as f64 + 0.321);
                let covered = cap.triangles.iter().any(|t| {
                    let a = t.points[1] - t.points[0];
                    let b = t.points[2] - t.points[0];
                    let d = p - t.points[0];
                    let det = a.perp_dot(b);
                    if det.abs() < 1.0e-12 {
                        return false;
                    }
                    let u = d.perp_dot(b) / det;
                    let v = a.perp_dot(d) / det;
                    u >= 0.0 && v >= 0.0 && u + v <= 1.0
                });
                assert_eq!(covered, domain.contains(p), "coverage at {p:?}");
            }
        }
    }

    #[test]
    fn coarse_roof_cap_preserves_near_fallbacks_glazing_collision_and_parent_body() {
        for level in [13, 16, 17, 19] {
            for mode in 0..4 {
                let frame = frame(level);
                let mut pack = pack_with(vec![square(-24.0, 24.0), square(-6.0, 6.0)], 12.0);
                if mode == 1 {
                    pack.buildings[0].roof_shape = Some(RoofShape::Gabled);
                }
                if mode == 3 {
                    for p in pack.buildings[0].rings.iter_mut().flatten() {
                        p[0] += (frame.grid_scale.x * N as f64 * 0.49) as f32;
                    }
                } else {
                    pack.buildings.push(Building {
                        rings: vec![vec![[8.0, -10.0], [18.0, -10.0], [18.0, 10.0], [8.0, 10.0]]],
                        location_rad: None,
                        height_m: 24.0,
                        min_height_m: 18.0,
                        seed: 43,
                        kind: BuildingKind::Part,
                        roof_shape: Some(RoofShape::Flat),
                        use_kind: Default::default(),
                    });
                }
                let mut overlay = tier0::build_overlay(
                    frame.addr.cell,
                    &frame,
                    &pack,
                    &|_, _| 100.0,
                    0,
                    None,
                    None,
                    Some(&|_| true),
                );
                let source = overlay
                    .source_buildings
                    .iter_mut()
                    .find(|s| s.source_index == Some(0))
                    .unwrap();
                source.roof_shape = Some(RoofShape::Gabled);
                source.roof_height_m =
                    Some((frame.addr.cell.sample_pitch_m() / 24.0).min(1.5) as f32);
                source.design.expression = 0;
                if mode == 2 {
                    source.lidar = true;
                    source.top_geo = 112.0;
                }
                let ground = |_, _, _| Some(100.0);
                let surface = |_, _, _| Some(112.0);
                let mut plan =
                    prepare_envelopes(&frame, &pack, &overlay, &[u32::MAX; N], &ground, &surface)
                        .unwrap();
                let actual = generate_prepared_envelopes::<true>(
                    &frame,
                    &pack,
                    &overlay,
                    &[u32::MAX; N],
                    &ground,
                    &surface,
                    false,
                    usize::MAX,
                    &plan,
                )
                .unwrap();
                plan.roof_resolution.envelope_rise_m = 0.0;
                let old = generate_prepared_envelopes::<true>(
                    &frame,
                    &pack,
                    &overlay,
                    &[u32::MAX; N],
                    &ground,
                    &surface,
                    false,
                    usize::MAX,
                    &plan,
                )
                .unwrap();
                assert!(actual.complete && old.complete && !actual.mesh.indices.is_empty());
                assert_eq!(actual.collision, old.collision);
                assert_eq!(
                    glazing_triangles(&actual.mesh),
                    glazing_triangles(&old.mesh)
                );
                if level <= 15 || level >= 17 || mode != 0 {
                    assert_same_building_output(&actual, &old);
                } else {
                    assert!(actual.mesh.indices.len() < old.mesh.indices.len());
                    let mut under_part = 0;
                    for tri in actual.mesh.indices.chunks_exact(3) {
                        if !is_roof_vertex(&actual.mesh.vertices[tri[0] as usize]) {
                            continue;
                        }
                        let center = tri
                            .iter()
                            .map(|&i| {
                                DVec3::from_array(
                                    actual.mesh.vertices[i as usize].pos.map(f64::from),
                                )
                            })
                            .sum::<DVec3>()
                            / 3.0;
                        let p = source_position(&frame, center.as_vec3().to_array());
                        if p.x > 8.1 && p.x < 17.9 && p.y > -9.9 && p.y < 9.9 && p.z < 115.0 {
                            assert!((p.z - 112.0).abs() < 0.02);
                            under_part += 1;
                        }
                    }
                    assert!(under_part > 0, "exposed parent cap missing");
                }
            }
        }
    }

    #[test]
    fn distant_caps_keep_exact_footprint_coverage_with_two_triangles_per_slab() {
        let rings = vec![square(-120.0, 120.0), square(-20.0, 20.0)];
        let excluded = vec![vec![
            [60.0, -120.0],
            [100.0, -120.0],
            [100.0, -60.0],
            [60.0, -60.0],
        ]];
        let domain = Footprint::new(vec![&rings], vec![&excluded]).unwrap();
        let boundary = domain.boundary();
        let mut stats = crate::contracts::RoofWorkCounters::default();
        let cap = distant_roof_cap(&domain, &boundary, 112.0, Some(&mut stats)).unwrap();
        assert_eq!(
            roof_planar_area(&cap),
            240.0 * 240.0 - 40.0 * 40.0 - 40.0 * 60.0
        );
        assert!(cap.triangles.len() <= 20);
        assert!(cap.triangles.len() <= stats.initial_cells as usize * 2);
        assert_eq!(stats.plane_attempts, 0);
        assert!(cap.triangles.iter().all(|triangle| {
            triangle.heights == [112.0; 3]
                && domain.contains(triangle.points.iter().copied().sum::<DVec2>() / 3.0)
        }));
        assert_eq!(
            cap.boundary
                .iter()
                .map(|edge| edge.points)
                .collect::<Vec<_>>(),
            boundary
        );
        let rectangle = vec![square(-120.0, 120.0)];
        let domain = Footprint::new(vec![&rectangle], vec![]).unwrap();
        assert_eq!(
            distant_roof_cap(&domain, &domain.boundary(), 112.0, None)
                .unwrap()
                .triangles
                .len(),
            2
        );
    }

    #[test]
    fn distant_buildings_keep_crossing_sources_holes_heights_and_collision() {
        for level in [11, 12, 13, 14, 15] {
            let frame = frame(level);
            for crossing in [false, true] {
                for shape in [RoofShape::Gabled, RoofShape::Flat, RoofShape::Dome] {
                    let mut pack = pack_with(vec![square(-40.0, 40.0), square(-10.0, 10.0)], 12.0);
                    pack.buildings[0].roof_shape = Some(shape);
                    let shift = if crossing {
                        (frame.grid_scale.x * N as f64 * 0.5) as f32
                    } else {
                        0.0
                    };
                    for point in pack.buildings[0].rings.iter_mut().flatten() {
                        point[0] += shift;
                    }
                    let mut overlay = tier0::build_overlay(
                        frame.addr.cell,
                        &frame,
                        &pack,
                        &|_, _| 100.0,
                        0,
                        None,
                        None,
                        Some(&|_| true),
                    );
                    for stamp in &mut overlay.source_buildings {
                        stamp.roof_shape = Some(shape);
                        stamp.roof_height_m = Some(2.0);
                    }
                    let result = generate(
                        &frame,
                        &pack,
                        &overlay,
                        &[u32::MAX; N],
                        &|_, _, _| Some(100.0),
                        &|_, _, _| None,
                    )
                    .unwrap();
                    assert!(result.complete && !result.mesh.indices.is_empty());
                    assert!(result.details.is_empty() && glazing(&result.mesh).is_empty());
                    assert!(
                        result.mesh.indices.len() / 3 < 120,
                        "L{level} {shape:?} crossing={crossing}"
                    );
                    assert!(!result.collision.is_empty());
                    crate::collision::SurfaceCollision::from_triangles(result.collision)
                        .validate()
                        .unwrap();
                    let mut roofs = 0;
                    for indices in result.mesh.indices.chunks_exact(3) {
                        if !is_roof_vertex(&result.mesh.vertices[indices[0] as usize]) {
                            continue;
                        }
                        let points = indices
                            .iter()
                            .map(|&index| {
                                source_position(&frame, result.mesh.vertices[index as usize].pos)
                            })
                            .collect::<Vec<_>>();
                        for point in &points {
                            assert!((point.z - 112.0).abs() < 0.05, "{point:?}");
                        }
                        let center = points.iter().copied().sum::<DVec3>() / 3.0;
                        assert!(
                            (center.x - f64::from(shift)).abs() >= 9.9 || center.y.abs() >= 9.9
                        );
                        roofs += 1;
                    }
                    assert!(roofs > 0);
                }
            }
        }
        for level in 16..=19 {
            assert!(coarse_building_pixel_m(&frame(level)).is_none());
        }
    }

    #[test]
    fn distant_measured_roofs_retain_resolved_interior_rises() {
        let frame = frame(15);
        let mut pack = pack_with(vec![square(-12.0, 12.0)], 12.0);
        pack.buildings[0].roof_shape = Some(RoofShape::Flat);
        let mut overlay = tier0::build_overlay(
            frame.addr.cell,
            &frame,
            &pack,
            &|_, _| 100.0,
            0,
            None,
            None,
            Some(&|_| true),
        );
        for stamp in &mut overlay.source_buildings {
            stamp.lidar = true;
            stamp.top_geo = 112.0;
        }
        let (lat, lon) = frame.column_latlon(0.5, 0.5);
        let surface = |sample_lat: f64, sample_lon: f64, _| {
            let en = DVec2::new(
                (sample_lon - lon) * MEAN_RADIUS * lat.cos(),
                (sample_lat - lat) * MEAN_RADIUS,
            );
            Some(if en.abs().max_element() < 5.0 {
                122.0
            } else {
                112.0
            })
        };
        let result = generate(
            &frame,
            &pack,
            &overlay,
            &[u32::MAX; N],
            &|_, _, _| Some(100.0),
            &surface,
        )
        .unwrap();
        assert!(result.complete);
        let highest = result
            .mesh
            .vertices
            .iter()
            .map(|vertex| source_position(&frame, vertex.pos).z)
            .fold(f64::NEG_INFINITY, f64::max);
        assert!(highest > 121.9, "measured ridge flattened to {highest}");
    }

    #[test]
    fn distant_roof_owner_seals_face_outward_and_only_visit_new_roof_triangles() {
        let frame = frame(13);
        let mut mesh = MeshBuffers::default();
        let appearance = SurfaceAppearance::default();
        emit_quad(
            &mut mesh,
            [
                DVec3::new(0.0, 2.0, 0.0),
                DVec3::new(0.0, 3.0, 0.0),
                DVec3::new(0.0, 3.0, 2.0),
                DVec3::new(0.0, 2.0, 2.0),
            ],
            appearance,
            false,
        );
        let start = mesh.indices.len();
        emit_quad(
            &mut mesh,
            [
                DVec3::new(-2.0, 10.0, -2.0),
                DVec3::new(34.0, 10.0, -2.0),
                DVec3::new(34.0, 10.0, 34.0),
                DVec3::new(-2.0, 10.0, 34.0),
            ],
            appearance,
            true,
        );
        let first_seam = mesh.indices.len();
        let eave =
            ellipsoid::ecef_to_geodetic(frame.local_to_ecef(DVec3::new(16.0, 10.0, 16.0))).2 - 2.0;
        seal_coarse_roof_cuts(&frame, &mut mesh, start, eave, appearance);
        assert!(mesh.indices.len() > first_seam);
        for indices in mesh.indices[first_seam..].chunks_exact(3) {
            let points = [indices[0], indices[1], indices[2]]
                .map(|index| DVec3::from_array(mesh.vertices[index as usize].pos.map(f64::from)));
            let center = (points[0] + points[1] + points[2]) / 3.0;
            let normal = (points[1] - points[0])
                .cross(points[2] - points[0])
                .normalize();
            assert!(
                normal.dot(DVec3::new(center.x - 16.0, 0.0, center.z - 16.0)) > 15.9,
                "center={center:?} normal={normal:?} points={points:?}"
            );
            assert!(points.iter().all(|point| point.y > 9.0));
        }
    }

    #[test]
    fn distant_roof_seals_stop_at_eave_for_raised_sloping_parts() {
        let frame = frame(15);
        let (lat, lon) = frame.column_latlon(0.5, 0.5);
        let projection = Projection {
            frame: &frame,
            lat,
            lon,
        };
        let shift = frame.grid_scale.x * N as f64 * 0.5;
        let mut mesh = MeshBuffers::default();
        let appearance = SurfaceAppearance::default();
        emit_quad(
            &mut mesh,
            [
                projection.point(DVec2::new(shift - 40.0, -20.0), 111.0),
                projection.point(DVec2::new(shift + 40.0, -20.0), 111.0),
                projection.point(DVec2::new(shift + 40.0, 20.0), 123.0),
                projection.point(DVec2::new(shift - 40.0, 20.0), 123.0),
            ],
            appearance,
            true,
        );
        let first_seam = mesh.indices.len();
        seal_coarse_roof_cuts(&frame, &mut mesh, 0, 111.0, appearance);
        assert!(mesh.indices.len() > first_seam);
        for &index in &mesh.indices[first_seam..] {
            let point = source_position(&frame, mesh.vertices[index as usize].pos);
            assert!(
                point.z > 110.90 && point.z <= 123.05,
                "raised-part clearance blocked at {point:?}"
            );
        }
    }

    #[test]
    #[ignore = "paired complete roof construction benchmark; requires VEARTH_ROOF_CONCAVITY_FIXTURE"]
    fn coarse_roof_cap_actual_source_benchmark() {
        use vearth_core::cell::ChunkAddr;
        let mut totals = [0u128; 2];
        for case in roof_source_cases() {
            let frame = ChunkFrame::of(ChunkAddr {
                cell: case.cell,
                k: 0,
            });
            let (lat, lon) = frame.column_latlon(0.5, 0.5);
            let projection = Projection {
                frame: &frame,
                lat,
                lon,
            };
            let pack = pack_with(case.rings.clone(), (10.0 + case.rise) as f32);
            let stamp = facade_fixture(&frame, &pack);
            let construct = |enabled: bool| {
                let source = &pack.buildings[0];
                let mut roof = RoofField::new(
                    &source.rings,
                    Some(case.shape),
                    100.0,
                    100.0 + case.rise,
                    case.pitch,
                );
                roof.tiered = case.tiered;
                if let Some(expected) = case.expected_extent {
                    assert!(
                        (roof.bounds[1] - roof.bounds[0] - DVec2::from_array(expected))
                            .abs()
                            .max_element()
                            < 1.0e-8
                    );
                }
                let mut policy = RoofResolution::for_frame(&frame);
                if !enabled {
                    policy.envelope_rise_m = 0.0;
                }
                let cap = policy.contained_cap_top(&projection, source, &stamp, &roof);
                let domain = Footprint::new(vec![&source.rings], vec![]).unwrap();
                let boundary = domain.boundary();
                let mut stats = crate::contracts::RoofWorkCounters::default();
                let geometry = if let Some(top) = cap {
                    coarse_roof_cap(&domain, &boundary, top, Some(&mut stats)).unwrap()
                } else {
                    let rotated: Rings = source
                        .rings
                        .iter()
                        .map(|ring| {
                            ring.iter()
                                .map(|p| {
                                    roof.rotate(DVec2::from_array(p.map(f64::from)))
                                        .as_vec2()
                                        .to_array()
                                })
                                .collect()
                        })
                        .collect();
                    let rotated = Footprint::new(vec![&rotated], vec![]).unwrap();
                    stats.analytic_attempts = u64::from(case.rise > 0.0);
                    let analytic = (case.rise > 0.0)
                        .then(|| analytic_roof(&rotated, &boundary, &roof, true, 3.0, &projection))
                        .flatten();
                    stats.analytic_accepted = u64::from(analytic.is_some());
                    analytic.unwrap_or_else(|| {
                        let step = if case.rise > 0.0 { 3.0 } else { 4096.0 };
                        let cells = rotated
                            .roof_cells(step)
                            .or_else(|| rotated.roof_cells(4096.0))
                            .unwrap();
                        roof_geometry_profiled(
                            cells.into_iter().map(|q| q.map(|p| roof.unrotate(p))),
                            &boundary,
                            &|p| roof.height(p),
                            case.rise > 0.0,
                            policy.sampling_tolerance(case.rise, Some(case.shape)),
                            roof.height_bounds(),
                            Some(&mut stats),
                        )
                        .unwrap()
                    })
                };
                let mut mesh = MeshBuffers::default();
                let appearance = SurfaceAppearance {
                    material: crate::material::ROOF,
                    tint: [100; 3],
                };
                emit_roof(&projection, &mut mesh, &geometry, appearance);
                for edge in &geometry.boundary {
                    emit_roof_closure(&projection, &mut mesh, edge, roof.eave, appearance);
                }
                (geometry, mesh, stats, cap.is_some())
            };
            let old = construct(false);
            let new = construct(true);
            if new.3 {
                assert!(new.0.triangles.len() < old.0.triangles.len());
                let area = roof_planar_area(&old.0);
                assert!((roof_planar_area(&new.0) - area).abs() <= area * 1.0e-5 + 1.0e-6);
                let mut roof = RoofField::new(
                    &case.rings,
                    Some(case.shape),
                    100.0,
                    100.0 + case.rise,
                    case.pitch,
                );
                roof.tiered = case.tiered;
                let error = roof_policy_error(&new.0, &roof, 8);
                assert!(error.0 <= case.rise + 1.0e-8 && error.1 <= case.rise + 1.0e-8);
                assert_eq!(new.2.plane_attempts, 0);
            } else {
                assert_same_roof_geometry(&new.0, &old.0);
                assert_eq!(new.1.vertices, old.1.vertices);
                assert_eq!(new.1.indices, old.1.indices);
            }
            let mut times = [Vec::new(), Vec::new()];
            for run in 0..7 {
                for mode in [run % 2, 1 - run % 2] {
                    let start = vearth_platform::time::Instant::now();
                    let output = construct(mode == 1);
                    std::hint::black_box(&output);
                    times[mode].push(start.elapsed().as_nanos());
                }
            }
            for mode in 0..2 {
                times[mode].sort_unstable();
                totals[mode] += times[mode][3];
            }
            eprintln!(
                "{}: original EN/profile; controlled eave100/inferred-unmeasured/no neighbors; admitted={}; source_triangles={}->{}; emitted_triangles={}->{}; emitted_bytes={}->{}; initial_cells={}->{}; planes={}->{}; fallback_budget={}->{}; construct_project_closure_ms={:.3}->{:.3}",
                case.name,
                new.3,
                old.0.triangles.len(),
                new.0.triangles.len(),
                old.1.indices.len() / 3,
                new.1.indices.len() / 3,
                crate::built_surface::mesh_bytes(&old.1),
                crate::built_surface::mesh_bytes(&new.1),
                old.2.initial_cells,
                new.2.initial_cells,
                old.2.plane_attempts,
                new.2.plane_attempts,
                old.2.fallback_budget,
                new.2.fallback_budget,
                times[0][3] as f64 / 1.0e6,
                times[1][3] as f64 / 1.0e6
            );
        }
        eprintln!(
            "sum of paired roof medians: {:.3}->{:.3}ms",
            totals[0] as f64 / 1.0e6,
            totals[1] as f64 / 1.0e6
        );
    }

    #[test]
    fn roof_coarse_policy_caps_are_monotone_and_preserve_the_near_formula() {
        for rise in [0.0, 0.1, 0.2, 0.6, 1.0, 2.4, 5.0, 20.0] {
            // Levels up to 15 never refine roofs; 16 and finer refine and are
            // checked as their own sequence.
            let mut previous = [f64::INFINITY; 2];
            for level in 11..=30 {
                let frame = frame(level);
                let policy = RoofResolution::for_frame(&frame);
                let error = interior_roof_error(&frame, rise, Some(RoofShape::Hipped));
                let allowance = policy.interior_allowance(rise, Some(RoofShape::Hipped));
                let pixel = frame.addr.cell.sample_pitch_m() / 12.0;
                let refining = coarse_building_pixel_m(&frame).is_none();
                let floored = refining && level < vearth_core::cell::MAX_LEVEL;
                assert_eq!(policy.pixel_error, if floored { pixel } else { 0.0 });
                assert!(error <= allowance.max(policy.pixel_error));
                let original = (frame.grid_scale.x.min(frame.grid_scale.z) / 60.0)
                    .min(0.10)
                    .min(rise * 0.02)
                    .max(ROOF_PLANE_ERROR_M);
                let sequence = usize::from(refining);
                assert!(error <= previous[sequence]);
                assert!((ROOF_PLANE_ERROR_M..=0.50).contains(&error));
                assert!(error >= original);
                assert!(
                    error
                        <= (frame.grid_scale.x.min(frame.grid_scale.z) / 60.0)
                            .max(ROOF_PLANE_ERROR_M)
                            .max(policy.pixel_error)
                );
                assert_eq!(
                    RoofResolution::original(&frame)
                        .sampling_tolerance(rise, Some(RoofShape::Hipped))
                        .to_bits(),
                    original.to_bits()
                );
                if level >= 17 {
                    assert_eq!(error.to_bits(), original.max(policy.pixel_error).to_bits());
                }
                if level >= vearth_core::cell::MAX_LEVEL {
                    assert_eq!(error.to_bits(), original.to_bits());
                }
                for shape in [RoofShape::Dome, RoofShape::Onion] {
                    assert_eq!(
                        interior_roof_error(&frame, rise, Some(shape)),
                        ROOF_PLANE_ERROR_M
                    );
                }
                previous[sequence] = error;
            }
        }
        for (level, multiplier) in [
            (11, 5.0),
            (14, 5.0),
            (15, 4.0),
            (16, 2.0),
            (17, 1.0),
            (19, 1.0),
        ] {
            assert_eq!(
                RoofResolution::for_frame(&frame(level)).cap_scale,
                multiplier
            );
        }
        // A fixed pitch isolates the cap transition from the cube-face metric
        // and from the pixel floor.
        for rise in [0.1, 0.2, 0.6, 1.0, 2.4, 20.0] {
            let mut finer = ROOF_PLANE_ERROR_M;
            for level in (11..=22).rev() {
                let policy = RoofResolution::for_frame(&frame(level));
                let error = RoofResolution {
                    sample_error: 0.04 * 2f64.powi(17 - level as i32),
                    pixel_error: 0.0,
                    ..policy
                }
                .sampling_tolerance(rise, Some(RoofShape::Hipped));
                assert!(error >= finer && error <= finer * 2.0 + 1.0e-12);
                finer = error;
            }
        }
    }

    fn assert_same_building_output(a: &BuildingSurfaceData, b: &BuildingSurfaceData) {
        assert_eq!(a.complete, b.complete);
        assert_eq!(a.mesh.vertices, b.mesh.vertices);
        assert_eq!(a.mesh.indices, b.mesh.indices);
        assert_eq!(a.shadow_omissions, b.shadow_omissions);
        assert_eq!(a.collision, b.collision);
        assert_eq!(a.details.len(), b.details.len());
        for (a, b) in a.details.iter().zip(&b.details) {
            assert_eq!(a.mesh.vertices, b.mesh.vertices);
            assert_eq!(a.mesh.indices, b.mesh.indices);
            assert_eq!(a.bound_center, b.bound_center);
            assert_eq!(a.radius_m, b.radius_m);
            assert_eq!(a.min_screen_px, b.min_screen_px);
        }
    }

    fn assert_building_bits(a: &BuildingSurfaceData, b: &BuildingSurfaceData) {
        assert_same_building_output(a, b);
        for (a, b) in std::iter::once(&a.mesh)
            .chain(a.details.iter().map(|detail| &detail.mesh))
            .zip(std::iter::once(&b.mesh).chain(b.details.iter().map(|detail| &detail.mesh)))
        {
            for (a, b) in a.vertices.iter().zip(&b.vertices) {
                assert_eq!(a.pos.map(f32::to_bits), b.pos.map(f32::to_bits));
            }
        }
        for (a, b) in a.collision.iter().zip(&b.collision) {
            assert_eq!(
                a.vertices.map(|p| p.map(f32::to_bits)),
                b.vertices.map(|p| p.map(f32::to_bits))
            );
        }
        for (a, b) in a.details.iter().zip(&b.details) {
            assert_eq!(
                a.bound_center.map(f32::to_bits),
                b.bound_center.map(f32::to_bits)
            );
            assert_eq!(a.radius_m.to_bits(), b.radius_m.to_bits());
            assert_eq!(a.min_screen_px.to_bits(), b.min_screen_px.to_bits());
        }
        for (a, b) in a.shadow_omissions.iter().zip(&b.shadow_omissions) {
            assert_eq!(a.max_deviation_m.to_bits(), b.max_deviation_m.to_bits());
        }
    }

    fn coarse_reuse_fixture(frame: &ChunkFrame, count: usize) -> (FeaturePack, CellOverlay) {
        let mut pack = FeaturePack::empty();
        let span = (frame.grid_scale.x.min(frame.grid_scale.z) * 24.0).min(1000.0);
        let side = (count as f64).sqrt().ceil() as usize;
        let pitch = span / side as f64;
        for i in 0..count {
            let center = DVec2::new((i % side) as f64, (i / side) as f64) * pitch
                - DVec2::splat(span * 0.5 - pitch * 0.5);
            let radius = pitch * 0.3;
            let outer: Vec<_> = (0..24)
                .map(|j| {
                    let angle = j as f64 * std::f64::consts::TAU / 24.0;
                    let r = radius * if j % 3 == 0 { 0.75 } else { 1.0 };
                    (center + DVec2::new(angle.cos(), angle.sin()) * r)
                        .as_vec2()
                        .to_array()
                })
                .collect();
            let mut rings = vec![outer];
            if i % 3 == 0 {
                rings.push(
                    square(-0.2, 0.2)
                        .into_iter()
                        .map(|p| {
                            (center + DVec2::from_array(p.map(f64::from)) * radius)
                                .as_vec2()
                                .to_array()
                        })
                        .collect(),
                );
            }
            pack.buildings.push(Building {
                rings,
                location_rad: None,
                height_m: 12.0 + (i % 4) as f32 * 3.0,
                min_height_m: 0.0,
                seed: i as u64 + 42,
                kind: BuildingKind::Building,
                roof_shape: Some([RoofShape::Flat, RoofShape::Gabled][i % 2]),
                use_kind: Default::default(),
            });
        }
        let overlay = tier0::build_overlay(
            frame.addr.cell,
            frame,
            &pack,
            &|_, _| 100.0,
            0,
            None,
            None,
            Some(&|_| true),
        );
        (pack, overlay)
    }

    #[test]
    fn coarse_footprint_reuse_preserves_generation_collision_and_retry_bits() {
        for level in [11, 15, 17, 19] {
            let frame = frame(level);
            // Exercise both sheltered and exposed parent roofs below an
            // elevated part; both paths reuse the body's collision boundary.
            for part_bottom in [None, Some(7.0), Some(16.0)] {
                let (mut pack, mut overlay) = coarse_reuse_fixture(&frame, 4);
                if let Some(min_height_m) = part_bottom {
                    let (lo, hi) = bounds(&pack.buildings[0].rings).unwrap();
                    let center = (lo + hi) * 0.5;
                    let radius = (hi - lo) * 0.5;
                    pack.buildings.push(Building {
                        rings: vec![
                            [[0.3, -0.4], [0.5, -0.4], [0.5, 0.4], [0.3, 0.4]]
                                .map(|p| {
                                    (center + DVec2::from_array(p) * radius)
                                        .as_vec2()
                                        .to_array()
                                })
                                .to_vec(),
                        ],
                        location_rad: None,
                        height_m: 24.0,
                        min_height_m,
                        seed: 500,
                        kind: BuildingKind::Part,
                        roof_shape: Some(RoofShape::Flat),
                        use_kind: Default::default(),
                    });
                    overlay = tier0::build_overlay(
                        frame.addr.cell,
                        &frame,
                        &pack,
                        &|_, _| 100.0,
                        0,
                        None,
                        None,
                        Some(&|_| true),
                    );
                    let plan = prepare_envelopes(
                        &frame,
                        &pack,
                        &overlay,
                        &[u32::MAX; N],
                        &|_, _, _| Some(100.0),
                        &|_, _, _| None,
                    )
                    .unwrap();
                    assert!(plan.overlaps[0].contains(&4));
                    assert!(plan.datums[0].is_some() && plan.datums[4].is_some());
                }
                for budget in [usize::MAX, 1] {
                    let old = generate_bounded_with_coarse_reuse::<false>(
                        &frame,
                        &pack,
                        &overlay,
                        &[u32::MAX; N],
                        &|_, _, _| Some(100.0),
                        &|_, _, _| None,
                        budget,
                    )
                    .unwrap();
                    let new = generate_bounded_with_coarse_reuse::<true>(
                        &frame,
                        &pack,
                        &overlay,
                        &[u32::MAX; N],
                        &|_, _, _| Some(100.0),
                        &|_, _, _| None,
                        budget,
                    )
                    .unwrap();
                    assert!(!old.mesh.indices.is_empty());
                    assert_building_bits(&old, &new);
                }
            }
        }
    }

    #[test]
    fn coarse_footprint_cache_keeps_order_failures_and_shared_byte_limit() {
        let source = vec![square(-20.0, 20.0), square(-5.0, 5.0)];
        let bits = |cells: &[[DVec2; 4]]| {
            cells
                .iter()
                .map(|cell| cell.map(|point| point.to_array().map(f64::to_bits)))
                .collect::<Vec<_>>()
        };
        let expected = Footprint::new(vec![&source], vec![])
            .unwrap()
            .roof_cells(4096.0)
            .unwrap();
        let bytes = expected.len() * std::mem::size_of::<[DVec2; 4]>();
        let budget = Rc::new(std::cell::Cell::new(bytes));
        for cacheable in [true, false] {
            let mut footprint = Footprint::new(vec![&source], vec![]).unwrap();
            footprint.coarse_cache = Some(CoarseFootprintCache {
                remaining_bytes: Rc::clone(&budget),
                cells: std::cell::OnceCell::new(),
            });
            for _ in 0..3 {
                let cells = footprint.coarse_roof_cells().unwrap();
                assert_eq!(bits(&cells), bits(&expected));
                assert_eq!(matches!(cells, std::borrow::Cow::Borrowed(_)), cacheable);
                assert_eq!(budget.get(), 0);
            }
            assert_eq!(
                bits(&footprint.roof_cells(4096.0).unwrap()),
                bits(&expected)
            );
        }
        let mut failure = Footprint::with_limits(vec![&source], vec![], 512, 0).unwrap();
        failure.coarse_cache = Some(CoarseFootprintCache {
            remaining_bytes: Rc::clone(&budget),
            cells: std::cell::OnceCell::new(),
        });
        assert!(failure.roof_cells(4096.0).is_none());
        assert!(failure.coarse_roof_cells().is_none());
        assert!(matches!(
            failure.coarse_cache.as_ref().unwrap().cells.get(),
            Some(None)
        ));
        assert!(failure.coarse_roof_cells().is_none());
        assert_eq!(budget.get(), 0);
    }

    #[test]
    #[ignore = "paired complete building preparation and emission timing"]
    fn coarse_footprint_reuse_generation_benchmark() {
        for level in [14, 16] {
            let frame = frame(level);
            let (pack, overlay) = coarse_reuse_fixture(&frame, 64);
            let build = |reuse| {
                if reuse {
                    generate_bounded_with_coarse_reuse::<true>(
                        &frame,
                        &pack,
                        &overlay,
                        &[u32::MAX; N],
                        &|_, _, _| Some(100.0),
                        &|_, _, _| None,
                        crate::built_surface::BUILDING_BASE_BUDGET_BYTES,
                    )
                    .unwrap()
                } else {
                    generate_bounded_with_coarse_reuse::<false>(
                        &frame,
                        &pack,
                        &overlay,
                        &[u32::MAX; N],
                        &|_, _, _| Some(100.0),
                        &|_, _, _| None,
                        crate::built_surface::BUILDING_BASE_BUDGET_BYTES,
                    )
                    .unwrap()
                }
            };
            let reference = build(false);
            assert_building_bits(&reference, &build(true));
            let plan = prepare_envelopes(
                &frame,
                &pack,
                &overlay,
                &[u32::MAX; N],
                &|_, _, _| Some(100.0),
                &|_, _, _| None,
            )
            .unwrap();
            let bytes: usize = plan
                .domains
                .iter()
                .flatten()
                .filter_map(|d| d.footprint.coarse_cache.as_ref()?.cells.get()?.as_ref())
                .map(|cells| std::mem::size_of_val(cells.as_ref()))
                .sum();
            assert!(bytes <= COARSE_FOOTPRINT_CACHE_BYTES);
            let mut times = [Vec::new(), Vec::new()];
            for round in 0..16 {
                for mode in [round % 2, 1 - round % 2] {
                    let started = vearth_platform::time::Instant::now();
                    let result = build(mode == 1);
                    let elapsed = started.elapsed().as_nanos();
                    std::hint::black_box(result);
                    if round >= 2 {
                        times[mode].push(elapsed);
                    }
                }
            }
            times.iter_mut().for_each(|t| t.sort_unstable());
            eprintln!(
                "coarse footprint L{level} buildings={} cached_bytes={bytes} vertices={} old_ns={} reuse_ns={}",
                pack.buildings.len(),
                reference.mesh.vertices.len(),
                times[0][7],
                times[1][7]
            );
        }
    }

    #[test]
    fn roof_coarse_policy_preserves_near_analytic_measured_and_simple_envelopes() {
        for level in [14, 16, 17, 19] {
            for mode in 0..4 {
                let frame = frame(level);
                let rings = if mode == 1 {
                    vec![square(-8.0, 8.0)]
                } else {
                    vec![square(-8.0, 8.0), square(-2.0, 2.0)]
                };
                let mut pack = pack_with(rings, 18.0);
                pack.buildings[0].roof_shape = Some(RoofShape::Hipped);
                // The near sampled control also exercises the exposed parent.
                if mode == 0 {
                    pack.buildings.push(Building {
                        rings: vec![vec![[3.0, -6.0], [6.0, -6.0], [6.0, 6.0], [3.0, 6.0]]],
                        location_rad: None,
                        height_m: 22.0,
                        min_height_m: 8.0,
                        seed: 43,
                        kind: BuildingKind::Part,
                        roof_shape: Some(RoofShape::Flat),
                        use_kind: Default::default(),
                    });
                }
                let mut overlay = tier0::build_overlay(
                    frame.addr.cell,
                    &frame,
                    &pack,
                    &|_, _| 100.0,
                    0,
                    None,
                    None,
                    Some(&|_| true),
                );
                if mode == 2 {
                    for stamp in &mut overlay.source_buildings {
                        stamp.lidar = true;
                        stamp.top_geo = 118.0;
                        stamp.height = 18.0;
                    }
                }
                let ground = |_, _, _| Some(100.0);
                let calls = std::cell::Cell::new(0usize);
                let measured = |_, _, _| {
                    calls.set(calls.get() + 1);
                    Some(118.0)
                };
                let eligible = [u32::MAX; N];
                let mut plan =
                    prepare_envelopes(&frame, &pack, &overlay, &eligible, &ground, &measured)
                        .unwrap();
                let actual = generate_prepared_envelopes::<true>(
                    &frame,
                    &pack,
                    &overlay,
                    &eligible,
                    &ground,
                    &measured,
                    mode == 3,
                    usize::MAX,
                    &plan,
                )
                .unwrap();
                assert!(actual.complete && !actual.mesh.indices.is_empty());
                if mode == 2 {
                    assert!(calls.get() > 0);
                }
                plan.roof_resolution = RoofResolution::original(&frame);
                let old = generate_prepared_envelopes::<true>(
                    &frame,
                    &pack,
                    &overlay,
                    &eligible,
                    &ground,
                    &measured,
                    mode == 3,
                    usize::MAX,
                    &plan,
                )
                .unwrap();
                assert_eq!(actual.collision, old.collision);
                // Refined sampled roofs may use the pixel floor below the
                // finest level; analytic, measured and simple roofs may not.
                if level >= vearth_core::cell::MAX_LEVEL || mode != 0 {
                    assert_same_building_output(&actual, &old);
                }
            }
        }
    }

    fn roof_policy_cells(
        roof: &RoofField,
        rings: &Rings,
        parent: bool,
    ) -> (Vec<[DVec2; 4]>, Vec<[DVec2; 2]>) {
        let source = Footprint::new(vec![rings], vec![]).unwrap();
        let boundary = source.boundary();
        if parent {
            return (
                source
                    .roof_cells(2.0)
                    .or_else(|| source.roof_cells(4096.0))
                    .unwrap(),
                boundary,
            );
        }
        let rotated: Rings = rings
            .iter()
            .map(|ring| {
                ring.iter()
                    .map(|p| {
                        roof.rotate(DVec2::from_array(p.map(f64::from)))
                            .as_vec2()
                            .to_array()
                    })
                    .collect()
            })
            .collect();
        let domain = Footprint::new(vec![&rotated], vec![]).unwrap();
        let cells = domain
            .roof_cells(3.0)
            .or_else(|| domain.roof_cells(4096.0))
            .unwrap()
            .into_iter()
            .map(|q| q.map(|p| roof.unrotate(p)))
            .collect();
        (cells, boundary)
    }

    fn roof_policy_stats(
        cells: &[[DVec2; 4]],
        boundary: &[[DVec2; 2]],
        roof: &RoofField,
        error: f64,
    ) -> (RoofGeometry, crate::contracts::RoofWorkCounters, usize) {
        let mut stats = crate::contracts::RoofWorkCounters::default();
        let calls = std::cell::Cell::new(0usize);
        let mesh = roof_geometry_profiled(
            cells.to_vec(),
            boundary,
            &|p| {
                calls.set(calls.get() + 1);
                roof.height(p)
            },
            true,
            error,
            roof.height_bounds(),
            Some(&mut stats),
        )
        .unwrap();
        (mesh, stats, calls.get())
    }

    fn roof_policy_error(mesh: &RoofGeometry, roof: &RoofField, subdivisions: usize) -> (f64, f64) {
        let mut interior = 0.0f64;
        let mut boundary = 0.0f64;
        for tri in &mesh.triangles {
            for a in 0..=subdivisions {
                for b in 0..=subdivisions - a {
                    let w = [
                        a as f64 / subdivisions as f64,
                        b as f64 / subdivisions as f64,
                        (subdivisions - a - b) as f64 / subdivisions as f64,
                    ];
                    let point = (0..3).map(|i| tri.points[i] * w[i]).sum::<DVec2>();
                    let height = (0..3).map(|i| tri.heights[i] * w[i]).sum::<f64>();
                    interior = interior.max((height - roof.height(point)).abs());
                }
            }
        }
        let key = |p: DVec2, h: f64| [p.x, p.y, h].map(|v| if v == 0.0 { 0 } else { v.to_bits() });
        let vertices: rustc_hash::FxHashSet<_> = mesh
            .triangles
            .iter()
            .flat_map(|tri| (0..3).map(|i| key(tri.points[i], tri.heights[i])))
            .collect();
        for edge in &mesh.boundary {
            let middle = (edge.points[0] + edge.points[1]) * 0.5;
            boundary = boundary
                .max((roof.height(middle) - (edge.heights[0] + edge.heights[1]) * 0.5).abs());
            for i in 0..2 {
                assert!(
                    vertices.contains(&key(edge.points[i], edge.heights[i])),
                    "roof/closure/eave endpoint separated"
                );
            }
        }
        (interior, boundary)
    }

    // Query both rendered surfaces at one common probe set. In particular an
    // old unrefined fallback must not get a sparser validation sample merely
    // because it contains fewer triangles than the successful refinement.
    fn roof_policy_common_errors(
        a: &RoofGeometry,
        b: &RoofGeometry,
        roof: &RoofField,
        subdivisions: usize,
    ) -> ([f64; 2], usize, f64) {
        struct Query<'a> {
            mesh: &'a RoofGeometry,
            lo: DVec2,
            scale: DVec2,
            bins: Vec<Vec<usize>>,
        }
        impl<'a> Query<'a> {
            fn bin(&self, p: DVec2) -> [usize; 2] {
                let p = (p - self.lo) * self.scale;
                [
                    p.x.floor().clamp(0.0, 63.0) as usize,
                    p.y.floor().clamp(0.0, 63.0) as usize,
                ]
            }
            fn new(mesh: &'a RoofGeometry) -> Self {
                let mut lo = DVec2::splat(f64::INFINITY);
                let mut hi = -lo;
                for tri in &mesh.triangles {
                    for p in tri.points {
                        lo = lo.min(p);
                        hi = hi.max(p);
                    }
                }
                let mut result = Self {
                    mesh,
                    lo,
                    scale: DVec2::splat(64.0) / (hi - lo).max(DVec2::splat(1.0e-9)),
                    bins: vec![vec![]; 64 * 64],
                };
                for (i, tri) in mesh.triangles.iter().enumerate() {
                    let lower = result.bin(tri.points[0].min(tri.points[1]).min(tri.points[2]));
                    let upper = result.bin(tri.points[0].max(tri.points[1]).max(tri.points[2]));
                    for y in lower[1]..=upper[1] {
                        for x in lower[0]..=upper[0] {
                            result.bins[y * 64 + x].push(i);
                        }
                    }
                }
                result
            }
            fn height(&self, p: DVec2) -> Option<(f64, f64)> {
                let [x, y] = self.bin(p);
                let interior = self.bins[y * 64 + x].iter().find_map(|&i| {
                    let tri = &self.mesh.triangles[i];
                    let a = tri.points[1] - tri.points[0];
                    let b = tri.points[2] - tri.points[0];
                    let d = p - tri.points[0];
                    let det = a.perp_dot(b);
                    if det.abs() < 1.0e-14 {
                        return None;
                    }
                    let u = d.perp_dot(b) / det;
                    let v = a.perp_dot(d) / det;
                    let w = 1.0 - u - v;
                    (u.min(v).min(w) >= -1.0e-7)
                        .then(|| tri.heights[0] * w + tri.heights[1] * u + tri.heights[2] * v)
                });
                if let Some(height) = interior {
                    return Some((height, 0.0));
                }
                let mut nearest = (f64::INFINITY, 0.0);
                for by in y.saturating_sub(1)..=(y + 1).min(63) {
                    for bx in x.saturating_sub(1)..=(x + 1).min(63) {
                        for &i in &self.bins[by * 64 + bx] {
                            let tri = &self.mesh.triangles[i];
                            for edge in 0..3 {
                                let next = (edge + 1) % 3;
                                let a = tri.points[edge];
                                let d = tri.points[next] - a;
                                let t = ((p - a).dot(d) / d.length_squared()).clamp(0.0, 1.0);
                                let distance = p.distance(a + d * t);
                                if distance < nearest.0 {
                                    nearest = (
                                        distance,
                                        tri.heights[edge] * (1.0 - t) + tri.heights[next] * t,
                                    );
                                }
                            }
                        }
                    }
                }
                // Slender triangles make barycentric division ill-conditioned.
                // Require an actual edge within double-coordinate roundoff;
                // a metric gap or changed hole is still rejected.
                let roundoff = 128.0 * f64::EPSILON * p.abs().max_element().max(1.0);
                assert!(
                    nearest.0 <= roundoff,
                    "common probe {p:?}: nearest rendered edge {}m exceeds coordinate roundoff {roundoff}m",
                    nearest.0
                );
                (nearest.0 <= roundoff).then_some((nearest.1, nearest.0))
            }
        }
        let queries = [Query::new(a), Query::new(b)];
        let mut errors = [0.0f64; 2];
        let mut probes = 0;
        let mut maximum_roundoff = 0.0f64;
        for mesh in [a, b] {
            for tri in &mesh.triangles {
                for i in 0..=subdivisions {
                    for j in 0..=subdivisions - i {
                        let w = [
                            i as f64 / subdivisions as f64,
                            j as f64 / subdivisions as f64,
                            (subdivisions - i - j) as f64 / subdivisions as f64,
                        ];
                        let p = (0..3).map(|n| tri.points[n] * w[n]).sum::<DVec2>();
                        let expected = roof.height(p);
                        for (mode, query) in queries.iter().enumerate() {
                            let (height, roundoff) = query.height(p).unwrap_or_else(|| {
                                panic!("mode{mode} lost common roof probe {p:?}")
                            });
                            maximum_roundoff = maximum_roundoff.max(roundoff);
                            errors[mode] = errors[mode].max((height - expected).abs());
                        }
                        probes += 1;
                    }
                }
            }
        }
        (errors, probes, maximum_roundoff)
    }

    #[test]
    fn roof_coarse_policy_common_oracle_handles_slender_translated_triangles() {
        let origin = DVec2::new(216.36454728432523, -398.00380817127495);
        let mesh = RoofGeometry {
            triangles: vec![RoofTriangle {
                points: [
                    origin,
                    origin + DVec2::new(1.0e-8, 1.0),
                    origin + DVec2::new(2.0e-8, 3.0),
                ],
                heights: [100.0; 3],
            }],
            boundary: vec![],
        };
        let roof = RoofField::new(
            &vec![square(-1000.0, 1000.0)],
            Some(RoofShape::Flat),
            100.0,
            100.0,
            0.7,
        );
        let (errors, count, _) = roof_policy_common_errors(&mesh, &mesh, &roof, 6);
        assert_eq!(count, 56);
        assert!(errors.iter().all(|error| *error < 1.0e-9));
    }

    #[test]
    #[should_panic(expected = "nearest rendered edge")]
    fn roof_coarse_policy_common_oracle_rejects_real_coverage_gaps() {
        let triangle = RoofTriangle {
            points: [DVec2::ZERO, DVec2::X, DVec2::Y],
            heights: [100.0; 3],
        };
        let a = RoofGeometry {
            triangles: vec![triangle],
            boundary: vec![],
        };
        let b = RoofGeometry {
            triangles: vec![RoofTriangle {
                points: triangle.points.map(|p| p + DVec2::splat(0.01)),
                ..triangle
            }],
            boundary: vec![],
        };
        let roof = RoofField::new(
            &vec![square(-2.0, 2.0)],
            Some(RoofShape::Flat),
            100.0,
            100.0,
            0.7,
        );
        roof_policy_common_errors(&a, &b, &roof, 6);
    }

    fn roof_planar_area(mesh: &RoofGeometry) -> f64 {
        mesh.triangles
            .iter()
            .map(|tri| {
                (tri.points[1] - tri.points[0])
                    .perp_dot(tri.points[2] - tri.points[0])
                    .abs()
                    * 0.5
            })
            .sum()
    }

    #[test]
    fn roof_coarse_policy_preserves_dense_surfaces_boundaries_and_courtyards() {
        let mut cases = minified_roof_cases();
        cases.push(("gabled", cases[1].1.clone(), RoofShape::Gabled));
        cases.push(("gambrel", cases[0].1.clone(), RoofShape::Gambrel));
        cases.push(("onion", vec![square(-10.0, 10.0)], RoofShape::Onion));
        let mut savings = 0isize;
        for (name, rings, shape) in cases {
            let direction = DVec2::from_angle(0.37);
            let rings: Rings = rings
                .iter()
                .map(|ring| {
                    ring.iter()
                        .map(|p| {
                            let p = DVec2::from_array(p.map(f64::from));
                            (DVec2::new(-43.0, 27.0)
                                + direction * p.x
                                + DVec2::new(-direction.y, direction.x) * p.y)
                                .as_vec2()
                                .to_array()
                        })
                        .collect()
                })
                .collect();
            for (rise, level, parent) in [
                (0.6, 14, false),
                (2.4, 13, false),
                (5.0, 16, true),
                (5.0, 17, false),
            ] {
                let frame = frame(level);
                let roof = RoofField::new(&rings, Some(shape), 100.0, 100.0 + rise, 0.7);
                let (cells, boundary) = roof_policy_cells(&roof, &rings, parent);
                let old_error =
                    RoofResolution::original(&frame).sampling_tolerance(rise, Some(shape));
                let error = interior_roof_error(&frame, rise, Some(shape));
                let policy = RoofResolution::for_frame(&frame);
                let allowance = policy
                    .interior_allowance(rise, Some(shape))
                    .max(policy.pixel_error);
                let (old, old_stats, old_calls) =
                    roof_policy_stats(&cells, &boundary, &roof, old_error);
                let (new, stats, calls) = roof_policy_stats(&cells, &boundary, &roof, error);
                assert_eq!(stats.initial_cells, old_stats.initial_cells);
                let area = roof_planar_area(&old);
                assert!((roof_planar_area(&new) - area).abs() < area * 1.0e-8);
                assert!(new.triangles.len() <= MAX_ROOF_TRIANGLES);
                assert_eq!(stats.fallback_invalid, 0);
                let (interior, boundary_error) = roof_policy_error(&new, &roof, 12);
                if stats.fallback_budget + stats.fallback_depth == 0
                    && error.to_bits() != old_error.to_bits()
                {
                    assert!(
                        interior <= 2.0 * allowance + 1.0e-5,
                        "{name} L{level}: interior {interior}, budget {error}"
                    );
                    assert!(
                        boundary_error <= ROOF_PLANE_ERROR_M + 1.0e-5,
                        "{name}: boundary {boundary_error}"
                    );
                } else if stats.fallback_budget + stats.fallback_depth != 0 {
                    let base = roof_geometry_with_error(
                        cells.clone(),
                        &boundary,
                        &|p| roof.height(p),
                        false,
                        error,
                    )
                    .unwrap();
                    assert_same_roof_geometry(&new, &base);
                }
                if error.to_bits() == old_error.to_bits() {
                    assert_same_roof_geometry(&new, &old);
                    assert_eq!(calls, old_calls);
                }
                // Area parity plus direct hole/interior samples detects filled courtyards.
                let domain = Footprint::new(vec![&rings], vec![]).unwrap();
                for tri in &new.triangles {
                    let center = tri.points.iter().copied().sum::<DVec2>() / 3.0;
                    assert!(
                        domain.contains(center)
                            || boundary.iter().any(|edge| {
                                let d = edge[1] - edge[0];
                                let t = ((center - edge[0]).dot(d) / d.length_squared())
                                    .clamp(0.0, 1.0);
                                center.distance(edge[0] + d * t) < 0.001
                            })
                    );
                }
                savings += old_calls as isize - calls as isize;
            }
        }
        assert!(
            savings > 100,
            "must exercise reduced work, not only unchanged paths"
        );
    }

    #[test]
    #[ignore = "paired whole-source roof policy benchmark; requires VEARTH_ROOF_CONCAVITY_FIXTURE"]
    fn roof_coarse_policy_actual_source_benchmark() {
        use vearth_core::cell::ChunkAddr;
        let mut totals = [0u128; 2];
        let mut dense_failures = Vec::new();
        let mut triangles = [0usize; 2];
        let mut total_calls = [0usize; 2];
        for case in roof_source_cases() {
            let frame = ChunkFrame::of(ChunkAddr {
                cell: case.cell,
                k: 0,
            });
            let mut roof = RoofField::new(
                &case.rings,
                Some(case.shape),
                100.0,
                100.0 + case.rise,
                case.pitch,
            );
            roof.tiered = case.tiered;
            if let Some(extent) = case.expected_extent {
                assert_eq!(
                    (roof.bounds[1] - roof.bounds[0])
                        .to_array()
                        .map(f64::to_bits),
                    extent.map(f64::to_bits)
                );
            }
            let (cells, boundary) = roof_policy_cells(&roof, &case.rings, false);
            let errors = [
                RoofResolution::original(&frame).sampling_tolerance(case.rise, Some(case.shape)),
                interior_roof_error(&frame, case.rise, Some(case.shape)),
            ];
            let allowance =
                RoofResolution::for_frame(&frame).interior_allowance(case.rise, Some(case.shape));
            let outputs = errors.map(|error| roof_policy_stats(&cells, &boundary, &roof, error));
            let area = roof_planar_area(&outputs[0].0);
            assert!((roof_planar_area(&outputs[1].0) - area).abs() < area * 1.0e-8);
            if errors[0].to_bits() == errors[1].to_bits() {
                assert_same_roof_geometry(&outputs[0].0, &outputs[1].0);
                assert_eq!(outputs[0].2, outputs[1].2);
            }
            let (common_errors, common_probes, maximum_roundoff) =
                roof_policy_common_errors(&outputs[0].0, &outputs[1].0, &roof, 6);
            let old_fell_back = outputs[0].1.fallback_depth + outputs[0].1.fallback_budget != 0;
            let quality_limit =
                (2.0 * allowance).max(if old_fell_back { common_errors[0] } else { 0.0 });
            let (lat, lon) = frame.column_latlon(0.5, 0.5);
            let projection = Projection {
                frame: &frame,
                lat,
                lon,
            };
            let mut bytes = [0usize; 2];
            let mut emitted = [0usize; 2];
            let mut dense = [(0.0, 0.0); 2];
            for mode in 0..2 {
                let (mesh, stats, calls) = &outputs[mode];
                assert!(mesh.triangles.len() <= MAX_ROOF_TRIANGLES);
                assert_eq!(stats.fallback_invalid, 0);
                dense[mode] = roof_policy_error(mesh, &roof, 6);
                // The unchanged whole-source fallback is explicitly outside the refined-error contract.
                if mode == 1
                    && errors[0].to_bits() != errors[1].to_bits()
                    && stats.fallback_budget + stats.fallback_depth == 0
                {
                    // A point on an almost-degenerate shared edge can have
                    // multiple primitive interpolants. Retain the direct
                    // per-primitive probes as well as the common query set.
                    if common_errors[mode].max(dense[mode].0) > quality_limit + 1.0e-5 {
                        dense_failures.push((
                            case.name.clone(),
                            common_errors[mode].max(dense[mode].0),
                            quality_limit,
                        ));
                    }
                    assert!(
                        dense[mode].1 <= ROOF_PLANE_ERROR_M + 1.0e-5,
                        "{} mode{mode}: boundary {}",
                        case.name,
                        dense[mode].1
                    );
                }
                let mut visual = MeshBuffers::default();
                emit_roof(
                    &projection,
                    &mut visual,
                    mesh,
                    SurfaceAppearance {
                        material: crate::material::TILE_ROOF,
                        tint: [100; 3],
                    },
                );
                crate::built_surface::compact_mesh(&mut visual);
                bytes[mode] = crate::built_surface::mesh_bytes(&visual);
                emitted[mode] = visual.indices.len() / 3;
                triangles[mode] += mesh.triangles.len();
                total_calls[mode] += calls;
            }
            let mut times = [vec![], vec![]];
            for round in 0..5 {
                for mode in [round % 2, 1 - round % 2] {
                    let start = vearth_platform::time::Instant::now();
                    std::hint::black_box(
                        roof_geometry_profiled(
                            cells.clone(),
                            &boundary,
                            &|p| roof.height(p),
                            true,
                            errors[mode],
                            roof.height_bounds(),
                            None,
                        )
                        .unwrap(),
                    );
                    times[mode].push(start.elapsed().as_nanos());
                }
            }
            for mode in 0..2 {
                times[mode].sort();
                totals[mode] += times[mode][2];
            }
            eprintln!(
                "{}: controlled eave100m/full-source; sampling_tolerance={errors:?}; allowance={allowance}; calls={}->{}; triangles={}->{}; clipped_triangles={emitted:?}; clipped_bytes={bytes:?}; fallback(depth,budget)={:?}->{:?}; discarded={}->{}; dense(interior,boundary)={dense:?}; common_errors={common_errors:?}; common_probes={common_probes}; query_roundoff_m={maximum_roundoff}; quality_limit={quality_limit}; median_ms={:.3}->{:.3}",
                case.name,
                outputs[0].2,
                outputs[1].2,
                outputs[0].0.triangles.len(),
                outputs[1].0.triangles.len(),
                (outputs[0].1.fallback_depth, outputs[0].1.fallback_budget),
                (outputs[1].1.fallback_depth, outputs[1].1.fallback_budget),
                outputs[0].1.discarded_triangles,
                outputs[1].1.discarded_triangles,
                times[0][2] as f64 / 1e6,
                times[1][2] as f64 / 1e6
            );
        }
        eprintln!(
            "paired source-roof policy aggregate medians_ms={:.3}->{:.3}; triangles={triangles:?}; calls={total_calls:?}",
            totals[0] as f64 / 1e6,
            totals[1] as f64 / 1e6
        );
        assert!(
            dense_failures.is_empty(),
            "changed-policy dense envelope failures: {dense_failures:?}"
        );
    }

    fn minified_roof_cases() -> Vec<(&'static str, Rings, RoofShape)> {
        vec![
            (
                "courtyard",
                vec![square(-28.0, 28.0), square(-12.0, 12.0)],
                RoofShape::Hipped,
            ),
            (
                "concave",
                vec![vec![
                    [-30.0, -22.0],
                    [30.0, -22.0],
                    [30.0, -10.0],
                    [-12.0, -10.0],
                    [-12.0, 16.0],
                    [30.0, 16.0],
                    [30.0, 28.0],
                    [-30.0, 28.0],
                ]],
                RoofShape::Hipped,
            ),
            ("dome", vec![square(-12.0, 12.0)], RoofShape::Dome),
            (
                "mansard",
                vec![square(-15.0, 15.0), square(-5.0, 5.0)],
                RoofShape::Mansard,
            ),
        ]
    }

    fn concavity_known_samples(points: [DVec2; 3], at: &dyn Fn(DVec2) -> f64) -> [f64; 7] {
        [
            points[0],
            points[1],
            points[2],
            (points[0] + points[1]) * 0.5,
            (points[1] + points[2]) * 0.5,
            (points[2] + points[0]) * 0.5,
            (points[0] + points[1] + points[2]) / 3.0,
        ]
        .map(at)
    }

    #[test]
    fn roof_concavity_precheck_respects_matching_tolerance_and_conditioning() {
        let points = [DVec2::ZERO, DVec2::new(4.0, 0.0), DVec2::new(0.0, 3.0)];
        let check = |points: [DVec2; 3], heights, bounds| {
            let a = (points[1] - points[0]) * 0.02;
            let b = (points[2] - points[0]) * 0.02;
            roof_samples_cannot_be_concave(points, heights, bounds, a, b, a.perp_dot(b))
        };
        for base in [-400.0, 100.0, 10_000.0] {
            for index in [3, 6] {
                for delta in [0.001999, 0.002, 0.002001] {
                    let mut heights = [base; 7];
                    heights[index] -= delta;
                    let actual = check(points, heights, [base - 1.0, base + 1.0]);
                    assert_eq!(
                        actual,
                        delta > 0.002,
                        "base={base},delta={delta},index={index}"
                    );
                }
            }
        }
        let mut heights = [100.0; 7];
        heights[3] -= 0.1;
        assert!(!check(
            points.map(|p| p + DVec2::splat(1.0e14)),
            heights,
            [99.0, 101.0]
        ));
        assert!(!check(
            [DVec2::ZERO, DVec2::X, DVec2::new(2.0, 1.0e-15)],
            heights,
            [99.0, 101.0]
        ));
        assert!(!check(points, heights, [f64::NEG_INFINITY, f64::INFINITY]));
        assert!(
            !check(points, heights, [100.0, 101.0]),
            "known samples outside bound disable proof"
        );
    }

    #[test]
    fn roof_concavity_precheck_keeps_accepted_planes_at_the_two_millimeter_boundary() {
        let at = |p: DVec2| 100.0 - 0.2 * p.x.abs();
        let mut accepted = 0;
        for left in [-8.0, -6.0, -4.0, -2.0] {
            for right in [2.0, 3.0, 5.0, 7.0] {
                let points = [
                    DVec2::new(left, -3.0),
                    DVec2::new(left + 0.5, -1.0),
                    DVec2::new(right, 4.0),
                ];
                let known = concavity_known_samples(points, &at);
                for delta in [
                    0.0,
                    0.000999,
                    0.001_f64.next_down(),
                    0.001,
                    0.001_f64.next_up(),
                ] {
                    let mut known = known;
                    known[0] += delta;
                    known[1] += delta;
                    known[3] -= delta;
                    let render = |bounds| {
                        let mut sampler = RoofSampler::<false> {
                            stats: None,
                            failure: 0,
                            height_bounds: bounds,
                            at: &at,
                            interior_error: ROOF_PLANE_ERROR_M,
                            boundary_error: ROOF_PLANE_ERROR_M,
                            cache: rustc_hash::FxHashMap::default(),
                        };
                        sampler.planar_sections(points, known)
                    };
                    let old = render(None);
                    let new = render(Some([90.0, 110.0]));
                    assert_eq!(old.is_some(), new.is_some());
                    if let (Some(a), Some(b)) = (old, new) {
                        accepted += 1;
                        assert_same_roof_geometry(
                            &RoofGeometry {
                                triangles: a,
                                boundary: vec![],
                            },
                            &RoofGeometry {
                                triangles: b,
                                boundary: vec![],
                            },
                        );
                    }
                }
            }
        }
        assert!(
            accepted > 5,
            "must exercise successful fits, not only rejection parity"
        );
    }

    fn compare_concavity_roof(
        rings: &Rings,
        shape: RoofShape,
        eave: f64,
        rise: f64,
        pitch: f32,
        test_frame: &ChunkFrame,
        tiered: bool,
    ) -> (u64, usize, usize) {
        let mut roof = RoofField::new(rings, Some(shape), eave, eave + rise, pitch);
        roof.tiered = tiered;
        let domain = Footprint::new(vec![rings], Vec::new()).unwrap();
        let boundary = domain.boundary();
        let rotated: Rings = rings
            .iter()
            .map(|r| {
                r.iter()
                    .map(|p| {
                        roof.rotate(DVec2::from_array(p.map(f64::from)))
                            .as_vec2()
                            .to_array()
                    })
                    .collect()
            })
            .collect();
        let rotated_domain = Footprint::new(vec![&rotated], Vec::new()).unwrap();
        let cells: Vec<_> = rotated_domain
            .roof_cells(3.0)
            .or_else(|| rotated_domain.roof_cells(4096.0))
            .unwrap()
            .into_iter()
            .map(|q| q.map(|p| roof.unrotate(p)))
            .collect();
        let calls = std::cell::Cell::new(0);
        let at = |p| {
            calls.set(calls.get() + 1);
            roof.height(p)
        };
        let error = interior_roof_error(test_frame, rise, Some(shape));
        let old = roof_geometry_with_error(cells.clone(), &boundary, &at, true, error).unwrap();
        let old_calls = calls.replace(0);
        let mut stats = crate::contracts::RoofWorkCounters::default();
        let new = roof_geometry_profiled(
            cells,
            &boundary,
            &at,
            true,
            error,
            roof.height_bounds(),
            Some(&mut stats),
        )
        .unwrap();
        assert_same_roof_geometry(&new, &old);
        assert!(calls.get() <= old_calls);
        (stats.plane_concavity_rejections, old_calls, calls.get())
    }

    #[test]
    fn roof_concavity_precheck_preserves_exact_field_meshes_across_profiles_and_scales() {
        let mut rejects = 0;
        for (_, rings, shape) in minified_roof_cases() {
            for (translation, scale, eave, level) in [
                (DVec2::ZERO, 1.0, -400.0, 14),
                (DVec2::new(100_000.0, -70_000.0), 0.5, 10_000.0, 19),
                (DVec2::new(-93.0, 131.0), 1.1, 100.0, 19),
            ] {
                let direction = DVec2::from_angle(0.37);
                let rings: Rings = rings
                    .iter()
                    .map(|r| {
                        r.iter()
                            .map(|p| {
                                let p = DVec2::from_array(p.map(f64::from)) * scale;
                                (translation
                                    + direction * p.x
                                    + DVec2::new(-direction.y, direction.x) * p.y)
                                    .as_vec2()
                                    .to_array()
                            })
                            .collect()
                    })
                    .collect();
                rejects +=
                    compare_concavity_roof(&rings, shape, eave, 5.0, 0.7, &frame(level), false).0;
            }
        }
        for shape in [
            RoofShape::Gabled,
            RoofShape::Hipped,
            RoofShape::Pyramidal,
            RoofShape::Gambrel,
            RoofShape::Skillion,
            RoofShape::Onion,
        ] {
            rejects += compare_concavity_roof(
                &vec![square(-8.0, 8.0), square(-2.0, 2.0)],
                shape,
                100.0,
                3.2,
                0.55,
                &frame(17),
                false,
            )
            .0;
        }
        rejects += compare_concavity_roof(
            &vec![square(-8.0, 8.0), square(-2.0, 2.0)],
            RoofShape::Hipped,
            100.0,
            3.2,
            0.55,
            &frame(14),
            true,
        )
        .0;
        assert!(rejects > 100);
    }

    fn early_plane_geometry<const EARLY: bool>(
        cells: Vec<[DVec2; 4]>,
        boundary: &[[DVec2; 2]],
        at: &dyn Fn(DVec2) -> f64,
        error: f64,
        bounds: Option<[f64; 2]>,
    ) -> Option<RoofGeometry> {
        let mut sampler = RoofSampler::<false, false, EARLY> {
            stats: None,
            failure: 0,
            height_bounds: bounds,
            at,
            interior_error: error,
            boundary_error: ROOF_PLANE_ERROR_M,
            cache: rustc_hash::FxHashMap::default(),
        };
        roof_geometry_sampled(cells, boundary, &mut sampler, true)
    }

    fn early_plane_sections<const EARLY: bool>(
        points: [DVec2; 3],
        known: [f64; 7],
        at: &dyn Fn(DVec2) -> f64,
        bounds: Option<[f64; 2]>,
    ) -> (Option<Vec<RoofTriangle>>, usize) {
        let calls = std::cell::Cell::new(0);
        let counted = |p| {
            calls.set(calls.get() + 1);
            at(p)
        };
        let mut sampler = RoofSampler::<false, false, EARLY> {
            stats: None,
            failure: 0,
            height_bounds: bounds,
            at: &counted,
            interior_error: ROOF_PLANE_ERROR_M,
            boundary_error: ROOF_PLANE_ERROR_M,
            cache: rustc_hash::FxHashMap::default(),
        };
        (sampler.planar_sections(points, known), calls.get())
    }

    fn assert_same_sections(a: &Option<Vec<RoofTriangle>>, b: &Option<Vec<RoofTriangle>>) {
        assert_eq!(a.is_some(), b.is_some());
        if let (Some(a), Some(b)) = (a, b) {
            assert_same_roof_geometry(
                &RoofGeometry {
                    triangles: a.clone(),
                    boundary: vec![],
                },
                &RoofGeometry {
                    triangles: b.clone(),
                    boundary: vec![],
                },
            );
        }
    }

    #[test]
    fn roof_early_plane_rejection_preserves_one_millimeter_boundary_and_saves_probes() {
        let at = |p: DVec2| (100.0 - 0.2 * p.x.abs()).clamp(80.0, 100.0);
        let mut accepted = 0;
        let mut saved = 0;
        for left in [-8.0, -6.0, -4.0, -2.0] {
            for right in [2.0, 3.0, 5.0, 7.0] {
                let points = [
                    DVec2::new(left, -3.0),
                    DVec2::new(left + 0.5, -1.0),
                    DVec2::new(right, 4.0),
                ];
                for delta in [
                    -0.001001, -0.001, -0.000999, 0.0, 0.000999, 0.001, 0.001001, 0.005,
                ] {
                    let known = concavity_known_samples(points, &at).map(|h| h + delta);
                    let (old, old_calls) =
                        early_plane_sections::<false>(points, known, &at, Some([80.0, 110.0]));
                    let (new, new_calls) =
                        early_plane_sections::<true>(points, known, &at, Some([80.0, 110.0]));
                    assert_same_sections(&old, &new);
                    assert!(new_calls <= old_calls);
                    accepted += usize::from(old.is_some());
                    saved += old_calls - new_calls;
                }
            }
        }
        assert!(accepted > 5);
        assert!(saved > 100);
    }

    #[test]
    fn roof_early_plane_rejection_does_not_test_discarded_duplicate_planes() {
        let points = [
            DVec2::new(-8.0, -3.0),
            DVec2::new(-7.5, -1.0),
            DVec2::new(4.0, 4.0),
        ];
        let origin = points.iter().copied().sum::<DVec2>() / 3.0;
        let a = (points[1] - points[0]) * 0.02;
        let b = (points[2] - points[0]) * 0.02;
        let p = points[0] * 0.48 + points[1] * 0.48 + points[2] * 0.04;
        let base = |p: DVec2| (100.0 - 0.2 * p.x.abs()).clamp(80.0, 100.0);
        let at = |q: DVec2| {
            let value = base(q);
            if [p, p + a, p + b].contains(&q) {
                value + 0.0008 * (q.x - origin.x)
            } else {
                value
            }
        };
        let known = concavity_known_samples(points, &at);
        let (old, old_calls) =
            early_plane_sections::<false>(points, known, &at, Some([80.0, 110.0]));
        let (new, new_calls) =
            early_plane_sections::<true>(points, known, &at, Some([80.0, 110.0]));
        assert!(
            old.is_some(),
            "duplicate candidate must be ignored by the reference"
        );
        assert_same_sections(&old, &new);
        assert_eq!(old_calls, new_calls);
    }

    #[test]
    fn roof_early_plane_rejection_keeps_unbounded_and_nonfinite_query_behavior() {
        let ordinary = [
            DVec2::new(-8.0, -3.0),
            DVec2::new(-7.5, -1.0),
            DVec2::new(4.0, 4.0),
        ];
        for points in [
            ordinary,
            ordinary.map(|p| p + DVec2::splat(1.0e14)),
            [DVec2::ZERO, DVec2::X, DVec2::new(2.0, 1.0e-14)],
        ] {
            for bounds in [
                None,
                Some([f64::NEG_INFINITY, f64::INFINITY]),
                Some([90.0, 110.0]),
            ] {
                for invalid in [f64::NAN, -f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
                    let first = points[0] * 0.82 + points[1] * 0.09 + points[2] * 0.09;
                    let at = |p: DVec2| if p == first { invalid } else { 100.0 };
                    let known = concavity_known_samples(points, &at);
                    let (old, old_calls) =
                        early_plane_sections::<false>(points, known, &at, bounds);
                    let (new, new_calls) = early_plane_sections::<true>(points, known, &at, bounds);
                    assert_same_sections(&old, &new);
                    assert_eq!(old_calls, new_calls);
                }
            }
        }
    }

    #[test]
    fn roof_early_plane_rejection_preserves_whole_roofs_across_profiles_and_scales() {
        let mut saved = 0;
        for (_, rings, shape) in minified_roof_cases() {
            for (translation, scale, eave, level) in [
                (DVec2::ZERO, 1.0, -400.0, 14),
                (DVec2::new(100_000.0, -70_000.0), 0.5, 10_000.0, 19),
                (DVec2::new(-93.0, 131.0), 1.1, 100.0, 17),
            ] {
                let rotation = DVec2::from_angle(0.37);
                let rings: Rings = rings
                    .iter()
                    .map(|r| {
                        r.iter()
                            .map(|p| {
                                let p = DVec2::from_array(p.map(f64::from)) * scale;
                                (translation
                                    + rotation * p.x
                                    + DVec2::new(-rotation.y, rotation.x) * p.y)
                                    .as_vec2()
                                    .to_array()
                            })
                            .collect()
                    })
                    .collect();
                saved +=
                    compare_early_plane_roof(&rings, shape, eave, 3.2, 0.55, &frame(level), false)
                        .1;
            }
        }
        for shape in [
            RoofShape::Gabled,
            RoofShape::Pyramidal,
            RoofShape::Gambrel,
            RoofShape::Skillion,
            RoofShape::Onion,
        ] {
            saved += compare_early_plane_roof(
                &vec![square(-8.0, 8.0), square(-2.0, 2.0)],
                shape,
                100.0,
                3.2,
                0.55,
                &frame(17),
                false,
            )
            .1;
        }
        saved += compare_early_plane_roof(
            &vec![square(-8.0, 8.0), square(-2.0, 2.0)],
            RoofShape::Hipped,
            100.0,
            3.2,
            0.55,
            &frame(14),
            true,
        )
        .1;
        assert!(saved > 100);
    }

    fn compare_early_plane_roof(
        rings: &Rings,
        shape: RoofShape,
        eave: f64,
        rise: f64,
        pitch: f32,
        test_frame: &ChunkFrame,
        tiered: bool,
    ) -> (usize, usize) {
        let mut roof = RoofField::new(rings, Some(shape), eave, eave + rise, pitch);
        roof.tiered = tiered;
        let source = Footprint::new(vec![rings], vec![]).unwrap();
        let boundary = source.boundary();
        let rotated: Rings = rings
            .iter()
            .map(|r| {
                r.iter()
                    .map(|p| {
                        roof.rotate(DVec2::from_array(p.map(f64::from)))
                            .as_vec2()
                            .to_array()
                    })
                    .collect()
            })
            .collect();
        let domain = Footprint::new(vec![&rotated], vec![]).unwrap();
        let cells: Vec<_> = domain
            .roof_cells(3.0)
            .or_else(|| domain.roof_cells(4096.0))
            .unwrap()
            .into_iter()
            .map(|q| q.map(|p| roof.unrotate(p)))
            .collect();
        let calls = std::cell::Cell::new(0);
        let at = |p| {
            calls.set(calls.get() + 1);
            roof.height(p)
        };
        let error = interior_roof_error(test_frame, rise, Some(shape));
        let old = early_plane_geometry::<false>(
            cells.clone(),
            &boundary,
            &at,
            error,
            roof.height_bounds(),
        )
        .unwrap();
        let old_calls = calls.replace(0);
        let new = early_plane_geometry::<true>(cells, &boundary, &at, error, roof.height_bounds())
            .unwrap();
        assert_same_roof_geometry(&old, &new);
        assert!(calls.get() <= old_calls);
        (old_calls, old_calls - calls.get())
    }

    #[test]
    #[ignore = "paired source-roof benchmark; requires VEARTH_ROOF_CONCAVITY_FIXTURE"]
    fn roof_concavity_actual_source_benchmark() {
        benchmark_roof_source::<false>();
    }

    #[test]
    #[ignore = "paired source-roof benchmark; requires VEARTH_ROOF_CONCAVITY_FIXTURE"]
    fn roof_early_plane_actual_source_benchmark() {
        benchmark_roof_source::<true>();
    }

    struct RoofSourceCase {
        name: String,
        cell: vearth_core::cell::CellId,
        expected_extent: Option<[f64; 2]>,
        shape: RoofShape,
        rise: f64,
        pitch: f32,
        tiered: bool,
        rings: Rings,
    }
    fn roof_source_cases() -> Vec<RoofSourceCase> {
        let path =
            vearth_platform::env::var("VEARTH_ROOF_CONCAVITY_FIXTURE").expect("fixture path");
        roof_source_cases_from_path(&path)
    }
    fn roof_source_cases_from_path(path: &str) -> Vec<RoofSourceCase> {
        use vearth_core::cell::CellId;
        let text = std::fs::read_to_string(path).unwrap();
        let mut cases = Vec::<RoofSourceCase>::new();
        for line in text
            .lines()
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
        {
            let w: Vec<_> = line.split_whitespace().collect();
            if w[0] == "roof" {
                let shape = match w[7].to_ascii_lowercase().as_str() {
                    "gabled" => RoofShape::Gabled,
                    "hipped" => RoofShape::Hipped,
                    "flat" => RoofShape::Flat,
                    "mansard" => RoofShape::Mansard,
                    "gambrel" => RoofShape::Gambrel,
                    "pyramidal" => RoofShape::Pyramidal,
                    "skillion" => RoofShape::Skillion,
                    "dome" => RoofShape::Dome,
                    "onion" => RoofShape::Onion,
                    _ => panic!("unknown shape"),
                };
                cases.push(RoofSourceCase {
                    name: w[..8].join(" "),
                    cell: CellId {
                        face: vearth_core::cubesphere::Face(w[1].parse().unwrap()),
                        level: w[2].parse().unwrap(),
                        i: w[3].parse().unwrap(),
                        j: w[4].parse().unwrap(),
                    },
                    expected_extent: None,
                    shape,
                    rise: w[8].parse().unwrap(),
                    pitch: w[9].parse().unwrap(),
                    tiered: w[10] == "1",
                    rings: vec![],
                });
            } else if w[0] == "extent" {
                cases.last_mut().unwrap().expected_extent =
                    Some([w[1].parse().unwrap(), w[2].parse().unwrap()]);
            } else if w[0] == "ring" {
                cases.last_mut().unwrap().rings.push(vec![]);
            } else {
                cases
                    .last_mut()
                    .unwrap()
                    .rings
                    .last_mut()
                    .unwrap()
                    .push([w[0].parse().unwrap(), w[1].parse().unwrap()]);
            }
        }
        cases
    }

    fn benchmark_roof_source<const EARLY_ONLY: bool>() {
        use vearth_core::cell::ChunkAddr;
        let cases = roof_source_cases();
        let mut totals = [0u128; 2];
        for case in cases {
            let source_frame = ChunkFrame::of(ChunkAddr {
                cell: case.cell,
                k: 0,
            });
            let (rejects, old_calls, new_calls) = if EARLY_ONLY {
                let (old, saved) = compare_early_plane_roof(
                    &case.rings,
                    case.shape,
                    100.0,
                    case.rise,
                    case.pitch,
                    &source_frame,
                    case.tiered,
                );
                (0, old, old - saved)
            } else {
                compare_concavity_roof(
                    &case.rings,
                    case.shape,
                    100.0,
                    case.rise,
                    case.pitch,
                    &source_frame,
                    case.tiered,
                )
            };
            let mut roof = RoofField::new(
                &case.rings,
                Some(case.shape),
                100.0,
                100.0 + case.rise,
                case.pitch,
            );
            roof.tiered = case.tiered;
            if let Some(extent) = case.expected_extent {
                assert_eq!(
                    (roof.bounds[1] - roof.bounds[0])
                        .to_array()
                        .map(f64::to_bits),
                    extent.map(f64::to_bits)
                );
            }
            let domain = Footprint::new(vec![&case.rings], vec![]).unwrap();
            let boundary = domain.boundary();
            let rotated: Rings = case
                .rings
                .iter()
                .map(|r| {
                    r.iter()
                        .map(|p| {
                            roof.rotate(DVec2::from_array(p.map(f64::from)))
                                .as_vec2()
                                .to_array()
                        })
                        .collect()
                })
                .collect();
            let domain = Footprint::new(vec![&rotated], vec![]).unwrap();
            let cells: Vec<_> = domain
                .roof_cells(3.0)
                .or_else(|| domain.roof_cells(4096.0))
                .unwrap()
                .into_iter()
                .map(|q| q.map(|p| roof.unrotate(p)))
                .collect();
            let error = interior_roof_error(&source_frame, case.rise, Some(case.shape));
            let mut times = [vec![], vec![]];
            for round in 0..5 {
                for mode in [round % 2, 1 - round % 2] {
                    let start = vearth_platform::time::Instant::now();
                    std::hint::black_box(if EARLY_ONLY {
                        if mode == 0 {
                            early_plane_geometry::<false>(
                                cells.clone(),
                                &boundary,
                                &|p| roof.height(p),
                                error,
                                roof.height_bounds(),
                            )
                            .unwrap()
                        } else {
                            early_plane_geometry::<true>(
                                cells.clone(),
                                &boundary,
                                &|p| roof.height(p),
                                error,
                                roof.height_bounds(),
                            )
                            .unwrap()
                        }
                    } else {
                        roof_geometry_profiled(
                            cells.clone(),
                            &boundary,
                            &|p| roof.height(p),
                            true,
                            error,
                            if mode == 0 {
                                None
                            } else {
                                roof.height_bounds()
                            },
                            None,
                        )
                        .unwrap()
                    });
                    times[mode].push(start.elapsed().as_nanos());
                }
            }
            for mode in 0..2 {
                times[mode].sort();
                totals[mode] += times[mode][2];
            }
            eprintln!(
                "{}: source rings={}, controlled eave100m, exact mesh; rejects={rejects}, calls={old_calls}->{new_calls}, median_ms={:.3}->{:.3}",
                case.name,
                case.rings.len(),
                times[0][2] as f64 / 1e6,
                times[1][2] as f64 / 1e6
            );
        }
        eprintln!(
            "paired source-roof aggregate medians ms {:.3}->{:.3}",
            totals[0] as f64 / 1e6,
            totals[1] as f64 / 1e6
        );
    }

    #[test]
    fn roof_diagnostics_preserve_query_counts_meshes_and_fallback() {
        for (_, rings, shape) in minified_roof_cases() {
            let domain = Footprint::new(vec![&rings], Vec::new()).unwrap();
            let boundary = domain.boundary();
            let cells = domain.roof_cells(3.0).unwrap();
            let roof = RoofField::new(&rings, Some(shape), 100.0, 105.0, 0.7);
            for level in [14, 19] {
                for refine in [true, false] {
                    let calls = std::cell::Cell::new(0);
                    let at = |p| {
                        calls.set(calls.get() + 1);
                        roof.height(p)
                    };
                    let error = interior_roof_error(&frame(level), 5.0, Some(shape));
                    let expected =
                        roof_geometry_with_error(cells.clone(), &boundary, &at, refine, error)
                            .unwrap();
                    let expected_calls = calls.replace(0);
                    let mut stats = crate::contracts::RoofWorkCounters::default();
                    let actual = roof_geometry_profiled(
                        cells.clone(),
                        &boundary,
                        &at,
                        refine,
                        error,
                        None,
                        Some(&mut stats),
                    )
                    .unwrap();
                    assert_same_roof_geometry(&actual, &expected);
                    assert_eq!(calls.get(), expected_calls);
                    assert_eq!(stats.initial_cells, cells.len() as u64);
                    assert_eq!(
                        stats.plane_attempts,
                        stats.plane_successes + stats.plane_rejections.iter().sum::<u64>()
                    );
                    assert_eq!(stats.refinement_visits.iter().sum::<u64>() > 0, refine);
                }
            }
        }
        let rings = vec![square(-12.0, 12.0)];
        let domain = Footprint::new(vec![&rings], Vec::new()).unwrap();
        let cells = domain.roof_cells(4.0).unwrap();
        let boundary = domain.boundary();
        let at = |p: DVec2| {
            if p.x + p.y * 0.037 < 0.123 {
                100.0
            } else {
                105.0
            }
        };
        let mut stats = crate::contracts::RoofWorkCounters::default();
        let actual = roof_geometry_profiled(
            cells.clone(),
            &boundary,
            &at,
            true,
            ROOF_PLANE_ERROR_M,
            None,
            Some(&mut stats),
        )
        .unwrap();
        let expected = roof_geometry(cells.clone(), &boundary, &at, false).unwrap();
        assert_same_roof_geometry(&actual, &expected);
        assert_eq!(stats.initial_cells, cells.len() as u64);
        assert_eq!(stats.fallback_depth, 1);
        assert_eq!(stats.fallback_budget + stats.fallback_invalid, 0);
        assert!(stats.discarded_triangles > 0);
        assert!(stats.refinement_visits[9] > 0);
    }

    #[test]
    fn roof_diagnostics_distinguish_invalid_samples_from_budget_exhaustion() {
        let cells = vec![[DVec2::ZERO, DVec2::X, DVec2::ONE, DVec2::Y]];
        let mut invalid = crate::contracts::RoofWorkCounters::default();
        assert!(
            roof_geometry_profiled(
                cells,
                &[],
                &|_| f64::NAN,
                true,
                ROOF_PLANE_ERROR_M,
                None,
                Some(&mut invalid)
            )
            .is_none()
        );
        assert_eq!(invalid.fallback_invalid, 1);
        assert_eq!(invalid.fallback_depth + invalid.fallback_budget, 0);
        let mut stats = crate::contracts::RoofWorkCounters::default();
        let mut sampler = RoofSampler::<false, true> {
            stats: Some(&mut stats),
            failure: 0,
            height_bounds: None,
            at: &|_| 0.0,
            interior_error: ROOF_PLANE_ERROR_M,
            boundary_error: ROOF_PLANE_ERROR_M,
            cache: rustc_hash::FxHashMap::default(),
        };
        let triangle = RoofTriangle {
            points: [DVec2::ZERO, DVec2::X, DVec2::Y],
            heights: [0.0; 3],
        };
        let mut output = vec![triangle; MAX_ROOF_TRIANGLES];
        assert!(
            sampler
                .triangle(triangle.points, true, 0, &mut output, 0)
                .is_none()
        );
        assert_eq!(sampler.failure, 2);
        assert_eq!(output.len(), MAX_ROOF_TRIANGLES);
    }

    #[test]
    fn minified_roofs_preserve_domains_boundaries_and_bounded_surface_error() {
        for (name, rings, shape) in minified_roof_cases() {
            let domain = Footprint::new(vec![&rings], Vec::new()).unwrap();
            let boundary = domain.boundary();
            let cells = domain.roof_cells(3.0).unwrap();
            let roof = RoofField::new(&rings, Some(shape), 100.0, 105.0, 0.7);
            let samples = std::cell::Cell::new(0usize);
            let at = |p| {
                samples.set(samples.get() + 1);
                roof.height(p)
            };
            let fine = roof_geometry(cells.clone(), &boundary, &at, true).unwrap();
            let fine_samples = samples.replace(0);
            let error = interior_roof_error(&frame(15), 5.0, Some(shape));
            let coarse =
                roof_geometry_with_error(cells.clone(), &boundary, &at, true, error).unwrap();
            let coarse_samples = samples.get();
            let near = roof_geometry_with_error(
                cells,
                &boundary,
                &|p| roof.height(p),
                true,
                interior_roof_error(&frame(20), 5.0, Some(RoofShape::Hipped)),
            )
            .unwrap();
            assert_same_roof_geometry(&near, &fine);
            if matches!(shape, RoofShape::Dome | RoofShape::Onion) {
                assert_same_roof_geometry(&coarse, &fine);
                assert_eq!(coarse_samples, fine_samples);
                continue;
            }
            let area = |mesh: &RoofGeometry| {
                mesh.triangles
                    .iter()
                    .map(|tri| {
                        (tri.points[1] - tri.points[0])
                            .perp_dot(tri.points[2] - tri.points[0])
                            .abs()
                    })
                    .sum::<f64>()
            };
            assert!((area(&coarse) - area(&fine)).abs() < area(&fine) * 1.0e-8);
            let mut maximum_error = 0.0_f64;
            for tri in &coarse.triangles {
                let center = tri.points.iter().copied().sum::<DVec2>() / 3.0;
                assert!(
                    domain.contains(center),
                    "{name}: footprint/courtyard changed"
                );
                for a in 0..=12 {
                    for b in 0..=12 - a {
                        let weights =
                            [a as f64 / 12.0, b as f64 / 12.0, (12 - a - b) as f64 / 12.0];
                        let point = (0..3).map(|i| tri.points[i] * weights[i]).sum::<DVec2>();
                        let height = (0..3).map(|i| tri.heights[i] * weights[i]).sum::<f64>();
                        maximum_error = maximum_error.max((height - roof.height(point)).abs());
                    }
                }
            }
            assert!(
                maximum_error <= error * 2.0 + 1.0e-5,
                "{name}: {maximum_error} exceeds the conservative interior envelope {}",
                error * 2.0
            );
            let original_length: f64 = boundary.iter().map(|edge| edge[0].distance(edge[1])).sum();
            let sampled_length: f64 = coarse
                .boundary
                .iter()
                .map(|edge| edge.points[0].distance(edge.points[1]))
                .sum();
            assert!((original_length - sampled_length).abs() < 0.001);
            for edge in &coarse.boundary {
                assert!(
                    (roof.height((edge.points[0] + edge.points[1]) * 0.5)
                        - (edge.heights[0] + edge.heights[1]) * 0.5)
                        .abs()
                        <= ROOF_PLANE_ERROR_M + 1.0e-5,
                    "{name}: boundary tolerance changed"
                );
                for i in 0..2 {
                    assert!(
                        coarse
                            .triangles
                            .iter()
                            .any(|tri| (0..3).any(|j| tri.points[j] == edge.points[i]
                                && tri.heights[j] == edge.heights[i])),
                        "roof/closure/eave endpoint separated"
                    );
                }
            }
            assert!(coarse.triangles.len() <= fine.triangles.len());
            assert!(
                coarse_samples < fine_samples,
                "{name}: sample work did not fall"
            );
            eprintln!(
                "minified roof {name}: tris {} -> {}, field samples {fine_samples} -> {coarse_samples}, maximum dense error {maximum_error:.5}m",
                fine.triangles.len(),
                coarse.triangles.len()
            );
        }
    }

    #[test]
    fn courtyard_roof_crests_refine_only_where_a_fan_would_cross_roof_planes() {
        let rings = vec![
            vec![[-40.0, -28.0], [40.0, -28.0], [40.0, 28.0], [-40.0, 28.0]],
            vec![[-21.0, -10.0], [21.0, -10.0], [21.0, 10.0], [-21.0, 10.0]],
        ];
        let domain = Footprint::new(vec![&rings], Vec::new()).unwrap();
        let roof = RoofField::new(&rings, Some(RoofShape::Hipped), 8.0, 11.0, 0.55);
        let cells = domain.roof_cells(3.0).unwrap();
        let boundary = domain.boundary();
        let at = |p| roof.height(p);
        let original = roof_geometry(cells.clone(), &boundary, &at, false).unwrap();
        let refined = roof_geometry(cells, &boundary, &at, true).unwrap();
        let maximum_error = |mesh: &RoofGeometry| {
            mesh.triangles
                .iter()
                .map(|tri| {
                    let center = tri.points.iter().copied().sum::<DVec2>() / 3.0;
                    let mut error =
                        (roof.height(center) - tri.heights.iter().sum::<f64>() / 3.0).abs();
                    for i in 0..3 {
                        let next = (i + 1) % 3;
                        error = error.max(
                            (roof.height((tri.points[i] + tri.points[next]) * 0.5)
                                - (tri.heights[i] + tri.heights[next]) * 0.5)
                                .abs(),
                        );
                    }
                    error
                })
                .max_by(f64::total_cmp)
                .unwrap()
        };
        assert!(
            maximum_error(&original) > 0.10,
            "fixture must expose the visible fan-shaped crest fault"
        );
        assert!(maximum_error(&refined) <= ROOF_PLANE_ERROR_M + 1.0e-5);
        assert!(refined.triangles.len() < MAX_ROOF_TRIANGLES);
        let plane = |p: DVec2| 100.0 + p.x * 0.2 + p.y * 0.1;
        let cells = domain.roof_cells(3.0).unwrap();
        let flat = roof_geometry(cells.clone(), &boundary, &plane, false).unwrap();
        let adaptive = roof_geometry(cells, &boundary, &plane, true).unwrap();
        assert_eq!(flat.triangles.len(), adaptive.triangles.len());
    }

    #[test]
    fn gable_closures_and_eaves_share_the_exact_sampled_roof_boundary() {
        let angle = 0.37f64;
        let rotate = |p: [f32; 2]| {
            [
                (f64::from(p[0]) * angle.cos() - f64::from(p[1]) * angle.sin()) as f32,
                (f64::from(p[0]) * angle.sin() + f64::from(p[1]) * angle.cos()) as f32,
            ]
        };
        let rings = vec![
            vec![
                [-15.85, -5.65],
                [15.85, -5.65],
                [15.85, 5.65],
                [-15.85, 5.65],
            ]
            .into_iter()
            .map(rotate)
            .collect(),
        ];
        let domain = Footprint::new(vec![&rings], Vec::new()).unwrap();
        let roof = RoofField::new(&rings, Some(RoofShape::Gabled), 8.0, 11.0, 0.55);
        let geometry = roof_geometry(
            domain.roof_cells(3.0).unwrap(),
            &domain.boundary(),
            &|p| roof.height(p),
            true,
        )
        .unwrap();
        assert!(!geometry.boundary.is_empty());
        for edge in &geometry.boundary {
            for i in 0..2 {
                assert!(geometry.triangles.iter().any(|tri| {
                    (0..3).any(|j| {
                        tri.points[j] == edge.points[i] && tri.heights[j] == edge.heights[i]
                    })
                }));
            }
            assert!(
                (roof.height((edge.points[0] + edge.points[1]) * 0.5)
                    - (edge.heights[0] + edge.heights[1]) * 0.5)
                    .abs()
                    <= ROOF_PLANE_ERROR_M + 1.0e-5
            );
        }
        let outline = domain
            .boundary()
            .iter()
            .map(|edge| edge[0].distance(edge[1]))
            .sum::<f64>();
        let sampled = geometry
            .boundary
            .iter()
            .map(|edge| edge.points[0].distance(edge.points[1]))
            .sum::<f64>();
        assert!(
            (outline - sampled).abs() < 0.001,
            "roof boundary must remain closed"
        );
    }
    #[test]
    fn multi_courtyard_roof_retains_a_bounded_continuous_source_envelope() {
        // Irregular narrow wings and nearly collinear courtyard edges exercise
        // the small scanline triangles that must not magnify sampling noise.
        let rings: Rings = vec![
            vec![
                [2.3962831, -43.1796300],
                [-0.1152161, -27.4826720],
                [-2.6267154, -22.8782370],
                [-5.9753810, -15.1344250],
                [-12.0448380, -3.6233695],
                [-18.9514600, -6.5534544],
                [-21.4629600, 2.8646708],
                [-9.9519210, 6.2133340],
                [-13.0912950, 14.1664020],
                [15.5816550, 26.9331470],
                [16.0002370, 26.0959850],
                [63.0908470, 43.6763840],
                [69.5788900, 25.6774040],
                [81.7178040, -27.4826720],
                [39.8594800, -34.8079150],
            ],
            vec![
                [59.7421840, 2.4460876],
                [52.6262660, -0.2747027],
                [23.1161520, -10.1114160],
                [21.4418180, -5.0884120],
                [10.9772390, -9.9021240],
                [12.8608640, -14.0879640],
                [7.6285734, -15.9715930],
                [8.6750320, -18.4830990],
                [37.1386900, -8.6463730],
                [62.6722640, -0.9025776],
                [54.9284740, 30.4910890],
                [49.4868930, 28.6074730],
                [50.1147700, 26.3052750],
                [50.7426450, 26.5145660],
                [58.2771400, 7.0504994],
            ],
            vec![
                [45.7196460, 14.5849840],
                [44.2546040, 18.1429330],
                [8.8843230, 4.3297110],
                [6.5821150, 9.9805780],
                [34.4179000, 21.0730060],
                [42.1616900, 23.3752040],
                [41.1152300, 25.4681130],
                [5.9542403, 11.6549080],
                [5.3263655, 13.3292370],
                [-0.9523826, 10.6084520],
                [3.8613243, -0.6932859],
                [18.0931530, 5.5854597],
                [18.5117360, 4.3297110],
            ],
            vec![
                [25.2090680, -24.7618700],
                [37.1386900, -22.0410690],
                [36.3015200, -17.8552230],
                [25.6276510, -20.5760210],
                [25.2090680, -19.7388530],
                [18.0931530, -21.6224840],
                [18.7210270, -23.9247000],
                [24.5811940, -22.2503600],
            ],
            vec![
                [57.2306820, -22.6689450],
                [63.0908470, -21.6224840],
                [63.0908470, -18.2738080],
                [66.0209300, -18.2738080],
                [66.2302250, -21.2039000],
                [70.6253400, -20.5760210],
                [69.7881800, -17.2273460],
                [68.5324300, -17.6459310],
                [68.1138460, -16.1808850],
                [56.3935170, -19.3202690],
            ],
            vec![
                [-9.1147550, 5.1668770],
                [-8.0682970, 2.6553793],
                [-4.3010483, 4.1204195],
                [-5.5567980, 6.6319165],
            ],
        ];
        let roof = RoofField::new(&rings, Some(RoofShape::Hipped), 12.345, 13.545, 0.55);
        let domain = Footprint::new(vec![&rings], Vec::new()).unwrap();
        let rotated_rings: Rings = rings
            .iter()
            .map(|ring| {
                ring.iter()
                    .map(|p| {
                        roof.rotate(DVec2::from_array(p.map(f64::from)))
                            .as_vec2()
                            .to_array()
                    })
                    .collect()
            })
            .collect();
        let rotated = Footprint::new(vec![&rotated_rings], Vec::new()).unwrap();
        let cells = rotated
            .roof_cells(3.0)
            .unwrap()
            .into_iter()
            .map(|c| c.map(|p| roof.unrotate(p)));
        let geometry = roof_geometry(cells, &domain.boundary(), &|p| roof.height(p), true)
            .expect("valid narrow courtyard wings must retain their source mesh");
        assert!(geometry.triangles.len() < MAX_ROOF_TRIANGLES);
        for triangle in &geometry.triangles {
            let p = triangle.points.iter().copied().sum::<DVec2>() / 3.0;
            assert!(
                (roof.height(p) - triangle.heights.iter().sum::<f64>() / 3.0).abs()
                    <= ROOF_PLANE_ERROR_M + 1.0e-6
            );
        }
        let outline: f64 = domain
            .boundary()
            .iter()
            .map(|edge| edge[0].distance(edge[1]))
            .sum();
        let sampled: f64 = geometry
            .boundary
            .iter()
            .map(|edge| edge.points[0].distance(edge.points[1]))
            .sum();
        assert!((outline - sampled).abs() < 0.001);
    }

    #[test]
    fn skewed_gable_height_is_continuous_at_the_source_boundary() {
        let rings = vec![vec![
            [-5.7479477, -16.627478],
            [19.15785, -12.232327],
            [23.134407, -34.417408],
            [-1.7713917, -39.021866],
        ]];
        let roof = RoofField::new(&rings, Some(RoofShape::Gabled), 26.345, 30.485, 0.55);
        let domain = Footprint::new(vec![&rings], Vec::new()).unwrap();
        for edge in domain.boundary() {
            let tangent = (edge[1] - edge[0]).normalize();
            let normal = DVec2::new(-tangent.y, tangent.x);
            for step in 0..=128 {
                let point = edge[0].lerp(edge[1], step as f64 / 128.0);
                let center = roof.height(point);
                for delta in [-0.001, 0.001] {
                    assert!((roof.height(point + normal * delta) - center).abs() <= 0.00056);
                }
            }
        }
        assert!(
            roof_geometry(
                domain.roof_cells(3.0).unwrap(),
                &domain.boundary(),
                &|p| roof.height(p),
                true
            )
            .is_some()
        );
    }

    #[test]
    fn roof_refinement_limit_keeps_the_source_grid_field_and_closed_boundary() {
        let rings = vec![square(-4.0, 4.0), square(-1.0, 1.0)];
        let domain = Footprint::new(vec![&rings], Vec::new()).unwrap();
        let cells = domain.roof_cells(3.0).unwrap();
        let boundary = domain.boundary();
        let height = |p: DVec2| if p.x > 0.123456 { 9.0 } else { 8.0 };
        let base = roof_geometry(cells.clone(), &boundary, &height, false).unwrap();
        let bounded = roof_geometry(cells, &boundary, &height, true)
            .expect("refinement failure must preserve the complete source envelope");
        assert_eq!(bounded.triangles.len(), base.triangles.len());
        assert_eq!(bounded.boundary.len(), base.boundary.len());
        for (actual, expected) in bounded.triangles.iter().zip(&base.triangles) {
            assert_eq!(actual.points, expected.points);
            assert_eq!(actual.heights, expected.heights);
        }
        for (actual, expected) in bounded.boundary.iter().zip(&base.boundary) {
            assert_eq!(actual.points, expected.points);
            assert_eq!(actual.heights, expected.heights);
        }
    }

    fn square(lo: f32, hi: f32) -> Vec<[f32; 2]> {
        vec![[lo, lo], [hi, lo], [hi, hi], [lo, hi]]
    }

    fn area(cells: &[[DVec2; 4]]) -> f64 {
        cells
            .iter()
            .map(|cell| {
                (0..4)
                    .map(|i| cell[i].perp_dot(cell[(i + 1) % 4]))
                    .sum::<f64>()
                    .abs()
                    * 0.5
            })
            .sum()
    }

    #[test]
    fn polygon_roofs_keep_holes_concavity_and_disconnected_source_parts() {
        let mut ring = square(0.0, 20.0);
        ring.reverse();
        let rings = vec![ring, square(5.0, 15.0), square(30.0, 35.0)];
        let domain = Footprint::new(vec![&rings], Vec::new()).unwrap();
        let cells = domain.roof_cells(3.0).unwrap();
        assert!((area(&cells) - 325.0).abs() < 1.0e-6);
        assert!(
            cells
                .iter()
                .all(|cell| domain.contains(cell.iter().copied().sum::<DVec2>() / 4.0))
        );
        let boundary = domain.boundary();
        assert!((boundary.iter().map(|[a, b]| a.distance(*b)).sum::<f64>() - 140.0).abs() < 1.0e-6);
    }

    fn unscoped_footprint_reference<'a>(
        required: Vec<&'a Rings>,
        excluded: Vec<&'a Rings>,
    ) -> Footprint<'a> {
        let mut footprint =
            Footprint::with_limits(required, excluded, usize::MAX, 100_000).unwrap();
        footprint.edges = footprint
            .required
            .iter()
            .chain(&footprint.excluded)
            .flat_map(|rings| rings.iter())
            .flat_map(|ring| {
                (0..ring.len()).filter_map(|i| {
                    let a = DVec2::from_array(ring[i].map(f64::from));
                    let b = DVec2::from_array(ring[(i + 1) % ring.len()].map(f64::from));
                    (a.distance_squared(b) > GEOMETRY_EPSILON * GEOMETRY_EPSILON).then_some([a, b])
                })
            })
            .collect();
        footprint.arrangement_bounds = footprint.edges.iter().flatten().fold(
            [DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY)],
            |[lo, hi], &p| [lo.min(p), hi.max(p)],
        );
        footprint
    }

    fn arrangement_circle(radius: f64, count: usize) -> Vec<[f32; 2]> {
        (0..count)
            .map(|i| {
                let angle = i as f64 * std::f64::consts::TAU / count as f64;
                [(radius * angle.cos()) as f32, (radius * angle.sin()) as f32]
            })
            .collect()
    }

    #[test]
    fn local_arrangements_keep_full_membership_for_remote_holes_and_enclosing_exclusions() {
        let required = vec![square(-2.0, 2.0)];
        let outside = vec![arrangement_circle(100.0, 600)];
        let hole = vec![
            arrangement_circle(100.0, 600),
            arrangement_circle(50.0, 600),
        ];
        for (excluded, expected_area) in [(&outside, 0.0), (&hole, 16.0)] {
            let reference = unscoped_footprint_reference(vec![&required], vec![excluded]);
            assert!(reference.edges.len() > MAX_FOOTPRINT_EDGES);
            let actual = Footprint::new(vec![&required], vec![excluded]).unwrap();
            assert_eq!(actual.edges.len(), 4);
            let cells = actual.roof_cells(1.0).unwrap();
            assert!((area(&cells) - expected_area).abs() < 1.0e-8);
            assert!((area(&reference.roof_cells(4096.0).unwrap()) - expected_area).abs() < 1.0e-8);
            assert_eq!(actual.boundary(), reference.boundary());
            for y in -8..=8 {
                for x in -8..=8 {
                    let point = DVec2::new(x as f64 * 0.37, y as f64 * 0.37);
                    assert_eq!(actual.contains(point), reference.contains(point));
                }
            }
        }
    }

    #[test]
    fn local_arrangements_preserve_crossing_segments_and_ignore_remote_row_refinement() {
        let required = vec![square(-2.0, 2.0)];
        let exclusion = vec![vec![
            [-0.5, -10000.0],
            [0.5, -10000.0],
            [0.5, 10000.0],
            [-0.5, 10000.0],
        ]];
        let actual = Footprint::with_limits(vec![&required], vec![&exclusion], 6, 32).unwrap();
        assert_eq!(actual.edges.len(), 6);
        assert!(actual.edges.iter().any(|edge| edge[0].y.abs() == 10000.0));
        let cells = actual.roof_cells(1.0).unwrap();
        assert!((area(&cells) - 12.0).abs() < 1.0e-8);
        let mut reference = unscoped_footprint_reference(vec![&required], vec![&exclusion]);
        assert_eq!(actual.boundary(), reference.boundary());
        assert!((area(&reference.roof_cells(4096.0).unwrap()) - 12.0).abs() < 1.0e-8);
        reference.max_cells = 32;
        assert!(reference.roof_cells(1.0).is_none());
        for cell in &cells {
            let center = cell.iter().copied().sum::<DVec2>() / 4.0;
            assert!(reference.contains(center));
            assert!(
                cell.iter()
                    .all(|point| point.abs().cmple(DVec2::splat(2.0)).all())
            );
        }
    }

    #[test]
    fn disjoint_required_domains_are_known_empty_without_synthetic_boundary_edges() {
        let required = vec![square(-2.0, 2.0)];
        let disjoint = vec![vec![[8.0, -2.0], [12.0, -2.0], [12.0, 2.0], [8.0, 2.0]]];
        let actual = Footprint::with_limits(vec![&required, &disjoint], vec![], 0, 1).unwrap();
        assert!(actual.edges.is_empty());
        assert!(actual.roof_cells(1.0).unwrap().is_empty());
        assert!(actual.boundary().is_empty());
        assert!(!actual.contains(DVec2::ZERO));
        assert!(!actual.contains(DVec2::new(10.0, 0.0)));

        let source = vec![arrangement_circle(100.0, 512)];
        assert_eq!(
            Footprint::with_limits(vec![&source], vec![], 512, 8192)
                .unwrap()
                .edges
                .len(),
            512
        );
        let mut reversed = source.clone();
        reversed[0].reverse();
        let duplicate = Footprint::with_limits(vec![&source], vec![&reversed], 512, 8192).unwrap();
        assert_eq!(duplicate.edges.len(), 512);
        assert!(duplicate.roof_cells(4096.0).unwrap().is_empty());
        assert!(duplicate.boundary().is_empty());
    }

    #[test]
    #[ignore = "local relevant-arrangement generation benchmark"]
    fn relevant_arrangement_generation_benchmark() {
        let required = vec![square(-2.0, 2.0)];
        let excluded = vec![
            arrangement_circle(100.0, 600),
            arrangement_circle(50.0, 600),
        ];
        let frame = frame(17);
        let (lat, lon) = frame.column_latlon(0.5, 0.5);
        let projection = Projection {
            frame: &frame,
            lat,
            lon,
        };
        let run = |scoped| {
            let domain = if scoped {
                Footprint::new(vec![&required], vec![&excluded]).unwrap()
            } else {
                unscoped_footprint_reference(vec![&required], vec![&excluded])
            };
            let roof = roof_geometry(
                domain.roof_cells(3.0).unwrap(),
                &domain.boundary(),
                &|_| 112.0,
                false,
            )
            .unwrap();
            let count = roof.triangles.len();
            let mut mesh = MeshBuffers::default();
            emit_roof(
                &projection,
                &mut mesh,
                &roof,
                SurfaceAppearance {
                    material: crate::material::TILE_ROOF,
                    tint: [100; 3],
                },
            );
            std::hint::black_box(mesh);
            count
        };
        let triangles = [run(false), run(true)];
        let mut times = [Vec::new(), Vec::new()];
        for round in 0..5 {
            for mode in [round % 2, 1 - round % 2] {
                let start = vearth_platform::time::Instant::now();
                for _ in 0..4 {
                    std::hint::black_box(run(mode == 1));
                }
                times[mode].push(start.elapsed().as_secs_f64() * 1000.0 / 4.0);
            }
        }
        for values in &mut times {
            values.sort_by(f64::total_cmp);
        }
        eprintln!(
            "relevant arrangement: {:.3}->{:.3}ms ({:.2}x), 1204->4 edges, {}->{} roof triangles; original512-edge admission rejects this source combination",
            times[0][2],
            times[1][2],
            times[0][2] / times[1][2],
            triangles[0],
            triangles[1]
        );
    }

    #[test]
    fn polygon_difference_and_intersection_preserve_crossing_edges() {
        let base = vec![square(0.0, 20.0)];
        let diagonal = vec![vec![
            [-10.0, 10.0],
            [10.0, -10.0],
            [30.0, 10.0],
            [10.0, 30.0],
        ]];
        let difference = Footprint::new(vec![&base], vec![&diagonal]).unwrap();
        assert!(difference.roof_cells(3.0).unwrap().is_empty());
        let cut = vec![vec![[0.0, -10.0], [30.0, 20.0], [0.0, 20.0]]];
        let inside = Footprint::new(vec![&base, &cut], Vec::new()).unwrap();
        let outside = Footprint::new(vec![&base], vec![&cut]).unwrap();
        assert!(
            (area(&inside.roof_cells(2.0).unwrap()) + area(&outside.roof_cells(2.0).unwrap())
                - 400.0)
                .abs()
                < 1.0e-6
        );
        assert!(!outside.boundary().is_empty());
    }

    fn cartesian_roof_boundary(
        triangles: &[RoofTriangle],
        boundary: &[[DVec2; 2]],
    ) -> Vec<RoofBoundary> {
        let mut result = Vec::new();
        for edge in boundary {
            let direction = edge[1] - edge[0];
            let length2 = direction.length_squared();
            let mut cuts = Vec::new();
            for triangle in triangles {
                for i in 0..3 {
                    let point = triangle.points[i];
                    let t = ((point - edge[0]).dot(direction) / length2).clamp(0.0, 1.0);
                    if point.distance_squared(edge[0] + direction * t) < 1.0e-10 {
                        cuts.push((t, point, triangle.heights[i]));
                    }
                }
            }
            cuts.sort_by(|a, b| a.0.total_cmp(&b.0));
            cuts.dedup_by(|a, b| a.1.distance_squared(b.1) < 1.0e-12);
            for pair in cuts.windows(2) {
                if pair[0].1.distance_squared(pair[1].1) > 1.0e-12 {
                    result.push(RoofBoundary {
                        points: [pair[0].1, pair[1].1],
                        heights: [pair[0].2, pair[1].2],
                    });
                }
            }
        }
        result
    }

    fn assert_same_roof_boundary(actual: &[RoofBoundary], expected: &[RoofBoundary]) {
        let bits = |edges: &[RoofBoundary]| {
            edges
                .iter()
                .map(|edge| {
                    (
                        edge.points.map(|point| point.to_array().map(f64::to_bits)),
                        edge.heights.map(f64::to_bits),
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(bits(actual), bits(expected));
    }

    fn check_roof_boundary_hash(
        triangles: &[RoofTriangle],
        boundary: &[[DVec2; 2]],
    ) -> (usize, usize) {
        let old = RoofBoundaryIndex::new_std(triangles);
        let new = RoofBoundaryIndex::new(triangles);
        assert_eq!(old.next_visit, new.next_visit);
        assert_eq!(old.by_axis, new.by_axis);
        assert_eq!(old.samples.len(), new.samples.len());
        assert_same_roof_boundary(&new.boundary(boundary), &old.boundary(boundary));
        for (original, sample) in old.samples.iter().zip(&new.samples) {
            assert_eq!(
                original.point.to_array().map(f64::to_bits),
                sample.point.to_array().map(f64::to_bits)
            );
            assert_eq!(original.height.to_bits(), sample.height.to_bits());
            assert_eq!(original.first_visit, sample.first_visit);
            assert_eq!(original.last_visit, sample.last_visit);
        }
        (old.samples.len(), new.samples.len())
    }

    #[test]
    fn roof_boundary_hash_preserves_visits_tolerance_first_heights_and_ties() {
        let points = [
            [[0.5, 0.5], [0.25, 0.25], [0.75, 0.75]],
            [[0., 0.], [0.5, 0.], [0.5, 3e-6]],
            [[0.5, 0.], [0.5, 3.1e-6], [1., 0.]],
            [[0.5, 3.9e-6], [1. + 0.9e-5, 0.], [1. + 1.1e-5, 0.]],
            [[0.75, 0.9999e-5], [0.75, 1.0001e-5], [-0., 0.]],
            [[0., 0.], [0., 0.], [0., 1.]],
        ];
        let mut triangles: Vec<_> = points
            .into_iter()
            .enumerate()
            .map(|(i, p)| RoofTriangle {
                points: p.map(DVec2::from_array),
                heights: [i as f64, 100. + i as f64, -(i as f64)],
            })
            .collect();
        let boundary = [
            [DVec2::ZERO, DVec2::X],
            [DVec2::X, DVec2::ZERO],
            [DVec2::ZERO, DVec2::Y],
        ];
        let (old, new) = check_roof_boundary_hash(&triangles, &boundary);
        assert_eq!(new, old);
        triangles[1].heights[0] = f64::from_bits(0x7ff8000000000001);
        check_roof_boundary_hash(&triangles, &boundary);
        for (a, b) in [
            (DVec2::new(1e8, -1e8), DVec2::new(1e8 + 1., -1e8 + 1e-7)),
            (DVec2::new(-1e16, 1e8), DVec2::new(1., 1e8 + 1.)),
            (DVec2::ZERO, DVec2::new(1e-12, 1e-18)),
            (
                DVec2::new(1e150, -1e150),
                DVec2::new(1e150 + 1e140, -1e150 + 1e140),
            ),
        ] {
            let end = a + (b - a);
            let samples = [
                a,
                b,
                end,
                a.lerp(b, 0.5),
                end.map(f64::next_up),
                end.map(f64::next_down),
            ];
            let triangles: Vec<_> = samples
                .chunks_exact(3)
                .enumerate()
                .map(|(i, p)| RoofTriangle {
                    points: [p[0], p[1], p[2]],
                    heights: [i as f64; 3],
                })
                .collect();
            check_roof_boundary_hash(&triangles, &[[a, b], [b, a]]);
        }
    }

    #[test]
    fn roof_boundary_hash_preserves_nonfinite_and_overflow_behavior() {
        let normal = RoofTriangle {
            points: [DVec2::ZERO, DVec2::X, DVec2::new(0.5, 0.5)],
            heights: [100.; 3],
        };
        let boundary = [[DVec2::ZERO, DVec2::X]];
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let mut invalid = normal.clone();
            invalid.points[2].x = value;
            let (old, new) = check_roof_boundary_hash(&[normal.clone(), invalid], &boundary);
            assert_eq!(old, new);
            let (old, new) =
                check_roof_boundary_hash(&[normal.clone()], &[[DVec2::new(value, 0.), DVec2::X]]);
            assert_eq!(old, new);
        }
        let overflow = [[DVec2::new(-f64::MAX, 0.), DVec2::new(f64::MAX, 0.)]];
        let (old, new) = check_roof_boundary_hash(&[normal.clone()], &overflow);
        assert_eq!(old, new);
        let (old, new) = check_roof_boundary_hash(&[normal], &[]);
        assert!(old > 0);
        assert_eq!(new, old);
    }

    #[test]
    fn roof_boundary_hash_preserves_dense_rotated_grid_order() {
        for angle in [0.0_f64, 0.19, 0.78] {
            let rotate = |p: DVec2| {
                DVec2::new(
                    p.x * angle.cos() - p.y * angle.sin(),
                    p.x * angle.sin() + p.y * angle.cos(),
                )
            };
            let mut triangles = Vec::new();
            for y in 0..48 {
                for x in 0..48 {
                    let p = DVec2::new(x as f64, y as f64);
                    let corners = [p, p + DVec2::X, p + DVec2::ONE, p + DVec2::Y];
                    let center = p + DVec2::splat(0.5);
                    for i in 0..4 {
                        triangles.push(RoofTriangle {
                            points: [corners[i], corners[(i + 1) % 4], center].map(rotate),
                            heights: [100.; 3],
                        });
                    }
                }
            }
            let corners = [
                DVec2::ZERO,
                DVec2::new(48., 0.),
                DVec2::splat(48.),
                DVec2::new(0., 48.),
            ]
            .map(rotate);
            let boundary: Vec<_> = (0..4).map(|i| [corners[i], corners[(i + 1) % 4]]).collect();
            let (old, new) = check_roof_boundary_hash(&triangles, &boundary);
            assert_eq!(new, old);
            assert_eq!(new, 49 * 49 + 48 * 48);
        }
    }

    #[test]
    #[ignore = "requires VEARTH_ROOF_CONCAVITY_FIXTURE (C41 all28 visits)"]
    fn roof_boundary_hash_actual_source_benchmark() {
        use vearth_platform::time::Instant;
        let cases = roof_source_cases();
        let mut totals = [0_u128; 2];
        let mut vertices = [0usize; 2];
        for case in cases {
            // Full current roof first; this is outside the measured component.
            let frame = ChunkFrame::of(vearth_core::cell::ChunkAddr {
                cell: case.cell,
                k: 0,
            });
            let mut roof = RoofField::new(
                &case.rings,
                Some(case.shape),
                100.0,
                100.0 + case.rise,
                case.pitch,
            );
            roof.tiered = case.tiered;
            if let Some(extent) = case.expected_extent {
                assert_eq!(
                    (roof.bounds[1] - roof.bounds[0])
                        .to_array()
                        .map(f64::to_bits),
                    extent.map(f64::to_bits),
                    "{} source projection differs",
                    case.name
                );
            }
            let (cells, source_boundary) = roof_policy_cells(&roof, &case.rings, false);
            let error = RoofResolution::for_frame(&frame)
                .sampling_tolerance(roof.top - roof.eave, roof.shape);
            let geometry = roof_policy_stats(&cells, &source_boundary, &roof, error).0;
            let domain = Footprint::new(vec![&case.rings], Vec::new()).unwrap();
            let boundary = domain.boundary();
            let mut times = [Vec::new(), Vec::new()];
            let mut counts = [0; 2];
            for repeat in 0..4 {
                let mut result = [Vec::new(), Vec::new()];
                for variant in if repeat % 2 == 0 { [0, 1] } else { [1, 0] } {
                    let start = Instant::now();
                    let index = match variant {
                        0 => RoofBoundaryIndex::new_std(&geometry.triangles),
                        _ => RoofBoundaryIndex::new(&geometry.triangles),
                    };
                    counts[variant] = index.samples.len();
                    result[variant] = index.boundary(&boundary);
                    drop(index);
                    if repeat > 0 {
                        times[variant].push(start.elapsed().as_nanos());
                    }
                }
                assert_same_roof_boundary(&result[0], &result[1]);
                assert_same_roof_boundary(&result[0], &geometry.boundary);
            }
            for values in &mut times {
                values.sort_unstable();
            }
            for i in 0..2 {
                totals[i] += times[i][1];
                vertices[i] += counts[i];
            }
            eprintln!(
                "roof_boundary_hash {} triangles={} edges={} unique_std_fx={}->{} complete_index_us_std_fx={:.3}->{:.3}",
                case.name,
                geometry.triangles.len(),
                boundary.len(),
                counts[0],
                counts[1],
                times[0][1] as f64 / 1000.,
                times[1][1] as f64 / 1000.
            );
        }
        eprintln!(
            "roof_boundary_hash totals unique_std_fx={vertices:?} complete_index_ms_std_fx={:.3}->{:.3}",
            totals[0] as f64 / 1e6,
            totals[1] as f64 / 1e6
        );
    }

    #[test]
    fn roof_boundary_index_preserves_cartesian_geometry_tolerance_and_ties() {
        let triangles: Vec<_> = [
            [[0.0, 0.0], [0.5, 0.0], [0.5, 3.0e-6]],
            [[0.5, 0.0], [0.5, 4.0e-6], [1.0, 0.0]],
            [[1.0 + 0.9e-5, 0.0], [1.0 + 1.1e-5, 0.0], [0.5, 3.1e-6]],
            [[0.75, 0.9999e-5], [0.75, 1.0001e-5], [-0.0, 0.0]],
            [[0.0, 1.0], [1.0, 1.0], [0.5, 0.5]],
        ]
        .into_iter()
        .map(|points| {
            let points = points.map(DVec2::from_array);
            RoofTriangle {
                points,
                heights: points.map(|p| 100.0 + p.x + p.y * 2.0),
            }
        })
        .collect();
        let boundary = [
            [DVec2::ZERO, DVec2::X],
            [DVec2::X, DVec2::ZERO],
            [DVec2::ZERO, DVec2::Y],
            [DVec2::ZERO, DVec2::ONE],
        ];
        let index = RoofBoundaryIndex::new(&triangles);
        assert!(index.samples.len() < triangles.len() * 3);
        assert_same_roof_boundary(
            &index.boundary(&boundary),
            &cartesian_roof_boundary(&triangles, &boundary),
        );

        let rings = vec![square(-16.0, 16.0), square(-5.0, 5.0)];
        let domain = Footprint::new(vec![&rings], Vec::new()).unwrap();
        let boundary = domain.boundary();
        let roof = RoofField::new(&rings, Some(RoofShape::Hipped), 8.0, 11.0, 0.55);
        let geometry = roof_geometry(
            domain.roof_cells(3.0).unwrap(),
            &boundary,
            &|p| roof.height(p),
            true,
        )
        .unwrap();
        assert_same_roof_boundary(
            &geometry.boundary,
            &cartesian_roof_boundary(&geometry.triangles, &boundary),
        );
    }

    #[test]
    fn roof_boundary_index_skips_interior_points_for_long_axis_aligned_edges() {
        const SIDE: usize = 32;
        let mut triangles = Vec::new();
        for y in 0..SIDE {
            for x in 0..SIDE {
                let lo = DVec2::new(x as f64, y as f64);
                let points = [lo, lo + DVec2::X, lo + DVec2::ONE, lo + DVec2::Y];
                let center = lo + DVec2::splat(0.5);
                for i in 0..4 {
                    triangles.push(RoofTriangle {
                        points: [points[i], points[(i + 1) % 4], center],
                        heights: [100.0; 3],
                    });
                }
            }
        }
        let index = RoofBoundaryIndex::new(&triangles);
        assert_eq!(index.samples.len(), (SIDE + 1).pow(2) + SIDE.pow(2));
        let far = SIDE as f64;
        let corners = [
            DVec2::ZERO,
            DVec2::new(far, 0.0),
            DVec2::splat(far),
            DVec2::new(0.0, far),
        ];
        let boundary: [_; 4] = std::array::from_fn(|i| [corners[i], corners[(i + 1) % 4]]);
        let mut candidate_count = 0;
        for edge in boundary {
            let candidates: Vec<_> = index.candidates(edge).collect();
            assert_eq!(candidates.len(), SIDE + 1);
            let direction = edge[1] - edge[0];
            assert!(
                candidates
                    .iter()
                    .all(|&i| { (index.samples[i].point - edge[0]).perp_dot(direction) == 0.0 })
            );
            candidate_count += candidates.len();
        }
        assert_eq!(candidate_count, 132);
        assert_eq!(boundary.len() * triangles.len() * 3, 49_152);
        assert_same_roof_boundary(
            &index.boundary(&boundary),
            &cartesian_roof_boundary(&triangles, &boundary),
        );
    }
    fn reference_emit_triangle(
        mesh: &mut MeshBuffers,
        mut points: [DVec3; 3],
        voxel: SurfaceAppearance,
        up: bool,
    ) {
        let mut normal = (points[1] - points[0]).cross(points[2] - points[0]);
        if !normal.is_finite() || normal.length_squared() < 1.0e-16 {
            return;
        }
        if up && normal.y < 0.0 {
            points.swap(1, 2);
            normal = -normal;
        }
        let normal = normal.normalize();
        let mut polygon = points.to_vec();
        for (axis, limit, greater) in [
            (0, 0.0, true),
            (0, N as f64, false),
            (2, 0.0, true),
            (2, N as f64, false),
        ] {
            let input = std::mem::take(&mut polygon);
            if input.len() < 3 {
                return;
            }
            for i in 0..input.len() {
                let a = input[i];
                let b = input[(i + 1) % input.len()];
                let inside = |p: DVec3| {
                    if greater {
                        p[axis] >= limit
                    } else {
                        p[axis] <= limit
                    }
                };
                if inside(a) {
                    polygon.push(a);
                }
                if inside(a) != inside(b) {
                    let mut p = a.lerp(b, (limit - a[axis]) / (b[axis] - a[axis]));
                    p[axis] = limit;
                    polygon.push(p);
                }
            }
        }
        if polygon.len() < 3 {
            return;
        }
        let base = mesh.vertices.len() as u32;
        let face = if normal.y > 0.65 {
            2
        } else if normal.y < -0.65 {
            3
        } else if normal.x.abs() > normal.z.abs() {
            if normal.x > 0.0 { 0 } else { 1 }
        } else if normal.z > 0.0 {
            4
        } else {
            5
        };
        let face_ao = encode_surface_normal(
            face | 3 << 3 | u32::from(voxel.material) << 5,
            normal.as_vec3().to_array(),
        );
        for p in &polygon {
            mesh.vertices.push(SurfaceVertex {
                pos: p.as_vec3().to_array(),
                face_ao,
                rgba: [voxel.tint[0], voxel.tint[1], voxel.tint[2], 255],
            });
        }
        for i in 1..polygon.len().saturating_sub(1) {
            mesh.indices
                .extend([base, base + i as u32, base + i as u32 + 1]);
        }
    }

    #[test]
    fn clipping_reuse_preserves_all_vertex_and_index_bits() {
        let appearance = SurfaceAppearance {
            material: crate::material::BRICK_WALL,
            tint: [149, 79, 63],
        };
        let mut expected = MeshBuffers::default();
        let mut actual = MeshBuffers::default();
        for i in 0..4096 {
            let x = ((i * 719) % 59) as f64 - 14.0;
            let y = ((i * 227) % 67) as f64 - 18.0;
            let points = [
                DVec3::new(x, 1.0, y),
                DVec3::new(x + 8.5, 2.0, y + 1.25),
                DVec3::new(x - 1.25, 5.0, y + 8.5),
            ];
            reference_emit_triangle(&mut expected, points, appearance, i % 2 == 0);
            emit_triangle(&mut actual, points, appearance, i % 2 == 0);
        }
        assert_eq!(actual.vertices, expected.vertices);
        assert_eq!(actual.indices, expected.indices);
    }
    struct ReferenceRoofSampler<'a> {
        at: &'a dyn Fn(DVec2) -> f64,
        // Exact coordinate bits are the key; this map is never iterated. A numeric
        // hasher changes lookup cost only, including distinct signed-zero samples.
        cache: rustc_hash::FxHashMap<(u64, u64), f64>,
    }
    impl ReferenceRoofSampler<'_> {
        fn height(&mut self, point: DVec2) -> Option<f64> {
            let value = *self
                .cache
                .entry((point.x.to_bits(), point.y.to_bits()))
                .or_insert_with(|| (self.at)(point));
            value.is_finite().then_some(value)
        }
        fn planar_sections(&mut self, points: [DVec2; 3]) -> Option<Vec<RoofTriangle>> {
            let origin = (points[0] + points[1] + points[2]) / 3.0;
            let a = (points[1] - points[0]) * 0.02;
            let b = (points[2] - points[0]) * 0.02;
            let determinant = a.perp_dot(b);
            if determinant.abs() < 1.0e-12 {
                return None;
            }
            let mut planes: Vec<RoofPlane> = Vec::new();
            for weights in [
                [0.82, 0.09, 0.09],
                [0.09, 0.82, 0.09],
                [0.09, 0.09, 0.82],
                [0.48, 0.48, 0.04],
                [0.48, 0.04, 0.48],
                [0.04, 0.48, 0.48],
                [1.0 / 3.0; 3],
            ] {
                let p = points[0] * weights[0] + points[1] * weights[1] + points[2] * weights[2];
                let h = self.height(p)?;
                let da = self.height(p + a)? - h;
                let db = self.height(p + b)? - h;
                let gradient = DVec2::new(
                    (da * b.y - db * a.y) / determinant,
                    (a.x * db - b.x * da) / determinant,
                );
                let plane = RoofPlane {
                    origin,
                    gradient,
                    height: h + gradient.dot(origin - p),
                };
                if !planes.iter().any(|other| {
                    other.gradient.distance(gradient) < 1.0e-3
                        && (other.height - plane.height).abs() < 0.001
                }) {
                    planes.push(plane);
                }
            }
            if !(2..=4).contains(&planes.len()) {
                return None;
            }
            for p in points.into_iter().chain([
                (points[0] + points[1]) * 0.5,
                (points[1] + points[2]) * 0.5,
                (points[2] + points[0]) * 0.5,
                origin,
            ]) {
                let predicted = planes
                    .iter()
                    .map(|plane| plane.at(p))
                    .min_by(f64::total_cmp)?;
                if (self.height(p)? - predicted).abs() > 0.001 {
                    return None;
                }
            }
            let mut result = Vec::new();
            for (i, plane) in planes.iter().enumerate() {
                let mut polygon = points.to_vec();
                for (j, other) in planes.iter().enumerate() {
                    if i != j {
                        polygon = clip_roof_plane(&polygon, *plane, *other);
                    }
                }
                if polygon.len() < 3 {
                    continue;
                }
                let center = polygon.iter().copied().sum::<DVec2>() / polygon.len() as f64;
                if (self.height(center)? - plane.at(center)).abs() > 0.001 {
                    return None;
                }
                for i in 0..polygon.len() {
                    let midpoint = (polygon[i] + polygon[(i + 1) % polygon.len()]) * 0.5;
                    if (self.height(midpoint)? - plane.at(midpoint)).abs() > 0.001 {
                        return None;
                    }
                }
                for i in 1..polygon.len().saturating_sub(1) {
                    let p = [polygon[0], polygon[i], polygon[i + 1]];
                    if (p[1] - p[0]).perp_dot(p[2] - p[0]).abs() < 1.0e-12 {
                        continue;
                    }
                    let heights = [self.height(p[0])?, self.height(p[1])?, self.height(p[2])?];
                    if (self.height((p[0] + p[1] + p[2]) / 3.0)?
                        - heights.iter().sum::<f64>() / 3.0)
                        .abs()
                        > ROOF_PLANE_ERROR_M
                        || (0..3).any(|j| {
                            let next = (j + 1) % 3;
                            self.height((p[j] + p[next]) * 0.5).is_none_or(|h| {
                                (h - (heights[j] + heights[next]) * 0.5).abs() > ROOF_PLANE_ERROR_M
                            })
                        })
                    {
                        return None;
                    }
                    result.push(RoofTriangle { points: p, heights });
                }
            }
            (!result.is_empty()).then_some(result)
        }

        fn triangle(
            &mut self,
            points: [DVec2; 3],
            refine: bool,
            depth: u8,
            output: &mut Vec<RoofTriangle>,
        ) -> Option<()> {
            if (points[1] - points[0])
                .perp_dot(points[2] - points[0])
                .abs()
                < 1.0e-12
            {
                return Some(());
            }
            if output.len() >= MAX_ROOF_TRIANGLES {
                return None;
            }
            let heights = [
                self.height(points[0])?,
                self.height(points[1])?,
                self.height(points[2])?,
            ];
            if refine {
                let midpoints = [
                    (points[0] + points[1]) * 0.5,
                    (points[1] + points[2]) * 0.5,
                    (points[2] + points[0]) * 0.5,
                ];
                let middle = (points[0] + points[1] + points[2]) / 3.0;
                let mut error = (self.height(middle)? - heights.iter().sum::<f64>() / 3.0).abs();
                for i in 0..3 {
                    error = error.max(
                        (self.height(midpoints[i])? - (heights[i] + heights[(i + 1) % 3]) * 0.5)
                            .abs(),
                    );
                }
                if error > ROOF_PLANE_ERROR_M {
                    if let Some(sections) = self.planar_sections(points) {
                        if output.len() + sections.len() > MAX_ROOF_TRIANGLES {
                            return None;
                        }
                        output.extend(sections);
                        return Some(());
                    }
                    if depth >= MAX_ROOF_REFINEMENT {
                        return None;
                    }
                    for child in [
                        [points[0], midpoints[0], midpoints[2]],
                        [midpoints[0], points[1], midpoints[1]],
                        [midpoints[2], midpoints[1], points[2]],
                        [midpoints[0], midpoints[1], midpoints[2]],
                    ] {
                        self.triangle(child, true, depth + 1, output)?;
                    }
                    return Some(());
                }
            }
            output.push(RoofTriangle { points, heights });
            Some(())
        }
    }

    fn reference_roof_geometry(
        cells: impl IntoIterator<Item = [DVec2; 4]>,
        boundary: &[[DVec2; 2]],
        at: &dyn Fn(DVec2) -> f64,
        refine: bool,
    ) -> Option<RoofGeometry> {
        let mut sampler = ReferenceRoofSampler {
            at,
            cache: rustc_hash::FxHashMap::default(),
        };
        let mut triangles = Vec::new();
        let snap = |point: DVec2| {
            for [a, b] in boundary {
                let t = ((point - *a).dot(*b - *a) / a.distance_squared(*b)).clamp(0.0, 1.0);
                let closest = a.lerp(*b, t);
                if point.distance_squared(closest) < 1.0e-8 {
                    return closest;
                }
            }
            point
        };
        let cells: Vec<_> = cells.into_iter().collect();
        for cell in &cells {
            let cell = cell.map(snap);
            let center = cell.iter().copied().sum::<DVec2>() * 0.25;
            for edge in 0..4 {
                if sampler
                    .triangle(
                        [cell[edge], cell[(edge + 1) % 4], center],
                        refine,
                        0,
                        &mut triangles,
                    )
                    .is_none()
                {
                    // Refinement is optional: the whole-feature base grid keeps
                    // the same height field and shared boundary within budget.
                    // Invalid samples still fail the unrefined pass.
                    return if refine {
                        reference_roof_geometry(cells, boundary, at, false)
                    } else {
                        None
                    };
                }
            }
        }
        // Roofs, gable closures and eaves consume the same boundary polyline.
        // Re-sampling that boundary on a different grid opens gaps at crests.
        let sampled_boundary = RoofBoundaryIndex::new(&triangles).boundary(boundary);
        Some(RoofGeometry {
            triangles,
            boundary: sampled_boundary,
        })
    }

    fn reference_roof_height(roof: &RoofField, p: DVec2) -> f64 {
        let p = roof.rotate(p);
        let rise = (roof.top - roof.eave).max(0.0);
        let short_span = (roof.bounds[1].y - roof.bounds[0].y).max(0.01);
        let height = match roof.shape {
            Some(
                shape @ (RoofShape::Gabled
                | RoofShape::Hipped
                | RoofShape::Pyramidal
                | RoofShape::Mansard
                | RoofShape::Gambrel),
            ) => {
                let gabled =
                    matches!(shape, RoofShape::Gabled | RoofShape::Gambrel) && !roof.reentrant;
                // A roof must be defined on its boundary as well as inside it.
                // Source-edge distances give continuous hip wings; gables use
                // the supporting planes of their long sides, retaining their
                // ridge at either end without cross-section membership tests.
                let distance = roof
                    .edges
                    .iter()
                    .map(|edge| &edge.points)
                    .filter_map(|[a, b]| {
                        let edge = *b - *a;
                        if gabled {
                            (edge.x.abs() >= edge.y.abs())
                                .then(|| edge.perp_dot(p - *a).abs() / edge.length())
                        } else {
                            let t = ((p - *a).dot(edge) / edge.length_squared()).clamp(0.0, 1.0);
                            Some(p.distance(a.lerp(*b, t)))
                        }
                    })
                    .min_by(f64::total_cmp)
                    .unwrap_or(0.0);
                let pitch = if roof.reentrant {
                    roof.pitch
                } else {
                    roof.pitch.min(rise * 2.0 / short_span)
                };
                if roof.tiered {
                    // Broad horizontal returns separate three pitched tiers.
                    // One continuous source-owned shell retains the exact base
                    // outline and top, including irregular courtyard wings.
                    let q = (distance * 2.0 / short_span).clamp(0.0, 1.0);
                    let profile = if q < 0.28 {
                        q / 0.28 * 0.34
                    } else if q < 0.35 {
                        0.34
                    } else if q < 0.63 {
                        0.34 + (q - 0.35) / 0.28 * 0.36
                    } else if q < 0.70 {
                        0.70
                    } else {
                        0.70 + (q - 0.70) / 0.30 * 0.30
                    };
                    rise * profile
                } else if matches!(shape, RoofShape::Mansard | RoofShape::Gambrel) {
                    // A steep lower roof and shallower upper plane share the
                    // same source boundary, ridge direction and reported top.
                    let q = (distance * 2.0 / short_span).clamp(0.0, 1.0);
                    rise * if q < 0.28 {
                        q / 0.28 * 0.78
                    } else {
                        0.78 + (q - 0.28) / 0.72 * 0.22
                    }
                } else {
                    distance * pitch
                }
            }
            Some(RoofShape::Skillion) => {
                rise * ((p.y - roof.bounds[0].y) / short_span).clamp(0.0, 1.0)
            }
            Some(RoofShape::Dome | RoofShape::Onion) => {
                let [lo, hi] = roof.bounds;
                let q = ((p - (lo + hi) * 0.5) / ((hi - lo) * 0.5))
                    .length()
                    .min(1.0);
                (1.0 - q * q).max(0.0).sqrt() * rise
            }
            _ => rise,
        };
        roof.eave + height.clamp(0.0, rise)
    }

    fn assert_same_roof_geometry(actual: &RoofGeometry, expected: &RoofGeometry) {
        let triangles = |mesh: &RoofGeometry| {
            mesh.triangles
                .iter()
                .map(|t| {
                    (
                        t.points.map(|p| [p.x.to_bits(), p.y.to_bits()]),
                        t.heights.map(f64::to_bits),
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(triangles(actual), triangles(expected));
        assert_same_roof_boundary(&actual.boundary, &expected.boundary);
    }

    #[test]
    fn roof_sample_reuse_preserves_exact_refinement_and_boundary() {
        let footprints = [
            vec![vec![
                [-13.0, -9.0],
                [15.0, -8.0],
                [14.0, 11.0],
                [-12.0, 10.0],
            ]],
            vec![vec![
                [-13.0, -9.0],
                [15.0, -9.0],
                [15.0, 0.0],
                [2.0, 0.0],
                [2.0, 11.0],
                [-13.0, 11.0],
            ]],
            vec![square(-12.0, 12.0), square(-4.0, 4.0)],
        ];
        for rings in &footprints {
            let domain = Footprint::new(vec![rings], Vec::new()).unwrap();
            let cells = domain.roof_cells(4.0).unwrap();
            let boundary = domain.boundary();
            for shape in [
                RoofShape::Flat,
                RoofShape::Gabled,
                RoofShape::Hipped,
                RoofShape::Mansard,
                RoofShape::Gambrel,
                RoofShape::Pyramidal,
                RoofShape::Skillion,
                RoofShape::Dome,
            ] {
                for tiered in [false, true] {
                    let mut roof = RoofField::new(rings, Some(shape), 100.0, 105.0, 0.7);
                    roof.tiered = tiered;
                    let current = |p| roof.height(p);
                    let legacy = |p| reference_roof_height(&roof, p);
                    for i in 0..257 {
                        let p = DVec2::new(
                            (i * 719 % 1009) as f64 / 31.0 - 16.0,
                            (i * 313 % 991) as f64 / 29.0 - 17.0,
                        );
                        assert_eq!(current(p).to_bits(), legacy(p).to_bits());
                    }
                    for refine in [false, true] {
                        let actual =
                            roof_geometry(cells.iter().copied(), &boundary, &current, refine)
                                .unwrap();
                        let expected = reference_roof_geometry(
                            cells.iter().copied(),
                            &boundary,
                            &legacy,
                            refine,
                        )
                        .unwrap();
                        assert_same_roof_geometry(&actual, &expected);
                    }
                }
            }
        }
    }

    #[test]
    fn indexed_roof_distances_preserve_complex_height_fields_exactly() {
        let outline: Vec<_> = (0..192)
            .map(|i| {
                let angle = i as f64 * std::f64::consts::TAU / 192.0;
                let radius = 25.0 + if i % 4 < 2 { 7.0 } else { 0.0 };
                [(angle.cos() * radius) as f32, (angle.sin() * radius) as f32]
            })
            .collect();
        let rings = vec![outline, square(-3.0, 3.0)];
        for shape in [
            RoofShape::Gabled,
            RoofShape::Hipped,
            RoofShape::Mansard,
            RoofShape::Gambrel,
            RoofShape::Pyramidal,
        ] {
            for tiered in [false, true] {
                let mut roof = RoofField::new(&rings, Some(shape), 60.0, 76.0, 0.7);
                roof.tiered = tiered;
                assert!(roof.distance_index.is_some());
                for i in 0..8192 {
                    let p = DVec2::new(
                        (i * 719 % 4093) as f64 / 53.0 - 40.0,
                        (i * 313 % 4091) as f64 / 51.0 - 40.0,
                    );
                    assert_eq!(
                        roof.height(p).to_bits(),
                        reference_roof_height(&roof, p).to_bits(),
                        "{shape:?}, {p:?}"
                    );
                }
                for p in rings.iter().flatten() {
                    let p = DVec2::from_array(p.map(f64::from));
                    for delta in [DVec2::ZERO, DVec2::splat(1e-10), DVec2::splat(-1e-10)] {
                        assert_eq!(
                            roof.height(p + delta).to_bits(),
                            reference_roof_height(&roof, p + delta).to_bits()
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn roof_distance_caps_require_saturation_in_actual_float_arithmetic() {
        for short_span in [
            f64::from_bits(1),
            0.01,
            1.0,
            31.7,
            1.0e150,
            f64::MAX,
            f64::INFINITY,
        ] {
            for (eave, top) in [
                (0.0, f64::from_bits(1)),
                (0.0, f64::MIN_POSITIVE),
                (100.0, 100.0f64.next_up()),
                (-1.0e150, 1.0e150),
                (100.0, 100.0),
                (100.0, 99.0),
                (0.0, f64::INFINITY),
                (f64::NAN, 100.0),
            ] {
                for pitch in [f64::from_bits(1), 0.0, -1.0, 0.7, f64::MAX, f64::NAN] {
                    let caps = roof_distance_caps(eave, top, short_span, pitch);
                    let rise = (top - eave).max(0.0);
                    if caps[0].is_finite() {
                        assert!(caps[0] > 0.0 && pitch > 0.0 && pitch.is_finite());
                        assert!(caps[0].sqrt() * pitch >= rise);
                    }
                    if caps[1].is_finite() {
                        assert!(caps[1] > 0.0 && short_span > 0.0 && short_span.is_finite());
                        assert!(caps[1].sqrt() * 2.0 / short_span >= 1.0);
                    }
                    if !eave.is_finite() || !top.is_finite() || rise <= 0.0 {
                        assert_eq!(caps, [f64::INFINITY; 2]);
                    }
                }
            }
        }
    }

    #[test]
    fn roof_distance_caps_preserve_height_bits_across_profiles_and_numeric_limits() {
        let ring = |count: usize, reentrant: bool, scale: f64| {
            (0..count)
                .map(|i| {
                    let angle = i as f64 * std::f64::consts::TAU / count as f64;
                    let radius = if reentrant && i % 4 < 2 { 23.0 } else { 31.0 };
                    [
                        (angle.cos() * radius * scale) as f32,
                        (angle.sin() * radius * scale) as f32,
                    ]
                })
                .collect::<Vec<_>>()
        };
        let shapes = [
            None,
            Some(RoofShape::Flat),
            Some(RoofShape::Gabled),
            Some(RoofShape::Hipped),
            Some(RoofShape::Pyramidal),
            Some(RoofShape::Mansard),
            Some(RoofShape::Gambrel),
            Some(RoofShape::Skillion),
            Some(RoofShape::Sawtooth),
            Some(RoofShape::Dome),
            Some(RoofShape::Onion),
        ];
        let mut finite_cap_cases = 0;
        for scale in [0.001, 1.0, 1000.0] {
            for mut rings in [
                vec![ring(24, false, scale)],
                vec![ring(64, true, scale)],
                vec![ring(32, true, scale), ring(12, false, scale * 0.2)],
            ] {
                if scale == 1000.0 {
                    for p in rings.iter_mut().flatten() {
                        p[0] += 1.0e8;
                        p[1] -= 2.0e8;
                    }
                }
                for (eave, top, pitch) in [
                    (0.0, f64::from_bits(1), 0.7),
                    (0.0, f64::MIN_POSITIVE, f32::MIN_POSITIVE),
                    (100.0, 100.0f64.next_up(), 0.7),
                    (100.0, 100.1, 0.7),
                    (-17.5, 5.0, 0.7),
                    (100.0, 106.0, 0.7),
                    (1.0e12, 1.0e12 + 0.125, 0.7),
                    (100.0, 100.0, 0.7),
                    (100.0, 99.0, 0.7),
                    (0.0, 1.0e150, f32::MIN_POSITIVE),
                    (100.0, 106.0, 0.0),
                    (100.0, 106.0, -1.0),
                    (100.0, 106.0, f32::MAX),
                    (100.0, 106.0, f32::INFINITY),
                    (100.0, 106.0, f32::NAN),
                    (100.0, f64::INFINITY, 0.7),
                ] {
                    let mut roof =
                        RoofField::new(&rings, Some(RoofShape::Hipped), eave, top, pitch);
                    assert!(roof.distance_index.is_some());
                    let mut points: Vec<_> = (0..97)
                        .map(|i| {
                            roof.unrotate(
                                roof.bounds[0]
                                    + (roof.bounds[1] - roof.bounds[0])
                                        * DVec2::new(
                                            (i * 719 % 1009) as f64 / 840.0 - 0.1,
                                            (i * 313 % 991) as f64 / 826.0 - 0.1,
                                        ),
                            )
                        })
                        .collect();
                    points.extend(
                        rings
                            .iter()
                            .flatten()
                            .map(|p| DVec2::from_array(p.map(f64::from))),
                    );
                    for cap in roof.distance_caps.into_iter().filter(|c| c.is_finite()) {
                        finite_cap_cases += 1;
                        let edge = &roof.edges[0];
                        let midpoint = (edge.points[0] + edge.points[1]) * 0.5;
                        let normal = DVec2::new(-edge.delta.y, edge.delta.x) / edge.length;
                        for d in [cap.sqrt().next_down(), cap.sqrt(), cap.sqrt().next_up()] {
                            for sign in [-1.0, 1.0] {
                                let point = roof.unrotate(midpoint + normal * (d * sign));
                                if point.is_finite() {
                                    points.push(point);
                                }
                            }
                        }
                    }
                    for shape in shapes {
                        // Cached plain/profile bounds must survive the supported
                        // post-construction shape and regional-tier selection.
                        roof.shape = shape;
                        for tiered in [false, true] {
                            roof.tiered = tiered;
                            for &p in &points {
                                assert_eq!(
                                    roof.height(p).to_bits(),
                                    roof.height_with_distance_cap::<false>(p).to_bits(),
                                    "shape={shape:?} tiered={tiered} scale={scale} eave={eave} top={top} pitch={pitch} p={p:?}"
                                );
                            }
                        }
                    }
                }
            }
        }
        assert!(finite_cap_cases > 100);
    }

    #[test]
    fn roof_distance_caps_keep_refinement_and_boundary_geometry_identical() {
        let outline: Vec<_> = (0..32)
            .map(|i| {
                let angle = i as f64 * std::f64::consts::TAU / 32.0;
                let radius = if i % 4 < 2 { 13.0 } else { 17.0 };
                [(angle.cos() * radius) as f32, (angle.sin() * radius) as f32]
            })
            .collect();
        let rings = vec![outline, square(-2.0, 2.0)];
        let footprint = Footprint::new(vec![&rings], Vec::new()).unwrap();
        let cells = footprint.roof_cells(3.0).unwrap();
        let boundary = footprint.boundary();
        for shape in [RoofShape::Hipped, RoofShape::Mansard] {
            for tiered in [false, true] {
                let mut roof = RoofField::new(&rings, Some(shape), 100.0, 102.0, 0.7);
                roof.tiered = tiered;
                let capped = |p| roof.height(p);
                let unbounded = |p| roof.height_with_distance_cap::<false>(p);
                for refine in [false, true] {
                    let actual =
                        roof_geometry(cells.iter().copied(), &boundary, &capped, refine).unwrap();
                    let expected =
                        roof_geometry(cells.iter().copied(), &boundary, &unbounded, refine)
                            .unwrap();
                    assert_same_roof_geometry(&actual, &expected);
                }
            }
        }
    }

    #[test]
    #[ignore = "requires VEARTH_ROOF_CAP_FIXTURE with attributed source component EN rings"]
    fn actual_source_roof_distance_cap_benchmark() {
        use std::{hint::black_box, time::Instant};
        let path =
            vearth_platform::env::var("VEARTH_ROOF_CAP_FIXTURE").expect("source fixture path");
        let input = std::fs::read_to_string(path).unwrap();
        let mut sources: Vec<(String, u64, [f64; 2], Rings)> = Vec::new();
        for line in input
            .lines()
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
        {
            let words: Vec<_> = line.split_whitespace().collect();
            if words[0] == "building" {
                assert_eq!(words.len(), 6);
                sources.push((
                    format!("{}/{}", words[1], words[2]),
                    words[3].parse().unwrap(),
                    [words[4].parse().unwrap(), words[5].parse().unwrap()],
                    Vec::new(),
                ));
            } else if words[0] == "ring" {
                sources.last_mut().unwrap().3.push(Vec::new());
            } else {
                assert_eq!(words.len(), 2);
                sources
                    .last_mut()
                    .unwrap()
                    .3
                    .last_mut()
                    .unwrap()
                    .push([words[0].parse().unwrap(), words[1].parse().unwrap()]);
            }
        }
        assert!(!sources.is_empty());
        for (id, seed, location, rings) in sources {
            let cell = vearth_core::cell::CellId::from_dir(
                ellipsoid::geodetic_to_ecef(location[0], location[1], 0.0),
                11,
            );
            let frame = ChunkFrame::of(vearth_core::cell::ChunkAddr { cell, k: 0 });
            let mut pack = FeaturePack::empty();
            pack.buildings.push(Building {
                rings: rings.clone(),
                location_rad: Some(location),
                height_m: 5.0 + (seed % 900) as f32 / 100.0,
                min_height_m: 0.0,
                seed,
                kind: BuildingKind::Building,
                roof_shape: None,
                use_kind: Default::default(),
            });
            let stamp =
                tier0::build_overlay(cell, &frame, &pack, &|_, _| 100.0, 0, None, None, None)
                    .source_buildings[0];
            eprintln!(
                "source={id} edges={} isolated-source style={:?} roof={:?} rise={:?}; native neighboring landuse absent",
                rings.iter().map(Vec::len).sum::<usize>(),
                stamp.design.style,
                stamp.roof_shape,
                stamp.roof_height_m
            );
            let native_rise = f64::from(stamp.roof_height_m.unwrap_or(0.0));
            let top = 100.0 + f64::from(pack.buildings[0].height_m);
            for (label, shape, rise, tiered) in [
                (
                    "isolated-source",
                    stamp.roof_shape,
                    native_rise,
                    expressed_style(&stamp).is_some_and(|s| s.roofline == Roofline::TieredHip),
                ),
                ("controlled-hip", Some(RoofShape::Hipped), 3.0, false),
                ("controlled-mansard", Some(RoofShape::Mansard), 3.0, false),
                ("controlled-tiered", Some(RoofShape::Hipped), 3.0, true),
            ] {
                let mut roof = RoofField::new(&rings, shape, top - rise, top, 0.7);
                roof.tiered = tiered;
                let points: Vec<_> = (0..8192)
                    .map(|i| {
                        roof.unrotate(
                            roof.bounds[0]
                                + (roof.bounds[1] - roof.bounds[0])
                                    * DVec2::new(
                                        (i * 719 % 8191) as f64 / 8191.0,
                                        (i * 313 % 8179) as f64 / 8179.0,
                                    ),
                        )
                    })
                    .collect();
                for &p in &points {
                    assert_eq!(
                        roof.height(p).to_bits(),
                        roof.height_with_distance_cap::<false>(p).to_bits()
                    );
                }
                let run = |capped| {
                    let started = Instant::now();
                    let mut sum = 0.0;
                    for _ in 0..4 {
                        for &p in &points {
                            sum += if capped {
                                roof.height(black_box(p))
                            } else {
                                roof.height_with_distance_cap::<false>(black_box(p))
                            };
                        }
                    }
                    black_box(sum);
                    started.elapsed().as_secs_f64() * 1000.0
                };
                let mut times = [Vec::new(), Vec::new()];
                for repeat in 0..4 {
                    for index in if repeat % 2 == 0 { [0, 1] } else { [1, 0] } {
                        times[index].push(run(index == 1));
                    }
                }
                for values in &mut times {
                    values.sort_by(f64::total_cmp);
                }
                eprintln!(
                    "source={id} {label} queries=32768 unrestricted_ms={:.3} capped_ms={:.3} ratio={:.3}",
                    times[0][1],
                    times[1][1],
                    times[1][1] / times[0][1]
                );
            }
        }
    }

    #[test]
    fn convex_roof_support_candidates_preserve_segment_and_height_bits() {
        let mut fast_queries = 0;
        for count in [3, 4, 7, 12, 16, 24, 64] {
            for orientation in 0..16 {
                let angle = orientation as f64 * std::f64::consts::TAU / 16.0;
                for (origin, scale) in [
                    (DVec2::ZERO, 0.001),
                    (DVec2::ZERO, 1.0),
                    (DVec2::new(1.0e7, -3.0e7), 1.0e4),
                    (DVec2::new(-1.0e12, 3.0e12), 1.0e8),
                ] {
                    let ring: Vec<_> = (0..count)
                        .map(|i| {
                            let phase = if orientation % 2 == 0 {
                                0.0
                            } else {
                                0.35 * (i as f64 * 5.17).sin()
                            };
                            let a = (i as f64 + phase) * std::f64::consts::TAU / count as f64;
                            let p = DVec2::new(a.cos() * 17.0, a.sin() * 9.0) * scale;
                            let p = origin
                                + DVec2::new(
                                    angle.cos() * p.x - angle.sin() * p.y,
                                    angle.sin() * p.x + angle.cos() * p.y,
                                );
                            [p.x as f32, p.y as f32]
                        })
                        .collect();
                    for reverse in [false, true] {
                        let mut ring = ring.clone();
                        if reverse {
                            ring.reverse();
                        }
                        let rings = vec![ring];
                        let mut roof =
                            RoofField::new(&rings, Some(RoofShape::Hipped), 0.0, 20.0 * scale, 0.7);
                        assert_eq!(roof.convex_distances.is_some(), (12..=16).contains(&count));
                        roof.convex_distances = ConvexRoofDistances::new(&roof.edges, true);
                        let convex = roof.convex_distances.as_ref().unwrap();
                        let mut points: Vec<_> = (0..257)
                            .map(|i| {
                                roof.bounds[0]
                                    + (roof.bounds[1] - roof.bounds[0])
                                        * DVec2::new(
                                            (i * 719 % 1009) as f64 / 840.0 - 0.1,
                                            (i * 313 % 991) as f64 / 826.0 - 0.1,
                                        )
                            })
                            .collect();
                        for edge in &roof.edges {
                            let normal = DVec2::new(-edge.delta.y, edge.delta.x) / edge.length;
                            for point in [
                                edge.points[0],
                                edge.points[1],
                                edge.points[0].lerp(edge.points[1], 0.5),
                            ] {
                                for offset in [
                                    0.0,
                                    -1e-12 * scale,
                                    1e-12 * scale,
                                    -1e-6 * scale,
                                    1e-6 * scale,
                                ] {
                                    points.push(point + normal * offset);
                                }
                            }
                        }
                        for &point in &points {
                            if let Some(actual) = convex.nearest_squared(&roof.edges, point) {
                                fast_queries += 1;
                                let expected = roof
                                    .edges
                                    .iter()
                                    .map(|edge| edge.distance_squared(point))
                                    .min_by(f64::total_cmp)
                                    .unwrap();
                                assert_eq!(
                                    actual.to_bits(),
                                    expected.to_bits(),
                                    "n={count} orientation={orientation} reverse={reverse} origin={origin:?} scale={scale} point={point:?}"
                                );
                            }
                        }
                        for shape in [RoofShape::Hipped, RoofShape::Pyramidal, RoofShape::Mansard] {
                            roof.shape = Some(shape);
                            for tiered in [false, true] {
                                roof.tiered = tiered;
                                for &point in &points {
                                    let point = roof.unrotate(point);
                                    assert_eq!(
                                        roof.height(point).to_bits(),
                                        reference_roof_height(&roof, point).to_bits(),
                                        "{shape:?} tiered={tiered} point={point:?}"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
        assert!(
            fast_queries > 100_000,
            "fixture must exercise the support path"
        );
    }

    #[test]
    fn convex_roof_support_filter_rejects_unproven_domains_and_boundary_queries() {
        for rings in [
            vec![square(-12.0, 12.0), square(-3.0, 3.0)],
            vec![vec![
                [0.0, 0.0],
                [10.0, 0.0],
                [10.0, 10.0],
                [5.0, 3.0],
                [0.0, 10.0],
            ]],
            vec![vec![
                [0.0, 0.0],
                [5.0, 0.0],
                [10.0, 0.0],
                [10.0, 10.0],
                [0.0, 10.0],
            ]],
            vec![vec![[0.0, 0.0], [10.0, 10.0], [0.0, 10.0], [10.0, 0.0]]],
        ] {
            let roof = RoofField::new(&rings, Some(RoofShape::Hipped), 0.0, 10.0, 0.7);
            assert!(ConvexRoofDistances::new(&roof.edges, rings.len() == 1).is_none());
        }
        let mut roof = RoofField::new(
            &vec![square(-12.0, 12.0)],
            Some(RoofShape::Hipped),
            0.0,
            10.0,
            0.7,
        );
        roof.convex_distances = ConvexRoofDistances::new(&roof.edges, true);
        let convex = roof.convex_distances.as_ref().unwrap();
        assert!(convex.nearest_squared(&roof.edges, DVec2::ZERO).is_some());
        for point in [
            DVec2::new(12.0, 0.0),
            DVec2::new(12.0, 12.0),
            DVec2::new(13.0, 0.0),
            DVec2::new(12.0 - 1e-13, 0.0),
            DVec2::splat(f64::NAN),
        ] {
            assert!(convex.nearest_squared(&roof.edges, point).is_none());
        }
    }

    #[test]
    #[ignore = "timed exact convex roof query benchmark; run without concurrent game workloads"]
    fn convex_roof_query_benchmark() {
        use std::{hint::black_box, time::Instant};
        for irregular in [false, true] {
            for count in [4, 7, 12, 16, 24, 32, 64] {
                let rings = vec![
                    (0..count)
                        .map(|i| {
                            let phase = if irregular {
                                0.35 * (i as f64 * 5.17).sin()
                            } else {
                                0.0
                            };
                            let angle =
                                (i as f64 + phase) * std::f64::consts::TAU / count as f64 + 0.317;
                            [(angle.cos() * 17.0) as f32, (angle.sin() * 9.0) as f32]
                        })
                        .collect(),
                ];
                let mut roof = RoofField::new(&rings, Some(RoofShape::Hipped), 0.0, 20.0, 0.7);
                roof.convex_distances = ConvexRoofDistances::new(&roof.edges, true);
                let points: Vec<_> = (0..100_000)
                    .map(|i| {
                        let radius = 0.95 * (i % 997) as f64 / 997.0;
                        let angle = (i * 313 % 4091) as f64 * std::f64::consts::TAU / 4091.0;
                        DVec2::new(angle.cos() * 17.0 * radius, angle.sin() * 9.0 * radius)
                    })
                    .collect();
                let started = Instant::now();
                for _ in 0..10 {
                    for &point in &points {
                        black_box(roof.height(black_box(point)));
                    }
                }
                let accelerated = started.elapsed();
                let convex = roof.convex_distances.take().unwrap();
                let started = Instant::now();
                for _ in 0..10 {
                    for &point in &points {
                        black_box(roof.height(black_box(point)));
                    }
                }
                let original = started.elapsed();
                let mut different = 0;
                let mut max_error = 0.0_f64;
                let mut max_ulps = 0;
                let mut fallback_queries = 0;
                for point in points {
                    let point = roof.rotate(point);
                    fallback_queries +=
                        usize::from(convex.nearest_squared(&roof.edges, point).is_none());
                    let direct = convex
                        .lines
                        .iter()
                        .map(|line| line.normal.dot(point) + line.offset)
                        .min_by(f64::total_cmp)
                        .unwrap();
                    if direct <= 0.0 {
                        continue;
                    }
                    let expected = roof
                        .edges
                        .iter()
                        .map(|edge| edge.distance_squared(point))
                        .min_by(f64::total_cmp)
                        .unwrap()
                        .sqrt();
                    different += usize::from(direct.to_bits() != expected.to_bits());
                    max_error = max_error.max((direct - expected).abs());
                    max_ulps = max_ulps.max(direct.to_bits().abs_diff(expected.to_bits()));
                }
                eprintln!(
                    "convex {count} irregular={irregular} / 1M: {original:?} -> {accelerated:?}; direct normalized lines differ {different}/100000, max abs {max_error:e}m, max ulps {max_ulps}; fallback {fallback_queries}/100000"
                );
            }
        }
    }

    fn reference_roof_index_nearest(
        index: &RoofDistanceIndex,
        edges: &[RoofEdge],
        point: DVec2,
        node: usize,
        best: &mut f64,
    ) {
        if index.lower_bound(node, point) > *best {
            return;
        }
        let current = &index.nodes[node];
        if let Some([a, b]) = current.children {
            let [first, second] = if index.lower_bound(a, point) <= index.lower_bound(b, point) {
                [a, b]
            } else {
                [b, a]
            };
            reference_roof_index_nearest(index, edges, point, first, best);
            reference_roof_index_nearest(index, edges, point, second, best);
        } else {
            for &id in &index.order[current.edges.clone()] {
                *best = best.min(edges[id].distance_squared(point));
            }
        }
    }

    #[test]
    #[ignore = "timed roof index traversal benchmark; run without concurrent game workloads"]
    fn roof_index_bound_reuse_benchmark() {
        use std::{hint::black_box, time::Instant};
        for count in [24, 64, 192] {
            let rings = vec![
                (0..count)
                    .map(|i| {
                        let angle = i as f64 * std::f64::consts::TAU / count as f64;
                        let radius = if i % 4 < 2 { 32.0 } else { 25.0 };
                        [(angle.cos() * radius) as f32, (angle.sin() * radius) as f32]
                    })
                    .collect(),
                square(-3.0, 3.0),
            ];
            let roof = RoofField::new(&rings, Some(RoofShape::Hipped), 0.0, 20.0, 0.7);
            let index = roof.distance_index.as_ref().unwrap();
            let points: Vec<_> = (0..100_000)
                .map(|i| {
                    DVec2::new(
                        (i * 719 % 4093) as f64 / 53.0 - 40.0,
                        (i * 313 % 4091) as f64 / 51.0 - 40.0,
                    )
                })
                .collect();
            for &point in &points {
                let mut expected = f64::INFINITY;
                reference_roof_index_nearest(index, &roof.edges, point, 0, &mut expected);
                let mut actual = f64::INFINITY;
                index.nearest(&roof.edges, point, 0, &mut actual);
                assert_eq!(actual.to_bits(), expected.to_bits());
            }
            let mut original = vearth_platform::time::Duration::MAX;
            let mut accelerated = vearth_platform::time::Duration::MAX;
            for _ in 0..5 {
                let started = Instant::now();
                for &point in &points {
                    let mut best = f64::INFINITY;
                    reference_roof_index_nearest(
                        index,
                        &roof.edges,
                        black_box(point),
                        0,
                        &mut best,
                    );
                    black_box(best);
                }
                original = original.min(started.elapsed());
                let started = Instant::now();
                for &point in &points {
                    let mut best = f64::INFINITY;
                    index.nearest(&roof.edges, black_box(point), 0, &mut best);
                    black_box(best);
                }
                accelerated = accelerated.min(started.elapsed());
            }
            eprintln!(
                "concave {count} edges + courtyard / 100k exact queries: {original:?} -> {accelerated:?}"
            );
        }
    }

    #[test]
    fn prepared_polygon_scanlines_preserve_holes_crossings_and_boundaries() {
        let ring: Vec<_> = (0..256)
            .map(|i| {
                let angle = i as f64 * std::f64::consts::TAU / 256.0;
                let radius = if i % 7 < 3 { 20.0 } else { 24.0 };
                [(angle.cos() * radius) as f32, (angle.sin() * radius) as f32]
            })
            .collect();
        for rings in [
            vec![ring.clone()],
            vec![ring, square(-5.0, 5.0)],
            vec![vec![[0.0, 0.0], [15.0, 15.0], [0.0, 15.0], [15.0, 0.0]]],
        ] {
            let query = PolygonQuery::new(&rings);
            for i in 0..8192 {
                let p = DVec2::new(
                    (i * 719 % 4093) as f64 / 71.0 - 28.0,
                    (i * 313 % 4091) as f64 / 69.0 - 28.0,
                );
                assert_eq!(query.contains(p), contains(&rings, p), "{p:?}");
            }
            for edge in &query.edges {
                for p in [edge[0], edge[1], (edge[0] + edge[1]) * 0.5] {
                    for p in [p, p + DVec2::splat(1e-10), p - DVec2::splat(1e-10)] {
                        assert_eq!(query.contains(p), contains(&rings, p), "boundary {p:?}");
                    }
                }
            }
            if !query.rows.is_empty() {
                let candidates: usize = query.rows.iter().map(Vec::len).sum();
                assert!(candidates < query.edges.len() * query.rows.len() / 4);
                for row in 0..=32 {
                    let y = query.y_bounds[0]
                        + (query.y_bounds[1] - query.y_bounds[0]) * row as f64 / 32.0;
                    for y in [y.next_down(), y, y.next_up()] {
                        for x in [-30.0, -2.0, 0.0, 2.0, 30.0] {
                            let p = DVec2::new(x, y);
                            assert_eq!(query.contains(p), contains(&rings, p));
                        }
                    }
                }
            }
        }
    }

    #[test]
    #[ignore = "timed analytic-query benchmark; run without concurrent game workloads"]
    fn building_query_benchmark() {
        use std::{hint::black_box, time::Instant};
        let ring: Vec<_> = (0..192)
            .map(|i| {
                let angle = i as f64 * std::f64::consts::TAU / 192.0;
                let radius = 25.0 + if i % 4 < 2 { 7.0 } else { 0.0 };
                [(angle.cos() * radius) as f32, (angle.sin() * radius) as f32]
            })
            .collect();
        let rings = vec![ring, square(-3.0, 3.0)];
        let roof = RoofField::new(&rings, Some(RoofShape::Hipped), 60.0, 76.0, 0.7);
        let query = PolygonQuery::new(&rings);
        let points: Vec<_> = (0..100_000)
            .map(|i| {
                DVec2::new(
                    (i * 719 % 4093) as f64 / 53.0 - 40.0,
                    (i * 313 % 4091) as f64 / 51.0 - 40.0,
                )
            })
            .collect();
        let started = Instant::now();
        for &p in &points {
            black_box(reference_roof_height(black_box(&roof), black_box(p)));
        }
        let roof_reference = started.elapsed();
        let started = Instant::now();
        for &p in &points {
            black_box(roof.height(black_box(p)));
        }
        let roof_indexed = started.elapsed();
        let started = Instant::now();
        for &p in &points {
            black_box(contains(black_box(&rings), black_box(p)));
        }
        let polygon_reference = started.elapsed();
        let started = Instant::now();
        for &p in &points {
            black_box(query.contains(black_box(p)));
        }
        let polygon_indexed = started.elapsed();
        eprintln!(
            "100000 complex queries: roof {roof_reference:?} -> {roof_indexed:?}; polygon {polygon_reference:?} -> {polygon_indexed:?}"
        );
    }

    #[test]
    fn failed_refinement_reuses_samples_without_changing_fallback() {
        use std::cell::Cell;
        let rings = vec![square(-12.0, 12.0)];
        let domain = Footprint::new(vec![&rings], Vec::new()).unwrap();
        let cells = domain.roof_cells(4.0).unwrap();
        let boundary = domain.boundary();
        let actual_calls = Cell::new(0);
        let expected_calls = Cell::new(0);
        let corner = cells[0][0];
        let height = |p: DVec2| {
            if p.x + p.y * 0.037 < 0.123 {
                100.0
            } else {
                105.0
            }
        };
        let actual = roof_geometry(
            cells.iter().copied(),
            &boundary,
            &|p| {
                if p == corner {
                    actual_calls.set(actual_calls.get() + 1);
                }
                height(p)
            },
            true,
        )
        .unwrap();
        let expected = reference_roof_geometry(
            cells.iter().copied(),
            &boundary,
            &|p| {
                if p == corner {
                    expected_calls.set(expected_calls.get() + 1);
                }
                height(p)
            },
            true,
        )
        .unwrap();
        let fallback = roof_geometry(cells.iter().copied(), &boundary, &height, false).unwrap();
        assert_same_roof_geometry(&actual, &expected);
        assert_same_roof_geometry(&actual, &fallback);
        assert!(actual_calls.get() < expected_calls.get());
    }

    fn reference_clear_rectangle(
        context: &FacadeContext,
        corners: [DVec2; 4],
        low: f64,
        high: f64,
    ) -> bool {
        let overlaps = |rings: &Rings| {
            if corners.iter().any(|&p| contains(rings, p)) {
                return true;
            }
            let inside = |p: DVec2| {
                let sides: [f64; 4] = std::array::from_fn(|i| {
                    (corners[(i + 1) % 4] - corners[i]).perp_dot(p - corners[i])
                });
                sides.iter().all(|&s| s >= -1.0e-8) || sides.iter().all(|&s| s <= 1.0e-8)
            };
            rings.iter().any(|ring| {
                (0..ring.len()).any(|i| {
                    let a = DVec2::from_array(ring[i].map(f64::from));
                    let b = DVec2::from_array(ring[(i + 1) % ring.len()].map(f64::from));
                    inside(a)
                        || (0..4).any(|j| {
                            intersection([a, b], [corners[j], corners[(j + 1) % 4]]).is_some()
                        })
                })
            })
        };
        !overlaps(&context.owner)
            && !context.blockers.iter().any(|blocker| {
                blocker.bottom < high && blocker.top > low && overlaps(&blocker.rings)
            })
    }
    fn reference_opening_outline(x: [f64; 2], y: [f64; 2], shape: OpeningShape) -> Vec<DVec2> {
        let mut points = vec![DVec2::new(x[0], y[0]), DVec2::new(x[1], y[0])];
        if shape == OpeningShape::Rectangular {
            points.extend([DVec2::new(x[1], y[1]), DVec2::new(x[0], y[1])]);
            return points;
        }
        let half = (x[1] - x[0]) * 0.5;
        let center = (x[0] + x[1]) * 0.5;
        let rise = match shape {
            OpeningShape::SegmentalArch => half * 0.38,
            OpeningShape::RoundArch => half,
            OpeningShape::PointedArch => half * 1.25,
            OpeningShape::Rectangular => unreachable!(),
        }
        .min((y[1] - y[0]) * 0.50);
        let spring = y[1] - rise;
        // Six convex crown facets retain the opening's silhouette without tiny
        // tessellation at world scale. Both sides share exact springing points.
        for i in 0..=6 {
            let t = i as f64 / 6.0;
            let q = 1.0 - (2.0 * t - 1.0).abs();
            let lift = if shape == OpeningShape::PointedArch {
                q.powf(0.78)
            } else {
                (std::f64::consts::PI * t).sin()
            };
            points.push(DVec2::new(
                center + half * (std::f64::consts::PI * t).cos(),
                spring + rise * lift,
            ));
        }
        points
    }
    #[test]
    fn facade_clearance_broadphase_matches_exact_polygon_checks() {
        let owner = vec![
            vec![
                [0.0, 0.0],
                [30.0, 0.0],
                [30.0, 14.0],
                [16.0, 14.0],
                [16.0, 28.0],
                [0.0, 28.0],
            ],
            square(5.0, 9.0),
        ];
        let obstacle = vec![vec![
            [-4.0, -5.0],
            [34.0, -5.0],
            [34.0, -4.999],
            [-4.0, -4.999],
        ]];
        let context = FacadeContext {
            party_walls: Vec::new(),
            opaque_core: None,
            shadow_walls: std::cell::RefCell::default(),
            owner: owner.clone(),
            owner_bounds: bounds(&owner),
            owner_query: PolygonQuery::new(&owner),
            entrance: None,
            blockers: vec![FacadeBlocker {
                rings: obstacle.clone(),
                bounds: bounds(&obstacle),
                query: PolygonQuery::new(&obstacle),
                bottom: 2.0,
                top: 20.0,
            }],
        };
        for i in 0..4096 {
            let a = (i * 977 % 1051) as f64 * 0.013;
            let tangent = DVec2::new(a.cos(), a.sin());
            let normal = DVec2::new(-tangent.y, tangent.x);
            let origin = DVec2::new((i * 331 % 83) as f64 - 20.0, (i * 347 % 61) as f64 - 20.0);
            let width = (i % 19) as f64 * 0.71;
            let depth = (i % 7) as f64 * 0.26;
            let corners = [
                origin,
                origin + tangent * width,
                origin + tangent * width + normal * depth,
                origin + normal * depth,
            ];
            let low = (i % 23) as f64;
            assert_eq!(
                context.clear_rectangle(corners, low, low + 4.0),
                reference_clear_rectangle(&context, corners, low, low + 4.0),
                "rectangle {i}"
            );
        }
        for x in [0.0, 5.0, 9.0, 16.0, 30.0] {
            for y in [-5.0, -4.999, 0.0, 5.0, 9.0, 14.0, 28.0] {
                for epsilon in [-1.0e-7, -1.0e-9, 0.0, 1.0e-9, 1.0e-7] {
                    for width in [1.0e-10, 0.001, 0.03, 1.0, 200.0] {
                        let a = DVec2::new(x + epsilon, y + epsilon);
                        let corners = [
                            a,
                            a + DVec2::new(width, 0.0),
                            a + DVec2::new(width, 0.08),
                            a + DVec2::new(0.0, 0.08),
                        ];
                        assert_eq!(
                            context.clear_rectangle(corners, 0.0, 15.0),
                            reference_clear_rectangle(&context, corners, 0.0, 15.0)
                        );
                    }
                }
            }
        }
    }

    fn complex_clearance_fixture() -> Rings {
        vec![
            (0..256)
                .map(|i| {
                    let a = i as f64 * std::f64::consts::TAU / 256.0;
                    let radius = if i % 7 < 3 { 40.0 } else { 44.0 };
                    [(a.cos() * radius) as f32, (a.sin() * radius) as f32]
                })
                .collect(),
            square(-6.0, 6.0),
        ]
    }

    #[test]
    fn facade_clearance_bins_preserve_complex_holes_edges_and_degenerate_queries() {
        let owner = complex_clearance_fixture();
        let barrier = vec![vec![
            [-45.0, -45.0],
            [45.0, -45.0],
            [45.0, -44.999],
            [-45.0, -44.999],
        ]];
        let context = FacadeContext {
            owner: owner.clone(),
            owner_bounds: bounds(&owner),
            owner_query: PolygonQuery::new(&owner),
            entrance: None,
            blockers: vec![FacadeBlocker {
                rings: barrier.clone(),
                bounds: bounds(&barrier),
                query: PolygonQuery::new(&barrier),
                bottom: 2.0,
                top: 20.0,
            }],
            party_walls: Vec::new(),
            opaque_core: None,
            shadow_walls: std::cell::RefCell::default(),
        };
        assert!(!context.owner_query.rows.is_empty());
        let check = |corners, low, high| {
            let expected = reference_clear_rectangle(&context, corners, low, high);
            assert_eq!(
                context.clear_rectangle_indexed::<false>(corners, low, high),
                expected
            );
            assert_eq!(
                context.clear_rectangle(corners, low, high),
                expected,
                "{corners:?}"
            );
        };
        for i in 0..8192 {
            let a = (i * 977 % 1051) as f64 * 0.013;
            let tangent = DVec2::new(a.cos(), a.sin());
            let normal = DVec2::new(-tangent.y, tangent.x);
            let origin = DVec2::new((i * 331 % 163) as f64 - 80.0, (i * 347 % 151) as f64 - 75.0);
            let width = (i % 23) as f64 * 0.71;
            let depth = (i % 7) as f64 * 0.26;
            check(
                [
                    origin,
                    origin + tangent * width,
                    origin + tangent * width + normal * depth,
                    origin + normal * depth,
                ],
                (i % 23) as f64,
                (i % 23) as f64 + 4.0,
            );
        }
        let query = &context.owner_query;
        for edge in &query.edges {
            for center in [edge[0], edge[1], (edge[0] + edge[1]) * 0.5] {
                for width in [0.0, 1e-10, 0.03, 1.0, 200.0] {
                    check(
                        [
                            center,
                            center + DVec2::new(width, 0.0),
                            center + DVec2::new(width, 0.08),
                            center + DVec2::new(0.0, 0.08),
                        ],
                        0.0,
                        15.0,
                    );
                }
            }
        }
        for row in 0..=32 {
            let y = query.y_bounds[0] + (query.y_bounds[1] - query.y_bounds[0]) * row as f64 / 32.0;
            for y in [y.next_down(), y, y.next_up()] {
                for x in [-44.0, -6.0, 0.0, 6.0, 44.0] {
                    let p = DVec2::new(x, y);
                    check(
                        [
                            p,
                            p + DVec2::new(0.01, 0.0),
                            p + DVec2::new(0.01, 0.001),
                            p + DVec2::new(0.0, 0.001),
                        ],
                        0.0,
                        15.0,
                    );
                }
            }
        }
        let mut candidates = 0;
        query.any_edge_in_bounds((DVec2::new(-45.0, -0.01), DVec2::new(45.0, 0.01)), |_| {
            candidates += 1;
            false
        });
        assert!(candidates < query.edges.len() / 4);
    }

    #[test]
    #[ignore = "timed exact facade clearance benchmark; run without concurrent game workloads"]
    fn facade_clearance_bins_benchmark() {
        use std::{hint::black_box, time::Instant};
        let owner = complex_clearance_fixture();
        let context = FacadeContext::from_sources(&owner, &[], &[], 0.0);
        let queries: Vec<_> = (0..20_000)
            .map(|i| {
                let edge = context.owner_query.edges[i % 256];
                let direction = (edge[1] - edge[0]).normalize();
                let outward = DVec2::new(direction.y, -direction.x);
                let a = edge[0].lerp(edge[1], 0.25 + (i % 7) as f64 / 20.0) + outward * 0.025;
                let b = a + direction * edge[0].distance(edge[1]) * 0.2;
                [a, b, b + outward * 0.25, a + outward * 0.25]
            })
            .collect();
        let mut times = [vearth_platform::time::Duration::MAX; 2];
        for round in 0..7 {
            for mode in [round % 2, 1 - round % 2] {
                let started = Instant::now();
                for &query in &queries {
                    black_box(if mode == 0 {
                        context.clear_rectangle_indexed::<false>(black_box(query), 0.0, 10.0)
                    } else {
                        context.clear_rectangle(black_box(query), 0.0, 10.0)
                    });
                }
                times[mode] = times[mode].min(started.elapsed());
            }
        }
        eprintln!("complex facade20k: {:?} -> {:?}", times[0], times[1]);
    }

    #[test]
    fn shared_facade_corners_preserve_every_vertex_and_index() {
        let frame = frame(20);
        let (lat, lon) = frame.column_latlon(0.5, 0.5);
        let projection = Projection {
            frame: &frame,
            lat,
            lon,
        };
        let plane = FacadePlane {
            shadow_units: None,
            projection: &projection,
            origin: DVec2::new(-5.0, -6.0),
            direction: DVec2::new(0.8, 0.6),
            outward: DVec2::new(0.6, -0.8),
        };
        let glass = SurfaceAppearance {
            material: crate::material::WINDOW,
            tint: [81, 102, 116],
        };
        let trim = SurfaceAppearance {
            material: crate::material::WALL,
            tint: [168, 151, 121],
        };
        let mut actual = MeshBuffers::default();
        let mut expected = MeshBuffers::default();
        for shape in [
            OpeningShape::Rectangular,
            OpeningShape::SegmentalArch,
            OpeningShape::RoundArch,
            OpeningShape::PointedArch,
        ] {
            for relief in [false, true] {
                for i in 0..17 {
                    let x = [i as f64 * 2.27, i as f64 * 2.27 + 1.11];
                    let y = [103.1, 105.9];
                    let depth = 0.025;
                    let frame = 0.11;
                    plane.opening(&mut actual, x, y, depth, shape, frame, glass, trim, relief);
                    let inner = reference_opening_outline(x, y, shape);
                    let outer = reference_opening_outline(
                        [x[0] - frame, x[1] + frame],
                        [y[0] - frame, y[1] + frame],
                        shape,
                    );
                    let mut polygon = |points: &[DVec2], depth, material| {
                        for i in 1..points.len() - 1 {
                            reference_emit_triangle(
                                &mut expected,
                                [points[0], points[i], points[i + 1]]
                                    .map(|p| plane.point(p.x, p.y, depth)),
                                material,
                                false,
                            );
                        }
                    };
                    polygon(&inner, depth, glass);
                    if relief {
                        for j in 0..inner.len() {
                            let next = (j + 1) % inner.len();
                            let q = [outer[j], outer[next], inner[next], inner[j]]
                                .map(|p| plane.point(p.x, p.y, depth + 0.05));
                            reference_emit_triangle(&mut expected, [q[0], q[1], q[2]], trim, false);
                            reference_emit_triangle(&mut expected, [q[0], q[2], q[3]], trim, false);
                        }
                    } else {
                        polygon(&outer, depth - 0.008, trim);
                    }
                }
            }
        }
        for i in 0..64 {
            let x = [i as f64 * 0.7 - 20.0, i as f64 * 0.7 - 18.8];
            let y = [102.0, 104.0];
            let depth = [0.025, 0.6];
            plane.structural_strip(&mut actual, x, y, depth, trim);
            plane.quad(&mut expected, x, y, depth[1], trim);
            let corners = [(x[0], y[0]), (x[1], y[0]), (x[1], y[1]), (x[0], y[1])];
            let side = SurfaceAppearance {
                tint: trim.tint.map(|v| (f32::from(v) * 0.82) as u8),
                ..trim
            };
            for edge in 0..4 {
                let a = corners[edge];
                let b = corners[(edge + 1) % 4];
                let q = [
                    plane.point(a.0, a.1, depth[0]),
                    plane.point(b.0, b.1, depth[0]),
                    plane.point(b.0, b.1, depth[1]),
                    plane.point(a.0, a.1, depth[1]),
                ];
                reference_emit_triangle(&mut expected, [q[0], q[1], q[2]], side, false);
                reference_emit_triangle(&mut expected, [q[0], q[2], q[3]], side, false);
            }
        }
        assert_eq!(actual.vertices, expected.vertices);
        assert_eq!(actual.indices, expected.indices);
    }

    #[test]
    #[ignore = "paired CPU benchmark; run explicitly without a concurrent renderer"]
    fn roof_gradient_probe_benchmark() {
        use vearth_platform::time::Instant;
        let cases = [
            (
                "dutch-townhouse",
                vec![vec![[-5.0, -12.0], [5.0, -11.8], [5.1, 12.0], [-5.0, 12.1]]],
                RoofShape::Gabled,
            ),
            (
                "long-skewed-gable",
                vec![vec![[0.0, 0.0], [70.0, 0.2], [70.3, 22.0], [0.0, 22.1]]],
                RoofShape::Gabled,
            ),
            (
                "mansard",
                vec![vec![[-6.0, -12.0], [6.0, -11.8], [6.1, 12.0], [-6.0, 12.1]]],
                RoofShape::Mansard,
            ),
            (
                "courtyard-hip",
                vec![square(-22.0, 22.0), square(-7.0, 7.0)],
                RoofShape::Hipped,
            ),
            (
                "many-edge-hip",
                vec![
                    (0..32)
                        .map(|i| {
                            let angle = i as f32 * std::f32::consts::TAU / 32.0;
                            [angle.cos() * 20.0, angle.sin() * 16.0]
                        })
                        .collect(),
                ],
                RoofShape::Hipped,
            ),
        ];
        fn run<const CACHE: bool>(
            cells: &[[DVec2; 4]],
            boundary: &[[DVec2; 2]],
            at: &dyn Fn(DVec2) -> f64,
        ) -> (RoofGeometry, usize) {
            let mut sampler = RoofSampler::<CACHE> {
                stats: None,
                failure: 0,
                height_bounds: None,
                at,
                interior_error: ROOF_PLANE_ERROR_M,
                boundary_error: ROOF_PLANE_ERROR_M,
                cache: rustc_hash::FxHashMap::default(),
            };
            let mesh = roof_geometry_sampled(cells.to_vec(), boundary, &mut sampler, true).unwrap();
            (mesh, sampler.cache.len())
        }
        for (name, rings, shape) in cases {
            let domain = Footprint::new(vec![&rings], Vec::new()).unwrap();
            let boundary = domain.boundary();
            let roof = RoofField::new(&rings, Some(shape), 100.0, 105.0, 0.7);
            let rotated: Rings = rings
                .iter()
                .map(|ring| {
                    ring.iter()
                        .map(|p| {
                            roof.rotate(DVec2::from_array(p.map(f64::from)))
                                .as_vec2()
                                .to_array()
                        })
                        .collect()
                })
                .collect();
            let cells: Vec<_> = Footprint::new(vec![&rotated], Vec::new())
                .unwrap()
                .roof_cells(3.0)
                .unwrap()
                .into_iter()
                .map(|cell| cell.map(|p| roof.unrotate(p)))
                .collect();
            let height = |p| roof.height(p);
            let (cached, cached_entries) = run::<true>(&cells, &boundary, &height);
            let (direct, direct_entries) = run::<false>(&cells, &boundary, &height);
            assert_same_roof_geometry(&direct, &cached);
            let mut times = [Vec::new(), Vec::new()];
            for round in 0..7 {
                for mode in [round % 2, 1 - round % 2] {
                    let start = Instant::now();
                    for _ in 0..12 {
                        if mode == 0 {
                            std::hint::black_box(run::<true>(&cells, &boundary, &height));
                        } else {
                            std::hint::black_box(run::<false>(&cells, &boundary, &height));
                        }
                    }
                    times[mode].push(start.elapsed().as_secs_f64() * 1000.0 / 12.0);
                }
            }
            for t in &mut times {
                t.sort_by(f64::total_cmp);
            }
            eprintln!(
                "{name}: cached {:.3}ms direct {:.3}ms speedup {:.2}x cache entries {cached_entries}->{direct_entries}, triangles {}",
                times[0][3],
                times[1][3],
                times[0][3] / times[1][3],
                direct.triangles.len()
            );
        }
    }
}
