//! Source samples and feature geometry produce the world surfaces directly.
use super::*;
use crate::collision::{SurfaceCollision, triangles_from_mesh};
use vearth_core::mesh::MeshBuffers;

fn track_sample<T>(
    sample: vearth_data::SourceSample<T>,
    complete: &std::sync::atomic::AtomicBool,
) -> Option<T> {
    if !sample.complete {
        complete.store(false, std::sync::atomic::Ordering::Relaxed);
    }
    sample.value
}

const COARSE_HEIGHT_MEMO_LIMIT: usize = 8_192;

fn rounded_geometry_height(frame: &ChunkFrame, position: [f32; 3]) -> Option<(f32, f32)> {
    let p = DVec3::from_array(position.map(f64::from));
    let (_, _, h) = vearth_core::ellipsoid::ecef_to_geodetic(frame.local_to_ecef(p));
    if !h.is_finite() {
        return None;
    }
    let rounded = h as f32;
    let low = if f64::from(rounded) > h {
        rounded.next_down()
    } else {
        rounded
    };
    let high = if f64::from(rounded) < h {
        rounded.next_up()
    } else {
        rounded
    };
    Some((low, high))
}

fn include_coarse_geometry_height_range(
    mut range: (f32, f32),
    mesh: &MeshBuffers,
    mut height: impl FnMut([f32; 3]) -> Option<(f32, f32)>,
) -> (f32, f32) {
    // Flat-shaded triangles repeat positions with different appearance. The
    // inverse depends only on their exact stored bits and this batch's frame.
    // Keep every bounds fold in its original order, including signed zeros.
    let mut memo = rustc_hash::FxHashMap::with_capacity_and_hasher(
        mesh.vertices.len().min(N * N),
        Default::default(),
    );
    for vertex in &mesh.vertices {
        let key = vertex.pos.map(f32::to_bits);
        let bounds = if let Some(bounds) = memo.get(&key) {
            *bounds
        } else {
            let bounds = height(vertex.pos);
            if memo.len() < COARSE_HEIGHT_MEMO_LIMIT {
                memo.insert(key, bounds);
            }
            bounds
        };
        if let Some((low, high)) = bounds {
            range.0 = range.0.min(low);
            range.1 = range.1.max(high);
        }
    }
    range
}

fn include_geometry_height_range(
    mut range: (f32, f32),
    frame: &ChunkFrame,
    mesh: &MeshBuffers,
) -> (f32, f32) {
    if frame.addr.cell.level < tier0::BUILDINGS_MIN_LEVEL {
        // A curved globe mesh's local box combines unrelated extrema: its
        // corners can sit tens of kilometres above every actual vertex.
        // Physical LOD heights must not inherit those synthetic elevations.
        // Cache exact positions only within this batch; uncached positions
        // beyond the fixed entry budget still use the full inverse.
        return include_coarse_geometry_height_range(range, mesh, |position| {
            rounded_geometry_height(frame, position)
        });
    }
    let mut low = DVec3::splat(f64::INFINITY);
    let mut high = DVec3::splat(f64::NEG_INFINITY);
    for vertex in &mesh.vertices {
        let p = DVec3::from_array(vertex.pos.map(f64::from));
        low = low.min(p);
        high = high.max(p);
    }
    if !low.is_finite() {
        return range;
    }
    for i in 0..8 {
        let p = DVec3::new(
            if i & 1 == 0 { low.x } else { high.x },
            if i & 2 == 0 { low.y } else { high.y },
            if i & 4 == 0 { low.z } else { high.z },
        );
        let (_, _, h) = vearth_core::ellipsoid::ecef_to_geodetic(frame.local_to_ecef(p));
        range.0 = range.0.min(h as f32);
        range.1 = range.1.max(h as f32);
    }
    range
}

fn terrain_relief_m(source: &TerrainSurfaceData) -> f32 {
    let mut low = f32::INFINITY;
    let mut high = f32::NEG_INFINITY;
    for z in 0..=N {
        for x in 0..=N {
            let incident = (z.saturating_sub(1)..=z.min(N - 1)).any(|cz| {
                (x.saturating_sub(1)..=x.min(N - 1))
                    .any(|cx| source.source_columns[cz] & (1 << cx) != 0)
            });
            let height = source.heights_m[z * (N + 1) + x];
            if incident && height.is_finite() {
                low = low.min(height);
                high = high.max(height);
            }
        }
    }
    if !low.is_finite() {
        return 0.0;
    }
    // Preserve finite metadata even for two finite source extrema whose
    // difference exceeds f32. Such relief already saturates LOD preference.
    (f64::from(high) - f64::from(low)).min(f64::from(f32::MAX)) as f32
}

struct SampledElevation<'a> {
    source: &'a dyn ElevationSource,
    complete: &'a std::sync::atomic::AtomicBool,
}

impl ElevationSource for SampledElevation<'_> {
    fn elevation_m(&self, lat: f64, lon: f64, pitch: f64) -> f32 {
        self.elevation_opt(lat, lon, pitch).unwrap_or(0.0)
    }

    fn elevation_opt(&self, lat: f64, lon: f64, pitch: f64) -> Option<f32> {
        track_sample(self.source.sample_elevation(lat, lon, pitch), self.complete)
    }
}

struct SampledColor<'a> {
    source: &'a dyn ColorSource,
    complete: &'a std::sync::atomic::AtomicBool,
}

impl ColorSource for SampledColor<'_> {
    fn month(&self) -> Option<u8> {
        self.source.month()
    }

    fn color(&self, lat: f64, lon: f64) -> Option<[u8; 3]> {
        track_sample(self.source.sample_color(lat, lon), self.complete)
    }

    fn color_detail(&self, lat: f64, lon: f64, pitch: f64) -> Option<[u8; 3]> {
        track_sample(
            self.source.sample_color_detail(lat, lon, pitch),
            self.complete,
        )
    }

    fn color_hr(&self, lat: f64, lon: f64, pitch: f64) -> Option<[u8; 3]> {
        track_sample(self.source.sample_color_hr(lat, lon, pitch), self.complete)
    }
}

pub fn generate_surface_cell(cell: CellId, sources: &Sources) -> SurfaceCell {
    generate(cell, sources, None)
}

/// Reuse the raw vector query acquired for this cell's source preparation.
/// `features` must cover `vector_query_bounds(cell)` with the same source
/// authority as `sources.vectors`; landmark ownership is still resolved here.
pub fn generate_surface_cell_with_features(
    cell: CellId,
    sources: &Sources,
    features: vearth_data::vectors::FeaturePack,
) -> SurfaceCell {
    generate(cell, sources, Some(features))
}

fn generate(
    cell: CellId,
    sources: &Sources,
    features: Option<vearth_data::vectors::FeaturePack>,
) -> SurfaceCell {
    let mut generation_profile = contracts::GenerationStageTimer::start();
    let frame0 = ChunkFrame::of(ChunkAddr { cell, k: 0 });
    let voxel = frame0.grid_scale;

    // Every sampled provider reports its own dependency status. A failure in
    // another cell must not poison this job, and an already tombstoned missing
    // dependency must not pass merely because its global counter is unchanged.
    let elevation_complete = std::sync::atomic::AtomicBool::new(true);
    let surface_complete = std::sync::atomic::AtomicBool::new(true);
    let imagery_complete = std::sync::atomic::AtomicBool::new(true);
    let elevation = SampledElevation {
        source: sources.elevation,
        complete: &elevation_complete,
    };
    let surface = sources.surface.map(|source| SampledElevation {
        source,
        complete: &surface_complete,
    });
    let imagery = sources.imagery.map(|source| SampledColor {
        source,
        complete: &imagery_complete,
    });
    let sources = &Sources {
        elevation: &elevation,
        surface: surface
            .as_ref()
            .map(|source| source as &dyn ElevationSource),
        imagery: imagery.as_ref().map(|source| source as &dyn ColorSource),
        vectors: sources.vectors,
    };

    // Resolve curated replacement footprints before loading vector geometry.
    // This lets the source pack discard ordinary landmark buildings once,
    // before any downstream extrusion, LiDAR massing, roofs, façades or night
    // lights can be derived from them.
    let lms_active = landmarks::active_for(cell, &frame0, &|lat, lon| {
        sources
            .elevation
            .elevation_m(lat, lon, voxel.y * SAMPLE_ZOOM_FACTOR)
    });

    let great_wall = crate::great_wall::Plan::for_frame(&frame0);

    // Vector features come first: the water mask decides whether terrain
    // sampling needs the margin ring for the flatten windows.
    let mut complete = true;
    let pack = match (sources.vectors, cell.level >= tier0::WATER_MIN_LEVEL) {
        (Some(vectors), true) => {
            let (lat0, lon0, half_e, half_n) = vector_query_bounds(cell);
            let mut pack = features.unwrap_or_else(|| vectors.features(lat0, lon0, half_e, half_n));
            complete = pack.complete;
            generation_profile.record_incomplete(0, cell.level, pack.complete);
            if lms_active.iter().any(|active| active.lm.suppress.buildings) {
                pack.buildings.retain(|building| {
                    !lms_active.iter().any(|active| {
                        active.replaces_source_building(building.seed, &building.rings)
                    })
                });
            }
            if !great_wall.is_empty() {
                pack.buildings.retain(|building| {
                    !building
                        .location_rad
                        .is_some_and(|[lat, lon]| great_wall.replaces_building(lat, lon))
                });
            }
            Some(pack)
        }
        _ => None,
    };
    // Night eligibility comes from the exact feature set that creates the
    // visible geometry. A separate zoom authority can disagree with it and is
    // therefore unsuitable for deciding whether a visible tower may light up.
    let night_eligible_ids: HashSet<u64> = pack
        .as_ref()
        .into_iter()
        .flat_map(|pack| &pack.buildings)
        .filter(|building| building.height_m - building.min_height_m >= NIGHT_LIGHT_MIN_HEIGHT_M)
        .map(|building| building.seed)
        .collect();
    // Water flattening, the cliff support stencil, and exact-tree roots may
    // read outside the interior grid. Allocate their bounded elevation margin
    // before exposing the indexed sampling closure.
    let margin = pack
        .as_ref()
        .map_or(1, |pack| {
            elevation_margin_for_pack(pack, &frame0, cell.level)
        })
        .max(1);

    generation_profile.checkpoint(0, cell.level);
    // Sample terrain heights on the (margin-expanded) column-corner grid of
    // the shared cell uv lattice. `uv_at` extrapolates the lattice exactly
    // (affine over dyadic bounds), so margin corners land on the identical
    // geographic points a neighboring cell samples for its own interior.
    let m = margin as i64;
    let nc = N + 2 * margin + 1; // corners per expanded edge
    let mut corner_h = vec![0f32; nc * nc];
    for gz in 0..nc {
        for gx in 0..nc {
            let (fx, fz) = (
                (gx as i64 - m) as f64 / N as f64,
                (gz as i64 - m) as f64 / N as f64,
            );
            let (lat, lon) = frame0.column_latlon(fx, fz);
            corner_h[gz * nc + gx] =
                sources
                    .elevation
                    .elevation_m(lat, lon, voxel.y * SAMPLE_ZOOM_FACTOR);
        }
    }
    if !lms_active.is_empty() {
        for gz in 0..nc {
            for gx in 0..nc {
                let fx = (gx as i64 - m) as f64 / N as f64;
                let fz = (gz as i64 - m) as f64 / N as f64;
                let (lat, lon) = frame0.column_latlon(fx, fz);
                let point = vearth_core::ellipsoid::geodetic_to_ecef(lat, lon, 0.0);
                for active in &lms_active {
                    if let Some((mx, _, mz)) = active.column_ecef(point, 0.0) {
                        if let Some((height, _)) =
                            active.flattened_terrain(mx, mz, corner_h[gz * nc + gx])
                        {
                            corner_h[gz * nc + gx] = height;
                        }
                    }
                }
            }
        }
    }
    // Local surface offset (curvature correction), interior corners only.
    let mut corner_off = [[0f32; N + 1]; N + 1];
    for gz in 0..=N {
        for gx in 0..=N {
            let (fx, fz) = (gx as f64 / N as f64, gz as f64 / N as f64);
            let (_, y_off) = frame0.column_surface(fx, fz);
            corner_off[gz][gx] = y_off as f32;
        }
    }
    // Geodetic per-column tops over the expanded grid (water flatten windows
    // read these; frame-independent, so levels match across cell borders).
    let nn = N + 2 * margin;
    let mut tops_geo = vec![0f32; nn * nn];
    for z in 0..nn {
        for x in 0..nn {
            tops_geo[z * nn + x] = 0.25
                * (corner_h[z * nc + x]
                    + corner_h[z * nc + x + 1]
                    + corner_h[(z + 1) * nc + x]
                    + corner_h[(z + 1) * nc + x + 1]);
        }
    }

    // Per-column terrain top, climate parameters, slope, and effective top.
    // Geodetic elevation stays invariant across LODs. Per-level vertical
    // exaggeration made adjacent cuts place the same mountain hundreds of
    // metres apart, exposing their otherwise-hidden cell-edge walls.
    // Tint jitter reads as speckle once voxels are ~kilometers; fade it.
    let jitter_scale = (100.0 / voxel.y).clamp(0.05, 1.0) as f32;
    let imagery_month = sources.imagery.and_then(|source| source.month());
    let mut cols = [[Col::EMPTY; N]; N];
    let mut height_range = (f32::INFINITY, f32::NEG_INFINITY);
    for z in 0..N {
        for x in 0..N {
            let (cz, cx) = (z + margin, x + margin);
            let h = tops_geo[cz * nn + cx];
            let off = 0.25
                * (corner_off[z][x]
                    + corner_off[z][x + 1]
                    + corner_off[z + 1][x]
                    + corner_off[z + 1][x + 1]);
            let dx = (corner_h[cz * nc + cx + 1] - corner_h[cz * nc + cx])
                .abs()
                .max((corner_h[(cz + 1) * nc + cx + 1] - corner_h[(cz + 1) * nc + cx]).abs());
            let dz = (corner_h[(cz + 1) * nc + cx] - corner_h[cz * nc + cx])
                .abs()
                .max((corner_h[(cz + 1) * nc + cx + 1] - corner_h[cz * nc + cx + 1]).abs());
            let mut local_min = f32::INFINITY;
            let mut local_max = f32::NEG_INFINITY;
            for rz in cz.saturating_sub(2)..=(cz + 2).min(nn - 1) {
                for rx in cx.saturating_sub(2)..=(cx + 2).min(nn - 1) {
                    let nearby = tops_geo[rz * nn + rx];
                    local_min = local_min.min(nearby);
                    local_max = local_max.max(nearby);
                }
            }
            let (lat, lon) =
                frame0.column_latlon((x as f64 + 0.5) / N as f64, (z as f64 + 0.5) / N as f64);
            let climate = climate_at(lat, lon, h);
            let col = Col {
                top: off + h,
                sea: if h < 0.0 { off } else { f32::NEG_INFINITY },
                elev: h,
                bathymetry: 0.0,
                landscape_variation: art_direction::landscape_variation(
                    lat.to_degrees(),
                    lon.to_degrees(),
                ),
                slope: (dx.max(dz) as f64 / voxel.x.max(1e-9)).atan() as f32,
                relief: local_max - local_min,
                climate,
                regional: crate::regional_landscape::resolve(lat, lon, h, climate, imagery_month),
                img: sources.imagery.and_then(|src| src.color(lat, lon)),
                img_detail: sources
                    .imagery
                    .and_then(|src| src.color_detail(lat, lon, voxel.y * SAMPLE_ZOOM_FACTOR)),
                img_hr: sources
                    .imagery
                    .and_then(|src| src.color_hr(lat, lon, voxel.y * SAMPLE_ZOOM_FACTOR)),
            };
            height_range = (height_range.0.min(h), height_range.1.max(h));
            cols[z][x] = col;
        }
    }

    // Some global elevation products contain landmark roofs and monuments
    // even though this channel is used as bare-earth terrain. Resolve clean
    // perimeter levels once in the landmark's canonical frame, then replace
    // only opted-in architectural footprints with a feathered level pad.
    // The exact same target later anchors the landmark mesh, preventing the DEM
    // pyramid/building from being rendered underneath a second copy.
    let mut flattened_terrain = false;
    if !lms_active.is_empty() {
        for z in 0..N {
            for x in 0..N {
                let point = frame0.local_to_ecef(DVec3::new(x as f64 + 0.5, 0.0, z as f64 + 0.5));
                for active in &lms_active {
                    let Some((mx, _, mz)) = active.column_ecef(point, voxel.x * 0.5) else {
                        continue;
                    };
                    let col = &mut cols[z][x];
                    if let Some((corrected, weight)) = active.flattened_terrain(mx, mz, col.elev) {
                        let delta = corrected - col.elev;
                        col.elev = corrected;
                        col.top += delta;
                        col.slope *= (1.0 - weight) as f32;
                        let (lat, lon) = frame0.column_latlon(
                            (x as f64 + 0.5) / N as f64,
                            (z as f64 + 0.5) / N as f64,
                        );
                        col.climate = climate_at(lat, lon, corrected);
                        col.regional = regional_landscape::resolve(
                            lat,
                            lon,
                            corrected,
                            col.climate,
                            imagery_month,
                        );
                        flattened_terrain = true;
                    }
                }
            }
        }
    }
    if flattened_terrain {
        height_range = (f32::INFINITY, f32::NEG_INFINITY);
        for col in cols.iter().flatten() {
            height_range = (height_range.0.min(col.elev), height_range.1.max(col.elev));
        }
    }

    // Grade road columns toward the smoothed
    // along-way DEM profile so roads climb hillsides instead of
    // stairstepping. Geodetic targets convert through the same off+h mapping
    // as terrain; `sea` stays derived from the raw DEM so grading
    // can never conjure ocean.
    let elev_en = {
        let (lat0, lon0) = frame0.column_latlon(0.5, 0.5);
        move |e: f64, n: f64| {
            let lat = lat0 + n / vearth_core::ellipsoid::MEAN_RADIUS;
            let lon = lon0 + e / (vearth_core::ellipsoid::MEAN_RADIUS * lat0.cos());
            sources
                .elevation
                .elevation_m(lat, lon, voxel.y * SAMPLE_ZOOM_FACTOR)
        }
    };
    let bridge_elev_en = {
        let (lat, lon) = frame0.column_latlon(0.5, 0.5);
        move |east: f64, north: f64| {
            let sample_lat = lat + north / vearth_core::ellipsoid::MEAN_RADIUS;
            let sample_lon = lon + east / (vearth_core::ellipsoid::MEAN_RADIUS * lat.cos());
            let raw = sources.elevation.elevation_m(
                sample_lat,
                sample_lon,
                crate::datum_pitch_m(sample_lat),
            );
            raw
        }
    };
    let mut road_surface_corners = None;
    if let Some(pack) = &pack
        && let Some(mut grades) = tier0::road_surface_grades(pack, &frame0, cell.level, &elev_en)
    {
        // Apply architectural ground corrections after along-way smoothing.
        // Otherwise the raw DEM road profile can restore roofs over cleared
        // landmark pads. Evaluate the shared geographic lattice so the same
        // correction reaches roads, terrain, and neighboring cell borders.
        let correct_target = |fx: f64, fz: f64, target: &mut (f32, f32)| {
            if target.0 == 0.0 {
                return;
            }
            let (lat, lon) = frame0.column_latlon(fx, fz);
            for active in &lms_active {
                if let Some((mx, mz)) = active.column_at(lat, lon, 0.0)
                    && let Some((height, _)) = active.flattened_terrain(mx, mz, target.1)
                {
                    target.1 = height;
                }
            }
        };
        if !lms_active.is_empty() {
            for z in 0..N {
                for x in 0..N {
                    correct_target(
                        (x as f64 + 0.5) / N as f64,
                        (z as f64 + 0.5) / N as f64,
                        &mut grades.columns[z][x],
                    );
                }
            }
            for z in 0..=N {
                for x in 0..=N {
                    correct_target(
                        x as f64 / N as f64,
                        z as f64 / N as f64,
                        &mut grades.corners[z][x],
                    );
                }
            }
        }
        for z in 0..N {
            for x in 0..N {
                let (w, target) = grades.columns[z][x];
                if w > 0.0 {
                    let col = &mut cols[z][x];
                    let delta = (target - col.elev).clamp(-15.0, 15.0) * w;
                    col.elev += delta;
                    col.top += delta;
                }
            }
        }
        road_surface_corners = Some(grades.corners);
    }

    // Activate curated landmark models whose footprint can touch
    // this cell, then resolve each column's model-space coordinates once.
    // These horizontal contexts only classify semantic exclusions and bounds.
    let mut lm_hits: Vec<Vec<&'static landmarks::Landmark>> = Vec::new();
    if !lms_active.is_empty() {
        lm_hits = vec![Vec::new(); N * N];
        for z in 0..N {
            for x in 0..N {
                let point = frame0.local_to_ecef(DVec3::new(x as f64 + 0.5, 0.0, z as f64 + 0.5));
                for a in &lms_active {
                    if a.excludes_source_at(point, voxel.x * 0.5) {
                        lm_hits[z * N + x].push(a.lm);
                    }
                }
            }
        }
    }

    // Roof authority samples only source buildings. The roughness gate rejects
    // canopy and roof-edge discontinuities before the per-building verdict.
    let surf_sampler = sources
        .surface
        .filter(|_| cell.level >= SURFACE_MIN_LEVEL)
        .map(|surf| {
            let hint = voxel.y * SAMPLE_ZOOM_FACTOR;
            let ring = voxel.x * 0.35;
            move |lat: f64, lon: f64| -> Option<f32> {
                const ROUGH_MAX_M: f32 = 4.0;
                let center = surf.elevation_opt(lat, lon, hint)?;
                let (mut lo, mut hi) = (center, center);
                let dlat = ring / vearth_core::ellipsoid::MEAN_RADIUS;
                let dlon = ring / (vearth_core::ellipsoid::MEAN_RADIUS * lat.cos().max(1e-6));
                for (sa, so) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                    // A NoData hole inside the ring is crown-shaped too
                    // (water-classified points under leaves).
                    let s = surf.elevation_opt(lat + sa * dlat, lon + so * dlon, hint)?;
                    lo = lo.min(s);
                    hi = hi.max(s);
                }
                (hi - lo <= ROUGH_MAX_M).then_some(center)
            }
        });
    let curated_near = cell_may_intersect_flower_destination(cell, &frame0);
    // Vector overlay, where enabled by source or where a
    // compact exact-site registry shape needs a visual-only fallback.
    generation_profile.checkpoint(1, cell.level);
    let overlay = pack
        .as_ref()
        .map(|pack| {
            let sparse_elevations = std::cell::RefCell::new(HashMap::new());
            let elev_geo = |x: i32, z: i32| {
                let gx = x + margin as i32;
                let gz = z + margin as i32;
                if (0..nn as i32).contains(&gx) && (0..nn as i32).contains(&gz) {
                    return tops_geo[gz as usize * nn + gx as usize];
                }
                // Exact neighboring roots can test a cliff stencil beyond
                // their admitted elevation margin. Sample only those columns
                // and keep the same corner average as interior columns.
                *sparse_elevations
                    .borrow_mut()
                    .entry((x, z))
                    .or_insert_with(|| {
                        [(0, 0), (1, 0), (0, 1), (1, 1)]
                            .into_iter()
                            .map(|(dx, dz)| {
                                let (lat, lon) = frame0.column_latlon(
                                    f64::from(x + dx) / N as f64,
                                    f64::from(z + dz) / N as f64,
                                );
                                sources.elevation.elevation_m(
                                    lat,
                                    lon,
                                    voxel.y * SAMPLE_ZOOM_FACTOR,
                                )
                            })
                            .sum::<f32>()
                            * 0.25
                    })
            };
            let surface_sampler = surf_sampler.as_ref().map(|at| tier0::SurfaceSampler {
                at: at as &dyn Fn(f64, f64) -> Option<f32>,
            });
            let height_eligible = |seed: u64| night_eligible_ids.contains(&seed);
            let night_eligible: Option<&dyn Fn(u64) -> bool> =
                Some(&height_eligible as &dyn Fn(u64) -> bool);
            let source_tree_columns = std::array::from_fn(|z| {
                (0..N).fold(0_u32, |mask, x| {
                    let unchanged =
                        (cols[z][x].elev - tops_geo[(z + margin) * nn + x + margin]).abs() < 1.0e-3;
                    let landmark = !lm_hits.is_empty() && !lm_hits[z * N + x].is_empty();
                    if unchanged && !landmark && cols[z][x].top > cols[z][x].sea {
                        mask | (1 << x)
                    } else {
                        mask
                    }
                })
            });
            let tree_ground = terrain_surface::SourceTreeGround::new(
                frame0,
                &corner_h,
                margin,
                source_tree_columns,
                sources.elevation,
            );
            let root_elevation = |lat, lon| tree_ground.elevation_at(lat, lon);
            tier0::build_overlay_with_tree_ground(
                cell,
                &frame0,
                pack,
                &elev_geo,
                margin,
                surface_sampler.as_ref(),
                None,
                night_eligible,
                Some(&root_elevation),
            )
        })
        .or_else(|| curated_near.then(tier0::CellOverlay::empty));
    // Landmark suppression, per-column half: roads and trees clear
    // where the model's footprint covers them (approach roads legitimately
    // continue beyond the bbox). Buildings are handled earlier and as whole
    // source features: the vector pack discards every ordinary footprint that
    // intersects a landmark replacement zone before massing, roofs, facades,
    // LiDAR or night-light channels can be derived from it. That decision is
    // therefore identical in every cell and cannot leave chopped fragments at
    // landmark or cell boundaries.
    let mut overlay = overlay;
    if let Some(overlay) = &mut overlay
        && curated_near
    {
        apply_curated_flower_destinations(cell, &frame0, overlay, &cols);
    }
    if let (Some(ov), false) = (&mut overlay, lm_hits.is_empty()) {
        for z in 0..N {
            for x in 0..N {
                for h in &lm_hits[z * N + x] {
                    if h.suppress.roads {
                        ov.cols[z][x].road = None;
                    }
                }
            }
        }
    }
    // Mapped coastlines own ocean classification; complete inland source coverage
    // keeps below-sea depressions dry. The direct water builder owns water levels.
    if let Some(ov) = &overlay {
        for z in 0..N {
            for x in 0..N {
                let col = &mut cols[z][x];
                if ov.cols[z][x].ocean {
                    col.sea = col.top - col.elev;
                } else if complete {
                    col.sea = f32::NEG_INFINITY;
                }
            }
        }
    }
    // Terrain geometry has already decided which columns are genuinely
    // ocean (including OSM-carved coast and complete-pack polders). Only
    // those columns pay for the extra fixed-level sample; ordinary land
    // generation keeps its previous sampling cost.
    for z in 0..N {
        for x in 0..N {
            let col = &mut cols[z][x];
            if col.sea > f32::NEG_INFINITY && cell.level >= WATER_BATHYMETRY_MIN_LEVEL {
                let (lat, lon) =
                    frame0.column_latlon((x as f64 + 0.5) / N as f64, (z as f64 + 0.5) / N as f64);
                col.bathymetry =
                    (-sources
                        .elevation
                        .elevation_m(lat, lon, WATER_BATHYMETRY_SAMPLE_M))
                    .max(0.0);
            } else {
                col.bathymetry = (-col.elev).max(0.0);
            }
        }
    }

    if let Some(overlay) = &mut overlay {
        resolve_crop_catalog(overlay, &frame0, &cols);
    }
    generation_profile.checkpoint(2, cell.level);
    let terrain_shading =
        terrain_appearance::sample(&frame0, sources, pack.as_ref(), &corner_h, margin);
    let mut terrain = TerrainSurfaceData {
        heights_m: Box::new(std::array::from_fn(|i| {
            let (x, z) = (i % (N + 1), i / (N + 1));
            let source = corner_h[(z + margin) * nc + x + margin];
            road_surface_corners.as_ref().map_or(source, |grades| {
                let (weight, target) = grades[z][x];
                source + (target - source).clamp(-15.0, 15.0) * weight
            })
        })),
        shading: Some(terrain_shading),
        source_columns: [u32::MAX; N],
    };
    let appearance = std::array::from_fn(|z| {
        std::array::from_fn(|x| {
            let col = &cols[z][x];
            let jitter = hash01(cell, x as u32, z as u32);
            let value = if let Some(flora) = overlay.as_ref().and_then(|o| o.cols[z][x].flora) {
                field_surface_appearance(flora, col, cell, x, z)
            } else if let Some(ground) = overlay.as_ref().and_then(|o| o.cols[z][x].ground) {
                ground_appearance(ground, col, jitter, jitter_scale)
            } else {
                terrain_appearance(col, col.top, jitter, jitter_scale)
            };
            SurfaceAppearance {
                material: value.material,
                tint: value.tint,
            }
        })
    });
    generation_profile.checkpoint(3, cell.level);
    let water_catalog = water_catalog::Candidates::for_frame(&frame0);
    let water = water_surface::build_corrected::<true>(
        &frame0,
        &mut terrain,
        pack.as_ref(),
        &|lat, lon| {
            sources
                .elevation
                .elevation_opt(lat, lon, crate::datum_pitch_m(lat))
        },
        &|lat, lon, depth, _surface, kind| {
            let p = frame0.ecef_to_local(vearth_core::ellipsoid::geodetic_to_ecef(lat, lon, 0.0));
            let x = (p.x.floor() as i32).clamp(0, N as i32 - 1) as usize;
            let z = (p.z.floor() as i32).clamp(0, N as i32 - 1) as usize;
            let inland = kind != vearth_data::vectors::WaterRegionKind::Ocean;
            let shore = overlay.as_ref().map_or(1.0, |o| o.cols[z][x].water_shore);
            let optical_depth = if inland {
                inland_water_depth(depth, shore)
            } else {
                ocean_water_depth(cols[z][x].bathymetry, shore)
            };
            let mut color_col = cols[z][x];
            // Evaluate at the shared vertex, not the containing column's
            // centre, so substrate patches agree across cell boundaries.
            color_col.landscape_variation = water_color::variation(
                lat,
                lon,
                frame0.grid_scale.x.max(frame0.grid_scale.z) as f32,
            );
            let value = water_appearance(optical_depth, &color_col, inland);
            SurfaceAppearance {
                material: value.material,
                tint: water_catalog.apply(
                    lat,
                    lon,
                    optical_depth,
                    frame0.grid_scale.x.max(frame0.grid_scale.z) as f32,
                    kind,
                    value.tint,
                ),
            }
        },
        &|lat, lon, height| {
            lms_active.iter().fold(height, |height, active| {
                active.corrected_water_height(lat, lon, height)
            })
        },
    );
    complete &= water.complete;
    generation_profile.record_incomplete(4, cell.level, water.complete);
    generation_profile.checkpoint(4, cell.level);
    let ground =
        terrain_surface::build_with_water(&frame0, &terrain, &appearance, &water.triangles);
    generation_profile.checkpoint(5, cell.level);
    let mut solid_triangles = ground.triangles;
    let mut building_collision = Vec::new();
    let mut batches = Vec::new();
    let mut push = |mesh: MeshBuffers, detail, shadow_omissions| {
        if !mesh.indices.is_empty() {
            batches.push(SurfaceBatch {
                addr: frame0.addr,
                mesh,
                detail,
                shadow_omissions,
            });
        }
    };
    push(ground.mesh, None, Vec::new());
    solid_triangles.extend(triangles_from_mesh(&water.mesh, &frame0));
    push(water.mesh, None, Vec::new());
    let road_exclusions: Vec<_> = lms_active
        .iter()
        .flat_map(|landmark| landmark.road_exclusions(&frame0))
        .collect();
    generation_profile.checkpoint(9, cell.level);
    if let Some((pack, overlay)) = pack.as_ref().zip(overlay.as_ref()) {
        push(
            hedgerow::build(&frame0, pack, overlay, &terrain, &road_exclusions, &lm_hits),
            None,
            Vec::new(),
        );
        let measured_roof = |lat: f64, lon: f64, hint: f32| {
            let source = sources.surface?;
            let centre = source.elevation_opt(lat, lon, f64::from(hint))?;
            let delta = 0.75 / vearth_core::ellipsoid::MEAN_RADIUS;
            let longitude_delta = delta / lat.cos().abs().max(1e-6);
            let (mut lo, mut hi) = (centre, centre);
            for (a, b) in [(-1., -1.), (1., -1.), (-1., 1.), (1., 1.)] {
                let value = source.elevation_opt(
                    lat + a * delta,
                    lon + b * longitude_delta,
                    f64::from(hint),
                )?;
                lo = lo.min(value);
                hi = hi.max(value);
            }
            (hi - lo <= 4.0).then_some(centre)
        };
        if let Some(building) = building_surface::generate(
            &frame0,
            pack,
            overlay,
            &[u32::MAX; N],
            &|lat, lon, hint| {
                sources
                    .elevation
                    .elevation_opt(lat, lon, f64::from(hint))
                    .map(|raw| landmarks::corrected_source_building_ground(lat, lon, raw))
            },
            &measured_roof,
        ) {
            complete &= building.complete;
            generation_profile.record_incomplete(5, cell.level, building.complete);
            // Admit whole building envelopes after ground/deck support. Never
            // truncate a closed prism or let optional building physics evict
            // valid terrain and bridges from an otherwise renderable cell.
            if building.collision.len() <= collision::MAX_COLLISION_TRIANGLES {
                building_collision = building.collision;
            }
            push(building.mesh, None, building.shadow_omissions);
            for detail in building.details {
                push(
                    detail.mesh,
                    Some(built_surface::BuiltDetailBounds {
                        center: detail.bound_center,
                        radius_m: detail.radius_m,
                        min_screen_px: detail.min_screen_px,
                    }),
                    Vec::new(),
                );
            }
        }
        generation_profile.checkpoint(6, cell.level);
        if let Some(bridge) =
            bridges::source_surface(&frame0, pack, &road_exclusions, &bridge_elev_en)
        {
            complete &= bridge.complete;
            generation_profile.record_incomplete(6, cell.level, bridge.complete);
            solid_triangles.extend(bridge.collision);
            push(bridge.mesh, None, Vec::new());
        }
        generation_profile.checkpoint(7, cell.level);
        if let Some(road) =
            road_surface::build(&frame0, pack, &terrain, &road_exclusions, &water.triangles)
        {
            complete &= road.complete;
            generation_profile.record_incomplete(7, cell.level, road.complete);
            solid_triangles.extend(triangles_from_mesh(&road.mesh, &frame0));
            push(road.mesh, None, Vec::new());
        }
        generation_profile.checkpoint(8, cell.level);
    }
    generation_profile.checkpoint(9, cell.level);
    let landmark = landmarks::generate(&frame0, &lms_active);
    solid_triangles.extend(landmark.collision);
    push(landmark.mesh, None, Vec::new());
    let wall = crate::great_wall::generate(&frame0, &great_wall, &|lat, lon| {
        f64::from(
            sources
                .elevation
                .elevation_m(lat, lon, crate::great_wall::SAMPLE_M),
        )
    });
    solid_triangles.extend(wall.collision);
    push(wall.mesh, None, Vec::new());
    drop(push);
    let flora = build_flora_descriptors(
        cell,
        &frame0,
        overlay.as_ref(),
        &cols,
        &lm_hits,
        ground.collision.as_ref(),
    );
    let tree_instances = overlay
        .as_mut()
        .map_or_else(Vec::new, |o| std::mem::take(&mut o.tree_instances));
    tree_render::append_surfaces(cell, &tree_instances, &mut batches);
    let road_guides = pack
        .as_ref()
        .map_or_else(Vec::new, |p| build_road_guides(cell, &frame0, p));
    generation_profile.checkpoint(9, cell.level);
    let terrain_step_m = (0..N)
        .flat_map(|z| (0..N).map(move |x| (x, z)))
        .map(|(x, z)| {
            let h = terrain.heights_m[z * (N + 1) + x];
            (terrain.heights_m[z * (N + 1) + x + 1] - h)
                .abs()
                .max((terrain.heights_m[(z + 1) * (N + 1) + x] - h).abs())
        })
        .fold(0.0_f32, f32::max);
    let terrain_relief_m = terrain_relief_m(&terrain);
    height_range = include_source_height_range(height_range, &frame0, &terrain);
    for batch in &batches {
        let frame = ChunkFrame::of(batch.addr);
        height_range = include_geometry_height_range(height_range, &frame, &batch.mesh);
    }
    for tree in &tree_instances {
        height_range.1 = height_range.1.max(tree.elevation_m + tree.height_m * 1.1);
    }
    if building_collision.len()
        <= collision::MAX_COLLISION_TRIANGLES.saturating_sub(solid_triangles.len())
    {
        solid_triangles.extend(building_collision);
    }
    let mut terrain_collision = ground.collision;
    let solid_collision =
        SurfaceCollision::from_bounded_triangles(solid_triangles, &mut terrain_collision);
    for (stage, sampled) in [
        (1, &elevation_complete),
        (2, &surface_complete),
        (3, &imagery_complete),
    ] {
        let sampled_complete = sampled.load(std::sync::atomic::Ordering::Relaxed);
        generation_profile.record_incomplete(stage, cell.level, sampled_complete);
        complete &= sampled_complete;
    }
    generation_profile.record_incomplete(8, cell.level, complete);
    let terrain_detail = elevation_complete
        .load(std::sync::atomic::Ordering::Relaxed)
        .then(|| TerrainDetailGrid::from_source(&terrain))
        .flatten();
    generation_profile.finish_generation();
    let result = SurfaceCell {
        cell,
        batches,
        height_range,
        terrain_relief_m,
        terrain_step_m,
        terrain_detail,
        terrain_collision,
        solid_collision,
        water: water.triangles,
        road_guides,
        night_lights: Vec::new(),
        tree_instances,
        flora,
        complete,
    };
    generation_profile.checkpoint(10, cell.level);
    generation_profile.finish();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use vearth_data::vectors::FeaturePack;

    fn original_coarse_height_range(
        mut range: (f32, f32),
        frame: &ChunkFrame,
        mesh: &MeshBuffers,
    ) -> (f32, f32) {
        for vertex in &mesh.vertices {
            let p = DVec3::from_array(vertex.pos.map(f64::from));
            let (_, _, h) = vearth_core::ellipsoid::ecef_to_geodetic(frame.local_to_ecef(p));
            if !h.is_finite() {
                continue;
            }
            let rounded = h as f32;
            let low = if f64::from(rounded) > h {
                rounded.next_down()
            } else {
                rounded
            };
            let high = if f64::from(rounded) < h {
                rounded.next_up()
            } else {
                rounded
            };
            range.0 = range.0.min(low);
            range.1 = range.1.max(high);
        }
        range
    }

    fn range_bits(range: (f32, f32)) -> (u32, u32) {
        (range.0.to_bits(), range.1.to_bits())
    }

    fn height_memo_mesh(positions: impl IntoIterator<Item = [f32; 3]>) -> MeshBuffers {
        MeshBuffers {
            vertices: positions
                .into_iter()
                .enumerate()
                .map(|(index, pos)| vearth_core::mesh::SurfaceVertex {
                    pos,
                    face_ao: index as u32,
                    rgba: [index as u8; 4],
                })
                .collect(),
            indices: Vec::new(),
        }
    }

    fn height_memo_frame(face: u8, level: u8, k: i32) -> ChunkFrame {
        ChunkFrame::of(ChunkAddr {
            cell: CellId {
                face: vearth_core::cubesphere::Face(face),
                level,
                i: (1 << level) - 1,
                j: (1 << level) / 2,
            },
            k,
        })
    }

    #[test]
    fn coarse_height_memo_matches_original_bits_across_frames_and_special_values() {
        for face in 0..6 {
            for level in [0, 3, 7, 10] {
                for k in [-3, 0, 4] {
                    let frame = height_memo_frame(face, level, k);
                    let mut positions: Vec<_> = [
                        (0.0, 0.0, -14_500.0),
                        (1.0, 1.0, 12_000.0),
                        (0.4, 0.7, 350.0),
                    ]
                    .into_iter()
                    .map(|(x, z, h)| {
                        let (surface, _) = frame.column_surface(x, z);
                        let normal = vearth_core::ellipsoid::geodetic_normal(surface);
                        frame
                            .ecef_to_local(surface + normal * h)
                            .as_vec3()
                            .to_array()
                    })
                    .collect();
                    positions.extend([
                        [0.0, 0.0, 0.0],
                        [-0.0, 0.0, 0.0],
                        [0.0, -0.0, 0.0],
                        [0.0, 0.0, -0.0],
                        [f32::INFINITY, 1.0, 2.0],
                        [f32::NEG_INFINITY, 1.0, 2.0],
                        [f32::from_bits(0x7fc0_0001), 1.0, 2.0],
                        [f32::from_bits(0x7fc0_0002), 1.0, 2.0],
                        [f32::MAX, -f32::MAX, f32::MAX],
                    ]);
                    positions.extend(positions.clone().into_iter().rev());
                    let mesh = height_memo_mesh(positions);
                    for initial in [
                        (-16_000.0, 10_000.0),
                        (f32::INFINITY, f32::NEG_INFINITY),
                        (-0.0, 0.0),
                        (0.0, -0.0),
                        (f32::NAN, f32::NAN),
                    ] {
                        assert_eq!(
                            range_bits(include_geometry_height_range(initial, &frame, &mesh)),
                            range_bits(original_coarse_height_range(initial, &frame, &mesh)),
                            "face={face} level={level} k={k} initial={initial:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn coarse_height_memo_reuses_inverse_for_generated_terrain_and_skirts() {
        for face in 0..6 {
            for level in [3, 7, 10] {
                let frame = height_memo_frame(face, level, 0);
                let source = TerrainSurfaceData {
                    heights_m: Box::new(std::array::from_fn(|i| {
                        let (x, z) = (i % (N + 1), i / (N + 1));
                        -300.0 + x as f32 * 19.25 + z as f32 * 23.75
                    })),
                    shading: None,
                    source_columns: [u32::MAX; N],
                };
                let geometry = terrain_surface::build(
                    &frame,
                    &source,
                    &[[SurfaceAppearance {
                        material: material::ROCK,
                        tint: [110, 115, 109],
                    }; N]; N],
                );
                let mesh = &geometry.mesh;
                let unique: rustc_hash::FxHashSet<_> = mesh
                    .vertices
                    .iter()
                    .map(|vertex| vertex.pos.map(f32::to_bits))
                    .collect();
                assert!(unique.len() < COARSE_HEIGHT_MEMO_LIMIT);
                let mut inverse_calls = 0;
                let initial = (-900.0, 1_600.0);
                let cached = include_coarse_geometry_height_range(initial, mesh, |position| {
                    inverse_calls += 1;
                    rounded_geometry_height(&frame, position)
                });
                assert_eq!(inverse_calls, unique.len());
                assert!(inverse_calls * 4 < mesh.vertices.len());
                assert_eq!(
                    range_bits(cached),
                    range_bits(original_coarse_height_range(initial, &frame, mesh))
                );
                assert_eq!(
                    range_bits(cached),
                    range_bits(include_geometry_height_range(initial, &frame, mesh))
                );
            }
        }
    }

    #[test]
    fn coarse_height_memo_bounds_capacity_and_caches_invalid_positions() {
        let frame = height_memo_frame(4, 10, -2);
        let overflow = 32;
        let unique = COARSE_HEIGHT_MEMO_LIMIT + overflow;
        let positions: Vec<_> = (0..unique)
            .map(|i| [i as f32 * 0.003, -5.0 + (i % 17) as f32, 9.0])
            .collect();
        let mesh = height_memo_mesh(positions.iter().copied().chain(positions.iter().copied()));
        let mut inverse_calls = 0;
        let cached = include_coarse_geometry_height_range((-0.0, 0.0), &mesh, |position| {
            inverse_calls += 1;
            rounded_geometry_height(&frame, position)
        });
        assert_eq!(inverse_calls, COARSE_HEIGHT_MEMO_LIMIT + 2 * overflow);
        assert_eq!(
            range_bits(cached),
            range_bits(original_coarse_height_range((-0.0, 0.0), &frame, &mesh))
        );

        let invalid = [f32::from_bits(0x7fc0_0013), 0.0, -0.0];
        let mesh = height_memo_mesh([invalid; 16]);
        let mut inverse_calls = 0;
        let initial = (-0.0, 0.0);
        let cached = include_coarse_geometry_height_range(initial, &mesh, |position| {
            inverse_calls += 1;
            rounded_geometry_height(&frame, position)
        });
        assert_eq!(inverse_calls, 1);
        assert_eq!(range_bits(cached), range_bits(initial));
    }

    #[test]
    fn coarse_height_memo_preserves_signed_zero_keys_and_original_fold_order() {
        let mesh = height_memo_mesh([
            [-0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0],
            [-0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0],
        ]);
        let bound = |position: [f32; 3]| Some((position[0], position[0]));
        let initial = (0.0_f32, -0.0_f32);
        let mut expected = initial;
        for vertex in &mesh.vertices {
            let (low, high) = bound(vertex.pos).unwrap();
            expected.0 = expected.0.min(low);
            expected.1 = expected.1.max(high);
        }
        let mut calls = 0;
        let actual = include_coarse_geometry_height_range(initial, &mesh, |position| {
            calls += 1;
            bound(position)
        });
        assert_eq!(calls, 2);
        assert_eq!(range_bits(actual), range_bits(expected));
        assert_eq!(
            range_bits(include_coarse_geometry_height_range(
                initial,
                &MeshBuffers::default(),
                |_| panic!("empty mesh must not invoke inverse")
            )),
            range_bits(initial)
        );
    }

    fn previous_box_height_range(
        mut range: (f32, f32),
        frame: &ChunkFrame,
        mesh: &MeshBuffers,
    ) -> (f32, f32) {
        let mut low = DVec3::splat(f64::INFINITY);
        let mut high = DVec3::splat(f64::NEG_INFINITY);
        for vertex in &mesh.vertices {
            let p = DVec3::from_array(vertex.pos.map(f64::from));
            low = low.min(p);
            high = high.max(p);
        }
        if !low.is_finite() {
            return range;
        }
        for i in 0..8 {
            let p = DVec3::new(
                if i & 1 == 0 { low.x } else { high.x },
                if i & 2 == 0 { low.y } else { high.y },
                if i & 4 == 0 { low.z } else { high.z },
            );
            let (_, _, h) = vearth_core::ellipsoid::ecef_to_geodetic(frame.local_to_ecef(p));
            range.0 = range.0.min(h as f32);
            range.1 = range.1.max(h as f32);
        }
        range
    }

    #[test]
    fn coarse_height_range_tracks_generated_surface_instead_of_box_corners() {
        let direction = vearth_core::ellipsoid::geodetic_to_ecef(
            51.9225_f64.to_radians(),
            4.47917_f64.to_radians(),
            0.0,
        );
        for (level, height) in [(3, 0.0_f32), (4, -500.0), (3, 9_000.0)] {
            let cell = CellId::from_dir(direction, level);
            let source = vearth_data::FnElevation(move |_, _| height);
            let output = generate_surface_cell(cell, &Sources::terrain_only(&source));
            assert!(output.complete);
            assert!(!output.batches.is_empty());
            assert_eq!(output.terrain_relief_m, 0.0);
            assert_eq!(output.terrain_step_m, 0.0);
            let grid = output.terrain_detail.as_ref().unwrap();
            assert!(grid.heights_m.iter().all(|sample| *sample == height));
            let range = TerrainDetailSummary::from_grid(grid)
                .unwrap()
                .query(cell, cell)
                .unwrap();
            assert_eq!(
                (range.min_m, range.max_m, range.step_m),
                (height, height, 0.0)
            );
            // Explore uses height_range as its floor fallback when the
            // curved globe has no compact terrain heightfield collider.
            assert!(output.terrain_collision.is_none());
            assert!(output.height_range.0 <= height);
            assert!(output.height_range.1 >= height);
            assert!(
                output.height_range.1 <= height.max(0.0) + 0.25,
                "L{level} source {height}: {:?}",
                output.height_range
            );
            let mut previous = (height, height);
            for batch in &output.batches {
                let frame = ChunkFrame::of(batch.addr);
                previous = previous_box_height_range(previous, &frame, &batch.mesh);
                for vertex in &batch.mesh.vertices {
                    let local = DVec3::from_array(vertex.pos.map(f64::from));
                    let (_, _, h) =
                        vearth_core::ellipsoid::ecef_to_geodetic(frame.local_to_ecef(local));
                    assert!(h >= f64::from(output.height_range.0));
                    assert!(h <= f64::from(output.height_range.1));
                }
            }
            if level == 3 && height == 0.0 {
                assert!(previous.1 > 50_000.0, "{previous:?}");
                assert!(output.height_range.0 < -70_000.0, "skirts must remain");
                eprintln!(
                    "Rotterdam sea L3: previous synthetic range={previous:?}, physical={:?}",
                    output.height_range
                );
            }
        }
    }

    #[test]
    fn terrain_relief_respects_owned_corners_and_finite_source_values() {
        let mut source = TerrainSurfaceData {
            heights_m: Box::new([9_000.0; (N + 1) * (N + 1)]),
            shading: None,
            source_columns: [0; N],
        };
        assert_eq!(terrain_relief_m(&source), 0.0);
        source.source_columns[N - 1] = 1 << (N - 1);
        let corners = [
            (N - 1) * (N + 1) + N - 1,
            (N - 1) * (N + 1) + N,
            N * (N + 1) + N - 1,
            N * (N + 1) + N,
        ];
        for (index, height) in corners.into_iter().zip([-350.0, -100.0, f32::NAN, 350.0]) {
            source.heights_m[index] = height;
        }
        // The last row/column belong to the active quad. Unowned high corners
        // and nonfinite values do not become terrain relief.
        assert_eq!(terrain_relief_m(&source), 700.0);
        for index in corners {
            source.heights_m[index] = f32::NAN;
        }
        assert_eq!(terrain_relief_m(&source), 0.0);
        source.heights_m[corners[0]] = -500.0;
        source.heights_m[corners[1]] = f32::INFINITY;
        source.heights_m[corners[2]] = f32::NEG_INFINITY;
        assert_eq!(terrain_relief_m(&source), 0.0);
        source.heights_m[corners[0]] = -f32::MAX;
        source.heights_m[corners[3]] = f32::MAX;
        assert_eq!(terrain_relief_m(&source), f32::MAX);
    }

    #[test]
    fn terrain_relief_preserves_hills_without_counting_tall_structures() {
        use vearth_data::vectors::{Building, BuildingKind, RoofShape};
        let cell = CellId {
            face: vearth_core::cubesphere::Face(0),
            level: 17,
            i: 1 << 16,
            j: 1 << 16,
        };
        let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
        let (latitude, _) = frame.column_latlon(0.5, 0.5);
        let hills = vearth_data::FnElevation(
            move |lat: f64, _: f64| {
                if lat < latitude { 100.0 } else { 800.0 }
            },
        );
        let mountain = generate_surface_cell(cell, &Sources::terrain_only(&hills));
        assert!(mountain.complete);
        assert_eq!(mountain.terrain_relief_m, 700.0);
        assert_eq!(mountain.terrain_step_m, 700.0);
        assert!(mountain.height_range.1 >= 800.0);
        let range = TerrainDetailSummary::from_grid(mountain.terrain_detail.as_ref().unwrap())
            .unwrap()
            .query(cell, cell)
            .unwrap();
        assert_eq!(
            (range.min_m, range.max_m, range.step_m),
            (100.0, 800.0, 700.0)
        );

        struct Vectors(bool);
        impl VectorSource for Vectors {
            fn features(&self, _: f64, _: f64, _: f64, _: f64) -> FeaturePack {
                let mut pack = FeaturePack::empty();
                if self.0 {
                    pack.buildings.push(Building {
                        location_rad: None,
                        roof_shape: Some(RoofShape::Flat),
                        use_kind: Default::default(),
                        rings: vec![vec![[-9.0, -8.0], [9.0, -8.0], [9.0, 8.0], [-9.0, 8.0]]],
                        height_m: 800.0,
                        min_height_m: 0.0,
                        seed: 17,
                        kind: BuildingKind::Building,
                    });
                }
                pack
            }
        }
        let ground = vearth_data::FnElevation(|_, _| 30.0);
        let generate = |has_building| {
            let vectors = Vectors(has_building);
            generate_surface_cell(
                cell,
                &Sources {
                    elevation: &ground,
                    surface: None,
                    imagery: None,
                    vectors: Some(&vectors),
                },
            )
        };
        let plain = generate(false);
        let built = generate(true);
        assert!(plain.complete && built.complete);
        assert!(built.height_range.1 > plain.height_range.1 + 750.0);
        assert_eq!(plain.terrain_relief_m, 0.0);
        assert_eq!(built.terrain_relief_m, 0.0);
        assert_eq!(built.terrain_step_m, plain.terrain_step_m);
        assert_eq!(built.terrain_detail, plain.terrain_detail);
        let fingerprint = contracts::surface_cell_fingerprint(&built);
        let mut changed_metric = built.clone();
        changed_metric.terrain_relief_m = 1.0;
        assert_ne!(
            fingerprint,
            contracts::surface_cell_fingerprint(&changed_metric)
        );
        for change in 0..3 {
            let mut changed = built.clone();
            match change {
                0 => changed.terrain_detail = None,
                1 => changed.terrain_detail.as_mut().unwrap().heights_m[0] += 1.0,
                _ => changed.terrain_detail.as_mut().unwrap().source_columns[0] ^= 1,
            }
            assert_ne!(fingerprint, contracts::surface_cell_fingerprint(&changed));
        }
    }

    #[test]
    fn coarse_height_range_preserves_authored_extrema_across_frames() {
        for face in 0..6 {
            for level in [3, 5, 10] {
                for k in [-2, 0, 3] {
                    let frame = ChunkFrame::of(ChunkAddr {
                        cell: CellId {
                            face: vearth_core::cubesphere::Face(face),
                            level,
                            i: (1 << level) - 1,
                            j: (1 << level) / 2,
                        },
                        k,
                    });
                    let vertices = [
                        (0.0, 0.0, -14_500.0),
                        (1.0, 1.0, 12_000.0),
                        (0.4, 0.7, 350.0),
                    ]
                    .map(|(x, z, h)| {
                        let (surface, _) = frame.column_surface(x, z);
                        let normal = vearth_core::ellipsoid::geodetic_normal(surface);
                        vearth_core::mesh::SurfaceVertex {
                            pos: frame
                                .ecef_to_local(surface + normal * h)
                                .as_vec3()
                                .to_array(),
                            face_ao: 0,
                            rgba: [255; 4],
                        }
                    });
                    let mesh = MeshBuffers {
                        vertices: vertices.to_vec(),
                        indices: vec![0, 1, 2],
                    };
                    let range = include_geometry_height_range((-16_000.0, 10_000.0), &frame, &mesh);
                    assert!(range.0 <= -16_000.0);
                    assert!(range.1 > 11_999.0);
                    for vertex in &mesh.vertices {
                        let local = DVec3::from_array(vertex.pos.map(f64::from));
                        let (_, _, h) =
                            vearth_core::ellipsoid::ecef_to_geodetic(frame.local_to_ecef(local));
                        assert!(
                            h >= f64::from(range.0) && h <= f64::from(range.1),
                            "{range:?} {h}"
                        );
                    }
                    assert_eq!(
                        include_geometry_height_range(range, &frame, &MeshBuffers::default()),
                        range
                    );
                }
            }
        }
    }

    #[test]
    fn fine_height_range_keeps_existing_geometry_bounds() {
        let mesh = MeshBuffers {
            vertices: [[-64.0, -8.0, 5.0], [128.0, 80.0, 96.0], [10.0, 6.0, -40.0]]
                .map(|pos| vearth_core::mesh::SurfaceVertex {
                    pos,
                    face_ao: 0,
                    rgba: [255; 4],
                })
                .to_vec(),
            indices: vec![0, 1, 2],
        };
        for face in 0..6 {
            for level in [tier0::BUILDINGS_MIN_LEVEL, 19] {
                for k in [-3, 0, 4] {
                    let frame = ChunkFrame::of(ChunkAddr {
                        cell: CellId {
                            face: vearth_core::cubesphere::Face(face),
                            level,
                            i: (1 << level) - 1,
                            j: 1 << (level - 1),
                        },
                        k,
                    });
                    for geometry in [&mesh, &MeshBuffers::default()] {
                        let range = (-35.0, 450.0);
                        assert_eq!(
                            include_geometry_height_range(range, &frame, geometry),
                            previous_box_height_range(range, &frame, geometry)
                        );
                    }
                }
            }
        }
    }

    fn sample_status_cell() -> CellId {
        CellId {
            face: vearth_core::cubesphere::Face(0),
            level: 10,
            i: 1 << 9,
            j: 1 << 9,
        }
    }

    #[test]
    fn repeated_missing_sample_remains_incomplete_without_new_global_errors() {
        struct Missing;
        impl ElevationSource for Missing {
            fn elevation_m(&self, _: f64, _: f64, _: f64) -> f32 {
                0.0
            }
            fn sample_elevation(&self, _: f64, _: f64, _: f64) -> vearth_data::SourceSample<f32> {
                vearth_data::SourceSample::unavailable()
            }
        }
        for _ in 0..2 {
            let output =
                generate_surface_cell(sample_status_cell(), &Sources::terrain_only(&Missing));
            assert!(
                !output.complete,
                "a live tombstone is still a missing dependency"
            );
            assert!(output.terrain_detail.is_none());
        }
    }

    #[test]
    fn valid_sample_is_complete_despite_concurrent_unrelated_provider_errors() {
        struct SharedProvider {
            errors: AtomicUsize,
            first: std::sync::atomic::AtomicBool,
            barrier: std::sync::Barrier,
        }
        impl ElevationSource for SharedProvider {
            fn elevation_m(&self, _: f64, _: f64, _: f64) -> f32 {
                if self.first.swap(false, Ordering::Relaxed) {
                    self.barrier.wait();
                    self.barrier.wait();
                }
                100.0
            }
            fn fetch_errors(&self) -> u64 {
                self.errors.load(Ordering::Relaxed) as u64
            }
        }
        let source = SharedProvider {
            errors: AtomicUsize::new(0),
            first: std::sync::atomic::AtomicBool::new(true),
            barrier: std::sync::Barrier::new(2),
        };
        vearth_platform::thread::scope(|scope| {
            scope.spawn(|| {
                source.barrier.wait();
                source.errors.fetch_add(1, Ordering::Relaxed);
                source.barrier.wait();
            });
            let output =
                generate_surface_cell(sample_status_cell(), &Sources::terrain_only(&source));
            assert!(
                output.complete,
                "another cell's failed sample cannot invalidate this geometry"
            );
            assert!(output.terrain_detail.is_some());
        });
        assert_eq!(source.fetch_errors(), 1);
    }

    #[test]
    fn degraded_sample_preserves_geometry_but_cannot_be_cached_as_complete() {
        struct Degraded;
        impl ElevationSource for Degraded {
            fn elevation_m(&self, _: f64, _: f64, _: f64) -> f32 {
                100.0
            }
            fn sample_elevation(&self, _: f64, _: f64, _: f64) -> vearth_data::SourceSample<f32> {
                vearth_data::SourceSample {
                    value: Some(100.0),
                    complete: false,
                }
            }
        }
        let expected = generate_surface_cell(
            sample_status_cell(),
            &Sources::terrain_only(&vearth_data::FnElevation(|_, _| 100.0)),
        );
        let actual = generate_surface_cell(sample_status_cell(), &Sources::terrain_only(&Degraded));
        assert!(expected.complete);
        assert!(!actual.complete);
        assert!(expected.terrain_detail.is_some());
        assert!(actual.terrain_detail.is_none());
        let mut actual = actual;
        actual.complete = true;
        actual.terrain_detail = expected.terrain_detail.clone();
        assert_eq!(
            contracts::surface_cell_fingerprint(&actual),
            contracts::surface_cell_fingerprint(&expected)
        );
    }

    #[test]
    fn terrain_detail_survives_unrelated_feature_incompleteness() {
        struct MissingVectors {
            calls: AtomicUsize,
        }
        impl VectorSource for MissingVectors {
            fn features(&self, _: f64, _: f64, _: f64, _: f64) -> FeaturePack {
                self.calls.fetch_add(1, Ordering::Relaxed);
                let mut pack = FeaturePack::empty();
                pack.complete = false;
                pack
            }
        }
        let elevation = vearth_data::FnElevation(|_, _| -12.0);
        let vectors = MissingVectors {
            calls: AtomicUsize::new(0),
        };
        let cell = sample_status_cell().children()[0];
        assert_eq!(cell.level, tier0::WATER_MIN_LEVEL);
        let output = generate_surface_cell(
            cell,
            &Sources {
                elevation: &elevation,
                surface: None,
                imagery: None,
                vectors: Some(&vectors),
            },
        );
        assert_eq!(vectors.calls.load(Ordering::Relaxed), 1);
        assert!(!output.complete);
        let summary =
            TerrainDetailSummary::from_grid(output.terrain_detail.as_ref().unwrap()).unwrap();
        let range = summary.query(output.cell, output.cell).unwrap();
        assert_eq!(
            (range.min_m, range.max_m, range.step_m),
            (-12.0, -12.0, 0.0)
        );
    }

    #[test]
    fn acquired_vector_features_preserve_generation_and_skip_the_second_query() {
        use vearth_data::vectors::{Building, BuildingKind, Road, RoadClass};
        struct Vectors {
            calls: AtomicUsize,
        }
        impl VectorSource for Vectors {
            fn features(&self, _: f64, _: f64, _: f64, _: f64) -> FeaturePack {
                self.calls.fetch_add(1, Ordering::Relaxed);
                let mut pack = FeaturePack::empty();
                pack.buildings.push(Building {
                    location_rad: None,
                    roof_shape: None,
                    use_kind: Default::default(),
                    rings: vec![vec![[-9.0, -8.0], [9.0, -8.0], [9.0, 8.0], [-9.0, 8.0]]],
                    height_m: 13.0,
                    min_height_m: 0.0,
                    seed: 17,
                    kind: BuildingKind::Building,
                });
                pack.roads.push(Road {
                    profile: Default::default(),
                    pts: vec![[-35.0, 20.0], [35.0, 20.0]],
                    class: RoadClass::Minor,
                    is_bridge: false,
                });
                pack
            }
        }
        let vectors = Vectors {
            calls: AtomicUsize::new(0),
        };
        let ground = vearth_data::FnElevation(|_, _| 100.0);
        let cell = CellId {
            face: vearth_core::cubesphere::Face(0),
            level: 17,
            i: 1 << 16,
            j: 1 << 16,
        };
        let sources = Sources {
            elevation: &ground,
            surface: None,
            imagery: None,
            vectors: Some(&vectors),
        };
        let expected = generate_surface_cell(cell, &sources);
        assert_eq!(vectors.calls.load(Ordering::Relaxed), 1);
        let (lat, lon, half_e, half_n) = vector_query_bounds(cell);
        let pack = vectors.features(lat, lon, half_e, half_n);
        let actual = generate_surface_cell_with_features(cell, &sources, pack);
        assert_eq!(vectors.calls.load(Ordering::Relaxed), 2);
        assert_eq!(
            contracts::surface_cell_fingerprint(&actual),
            contracts::surface_cell_fingerprint(&expected)
        );
    }

    #[test]
    fn cells_without_source_buildings_do_not_scan_surface_models() {
        struct EmptyVectors;
        impl VectorSource for EmptyVectors {
            fn features(&self, _: f64, _: f64, _: f64, _: f64) -> FeaturePack {
                FeaturePack::empty()
            }
        }
        let calls = AtomicUsize::new(0);
        let ground = vearth_data::FnElevation(|_, _| 100.0);
        let dsm = vearth_data::FnElevation(|_, _| {
            calls.fetch_add(1, Ordering::Relaxed);
            120.0
        });
        let empty = EmptyVectors;
        let cell = CellId {
            face: vearth_core::cubesphere::Face(0),
            level: 17,
            i: 1 << 16,
            j: 1 << 16,
        };
        for vectors in [None, Some(&empty as &dyn VectorSource)] {
            let output = generate_surface_cell(
                cell,
                &Sources {
                    elevation: &ground,
                    surface: Some(&dsm),
                    imagery: None,
                    vectors,
                },
            );
            assert!(output.complete);
            assert_eq!(calls.load(Ordering::Relaxed), 0);
        }
    }
    #[test]
    fn road_grading_preserves_curated_garden_ground() {
        use vearth_data::vectors::{Road, RoadClass};
        struct GardenPaths;
        impl VectorSource for GardenPaths {
            fn features(&self, _: f64, _: f64, _: f64, _: f64) -> FeaturePack {
                let mut pack = FeaturePack::empty();
                pack.roads.push(Road {
                    profile: vearth_data::road_profile::RoadProfile {
                        detail: vearth_data::road_profile::RoadDetail::Pedestrian,
                        ..Default::default()
                    },
                    pts: vec![[-200.0, 0.0], [200.0, 0.0]],
                    class: RoadClass::Path,
                    is_bridge: false,
                });
                pack
            }
        }
        let landmark = landmarks::LANDMARKS
            .iter()
            .find(|lm| lm.name == "singapore-supertrees")
            .unwrap();
        let cell = CellId::from_dir(
            vearth_core::ellipsoid::geodetic_to_ecef(
                landmark.lat_deg.to_radians(),
                landmark.lon_deg.to_radians(),
                0.0,
            ),
            19,
        );
        let roof_dem = vearth_data::FnElevation(|_, _| 40.0);
        let sources = Sources {
            elevation: &roof_dem,
            surface: None,
            imagery: None,
            vectors: Some(&GardenPaths),
        };
        let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
        let raw_grades = tier0::road_surface_grades(
            &GardenPaths.features(0.0, 0.0, 0.0, 0.0),
            &frame,
            cell.level,
            &|_, _| 40.0,
        )
        .unwrap();
        assert!(
            raw_grades
                .corners
                .iter()
                .flatten()
                .any(|(w, h)| *w > 0.9 && *h == 40.0)
        );
        let generated = generate_surface_cell(cell, &sources);
        assert!(generated.complete);
        let terrain = generated.terrain_detail.unwrap();
        assert!(
            terrain.heights_m.iter().all(|h| (*h - 5.28).abs() < 0.001),
            "road grading must not lift the corrected garden above its authored base"
        );
    }
    #[test]
    fn hongyadong_riverfront_road_and_terrain_stay_below_the_cliff_village() {
        use vearth_data::vectors::{Road, RoadClass};
        struct RiverfrontRoad {
            is_bridge: bool,
        }
        impl VectorSource for RiverfrontRoad {
            fn features(&self, _: f64, _: f64, _: f64, _: f64) -> FeaturePack {
                let mut pack = FeaturePack::empty();
                pack.roads.push(Road {
                    profile: Default::default(),
                    pts: vec![[-100.0, 0.0], [100.0, 0.0]],
                    class: RoadClass::Minor,
                    is_bridge: self.is_bridge,
                });
                pack
            }
        }
        let landmark = landmarks::LANDMARKS
            .iter()
            .find(|lm| lm.name == "hongyadong")
            .unwrap();
        // This correction is a sloping cliff cap, not a level-pad registration.
        // The test must exercise it even though terrain_flatten() is empty.
        assert!(landmark.terrain_flatten().is_empty());
        let landmarks::Base::Fixed(base) = landmark.base else {
            panic!("Hongya's audited datum must remain independent of the DEM sample pitch");
        };
        let ground = f64::from(base - 50.8);
        let lat = landmark.lat_deg.to_radians();
        let lon = landmark.lon_deg.to_radians();
        let anchor = vearth_core::ellipsoid::geodetic_to_ecef(lat, lon, 0.0);
        let east = DVec3::new(-lon.sin(), lon.cos(), 0.0);
        let north = DVec3::new(-lat.sin() * lon.cos(), -lat.sin() * lon.sin(), lat.cos());
        let (sin_h, cos_h) = landmark.heading_deg.to_radians().sin_cos();
        let mut tested = None;
        // Select a real maximum-detail cell entirely on the riverfront cap,
        // avoiding the outer feather and the upper part of the sloped cliff.
        'find: for x in [-60.0, -30.0, 0.0, 30.0, 60.0] {
            for z in [45.0, 50.0, 55.0, 60.0] {
                let point =
                    anchor + east * (x * cos_h + z * sin_h) + north * (-x * sin_h + z * cos_h);
                let cell = CellId::from_dir(point.normalize(), vearth_core::cell::MAX_LEVEL);
                let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
                let active = landmarks::active_for(cell, &frame, &|_, _| 231.0)
                    .into_iter()
                    .find(|a| a.lm.name == landmark.name)
                    .unwrap();
                if [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)]
                    .iter()
                    .all(|&(x, z)| {
                        let (lat, lon) = frame.column_latlon(x, z);
                        active
                            .column_at(lat, lon, 0.0)
                            .is_some_and(|(x, z)| x.abs() < 90.0 && (35.0..69.0).contains(&z))
                    })
                {
                    tested = Some((cell, frame));
                    break 'find;
                }
            }
        }
        let (cell, cell_frame) = tested.expect("Hongya riverfront cap must contain a fine cell");
        let active = landmarks::active_for(cell, &cell_frame, &|_, _| 231.0)
            .into_iter()
            .find(|a| a.lm.name == landmark.name)
            .unwrap();
        let contaminated_dem = vearth_data::FnElevation(|_, _| 231.0);
        for is_bridge in [false, true] {
            let output = generate_surface_cell(
                cell,
                &Sources {
                    elevation: &contaminated_dem,
                    surface: None,
                    imagery: None,
                    vectors: Some(&RiverfrontRoad { is_bridge }),
                },
            );
            assert!(output.complete);
            let terrain = output.terrain_detail.as_ref().unwrap();
            assert!(
                terrain
                    .heights_m
                    .iter()
                    .all(|h| (f64::from(*h) - ground).abs() < 0.001),
                "the original 231m cliff DEM must not return through road-grade corners"
            );
            assert!(output.terrain_relief_m < 0.001 && output.terrain_step_m < 0.001);
            let mut road_vertices = 0;
            for batch in &output.batches {
                let frame = ChunkFrame::of(batch.addr);
                for vertex in &batch.mesh.vertices {
                    if (vertex.face_ao >> 5) & 255 != u32::from(crate::material::ASPHALT) {
                        continue;
                    }
                    let point = frame.local_to_ecef(DVec3::from_array(vertex.pos.map(f64::from)));
                    let (lat, lon, height) = vearth_core::ellipsoid::ecef_to_geodetic(point);
                    let (x, z) = active.column_at(lat, lon, 0.0).unwrap();
                    // The bridge uses its independent 8m geographic terrain
                    // lattice. Inspect its flat interior, clear of lattice
                    // triangles that legitimately touch the outer feather.
                    if is_bridge && (x.abs() > 80.0 || !(42.0..59.0).contains(&z)) {
                        continue;
                    }
                    let expected = ground + if is_bridge { 5.0 } else { 0.035 };
                    assert!(
                        (height - expected).abs() < 0.015,
                        "riverfront asphalt floated over the lowered cliff terrain: {height}m"
                    );
                    road_vertices += 1;
                }
            }
            assert!(
                road_vertices > 0,
                "the retained riverfront road must actually render"
            );
        }
    }
}
