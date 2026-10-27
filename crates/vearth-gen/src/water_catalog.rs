use crate::{N, lerp3, palette::Rgb, smoothstep01, water_color};
use glam::DVec3;
use vearth_core::{ellipsoid::MEAN_RADIUS, frame::ChunkFrame};
use vearth_data::vectors::WaterRegionKind;

#[path = "water_catalog_data.rs"]
mod data;

struct Entry {
    name: &'static str,
    origin_deg: [f64; 2],
    metres_per_degree: [f64; 2],
    anchor: [f64; 3],
    radius_m: f64,
    bounds_m: [f64; 4],
    margin_m: f64,
    fade_m: f64,
    rings: &'static [&'static [[f64; 2]]],
    bottom: Rgb,
    body: Rgb,
    alternate: Rgb,
    extinction: Rgb,
    variation_scale_m: f32,
}

fn direction(lat: f64, lon: f64) -> DVec3 {
    DVec3::new(lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin())
}

pub(crate) struct Candidates(Vec<&'static Entry>);

impl Candidates {
    pub(crate) fn for_frame(frame: &ChunkFrame) -> Self {
        let (lat, lon) = frame.column_latlon(0.5, 0.5);
        let center = direction(lat, lon);
        // The full grid diagonal conservatively bounds this curved cell and
        // avoids dropping a lake that is visible only in the cell's corner.
        let cell_radius = (frame.grid_scale.x + frame.grid_scale.z) * N as f64;
        Self(
            data::ENTRIES
                .iter()
                .filter(|entry| {
                    let radius = (cell_radius + entry.radius_m) / MEAN_RADIUS;
                    center.distance_squared(DVec3::from_array(entry.anchor)) <= radius * radius
                })
                .inspect(|entry| log::trace!("water catalog: {}", entry.name))
                .collect(),
        )
    }

    pub(crate) fn apply(
        &self,
        lat: f64,
        lon: f64,
        depth_m: f32,
        sample_pitch_m: f32,
        kind: WaterRegionKind,
        base: [u8; 3],
    ) -> [u8; 3] {
        if self.0.is_empty()
            || !matches!(
                kind,
                WaterRegionKind::InlandWater | WaterRegionKind::Lake | WaterRegionKind::Reservoir
            )
        {
            return base;
        }
        for entry in &self.0 {
            let p = entry.project(lat.to_degrees(), lon.to_degrees());
            let weight = entry.weight(p);
            if weight <= 0.0 {
                continue;
            }
            let scale = entry.variation_scale_m;
            let detail = 1.0 - smoothstep01((sample_pitch_m / scale - 0.12) / 0.38);
            let field =
                0.5 + (noise(p[0] / f64::from(scale), p[1] / f64::from(scale)) - 0.5) * detail;
            let body = lerp3(entry.body, entry.alternate, field);
            let tint =
                water_color::inland_column_color(depth_m, entry.bottom, body, entry.extinction);
            return water_color::blend_tints(base, tint, weight);
        }
        base
    }
}

impl Entry {
    fn project(&self, lat: f64, lon: f64) -> [f64; 2] {
        [
            ((lon - self.origin_deg[1] + 180.0).rem_euclid(360.0) - 180.0)
                * self.metres_per_degree[1],
            (lat - self.origin_deg[0]) * self.metres_per_degree[0],
        ]
    }

    fn weight(&self, p: [f64; 2]) -> f32 {
        let reach = self.margin_m + self.fade_m;
        if !p.into_iter().all(f64::is_finite)
            || p[0] < self.bounds_m[0] - reach
            || p[1] < self.bounds_m[1] - reach
            || p[0] > self.bounds_m[2] + reach
            || p[1] > self.bounds_m[3] + reach
        {
            return 0.0;
        }
        let mut inside = false;
        for ring in self.rings {
            for (a, b) in ring
                .iter()
                .zip(ring.iter().cycle().skip(1))
                .take(ring.len())
            {
                if (a[1] > p[1]) != (b[1] > p[1])
                    && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]
                {
                    inside = !inside;
                }
            }
        }
        if inside {
            return 1.0;
        }
        // Most source-water vertices lie inside the mask. Only the sparse
        // exterior fringe needs distance-to-edge work.
        let mut nearest_squared = f64::INFINITY;
        for ring in self.rings {
            for (a, b) in ring
                .iter()
                .zip(ring.iter().cycle().skip(1))
                .take(ring.len())
            {
                let d = [b[0] - a[0], b[1] - a[1]];
                let length_squared = d[0] * d[0] + d[1] * d[1];
                let t = if length_squared > 0.0 {
                    (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / length_squared).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let dx = p[0] - a[0] - t * d[0];
                let dy = p[1] - a[1] - t * d[1];
                nearest_squared = nearest_squared.min(dx * dx + dy * dy);
            }
        }
        // The dry-land margin absorbs source simplification differences.
        // A shared geographic fade prevents a color seam if a revised
        // shoreline extends beyond it. This never creates water geometry.
        1.0 - smoothstep01(((nearest_squared.sqrt() - self.margin_m) / self.fade_m) as f32)
    }
}

fn noise(x: f64, y: f64) -> f32 {
    let hash = |x: i64, y: i64| {
        let mut n = (x as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
            ^ (y as u64).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        n = (n ^ (n >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        n = (n ^ (n >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        ((n ^ (n >> 31)) >> 40) as f32 / ((1u32 << 24) - 1) as f32
    };
    let ix = x.floor() as i64;
    let iy = y.floor() as i64;
    let u = smoothstep01((x - x.floor()) as f32);
    let v = smoothstep01((y - y.floor()) as f32);
    let a = hash(ix, iy) * (1.0 - u) + hash(ix + 1, iy) * u;
    let b = hash(ix, iy + 1) * (1.0 - u) + hash(ix + 1, iy + 1) * u;
    a * (1.0 - v) + b * v
}

#[cfg(test)]
mod tests {
    use super::*;
    use vearth_core::{
        cell::{CellId, ChunkAddr},
        cubesphere,
        ellipsoid::geodetic_to_ecef,
    };

    fn frame(lat: f64, lon: f64, level: u8) -> ChunkFrame {
        let direction = geodetic_to_ecef(lat.to_radians(), lon.to_radians(), 0.0).normalize();
        let (face, u, v) = cubesphere::dir_to_face_uv(direction);
        let count = (1u32 << level) as f64;
        ChunkFrame::of(ChunkAddr {
            cell: CellId {
                face,
                level,
                i: ((u + 1.0) * 0.5 * count) as u32,
                j: ((v + 1.0) * 0.5 * count) as u32,
            },
            k: 0,
        })
    }

    #[test]
    fn every_lake_survives_cell_filtering_and_has_bounded_optics() {
        for entry in data::ENTRIES {
            let [lat, lon] = entry.origin_deg;
            assert_eq!(entry.weight(entry.project(lat, lon)), 1.0, "{}", entry.name);
            for level in [0, 5, 8, 12, 16, 20] {
                let candidates = Candidates::for_frame(&frame(lat, lon, level));
                assert!(
                    candidates.0.iter().any(|e| e.name == entry.name),
                    "{} at L{level}",
                    entry.name
                );
            }
            for ring in entry.rings {
                assert!(ring.len() >= 3);
                for &p in *ring {
                    assert_eq!(entry.weight(p), 1.0);
                    let lat = entry.origin_deg[0] + p[1] / entry.metres_per_degree[0];
                    let lon = entry.origin_deg[1] + p[0] / entry.metres_per_degree[1];
                    for level in [8, 16] {
                        assert!(
                            Candidates::for_frame(&frame(lat, lon, level))
                                .0
                                .iter()
                                .any(|e| e.name == entry.name)
                        );
                    }
                }
            }
            assert!(
                entry
                    .extinction
                    .into_iter()
                    .all(|v| v > 0.0 && v.is_finite())
            );
            assert!(
                entry
                    .body
                    .into_iter()
                    .chain(entry.bottom)
                    .chain(entry.alternate)
                    .all(|v| (0.0..=255.0).contains(&v))
            );
        }
    }

    #[test]
    fn mask_fades_are_continuous_and_coarse_sampling_removes_pigment_detail() {
        let entry = &data::ENTRIES[0];
        let x = entry.bounds_m[2];
        let y = entry
            .rings
            .iter()
            .flat_map(|ring| ring.iter())
            .find(|p| p[0] == x)
            .unwrap()[1];
        assert_eq!(entry.weight([x + entry.margin_m, y]), 1.0);
        assert!((entry.weight([x + entry.margin_m + entry.fade_m * 0.5, y]) - 0.5).abs() < 1e-5);
        assert_eq!(entry.weight([x + entry.margin_m + entry.fade_m, y]), 0.0);
        let all = Candidates(vec![entry]);
        let [lat, lon] = entry.origin_deg.map(f64::to_radians);
        let sample = |lat, pitch| all.apply(lat, lon, 16.0, pitch, WaterRegionKind::Lake, [0; 3]);
        assert_eq!(sample(lat, 1.0), sample(lat, 48.0));
        assert_eq!(sample(lat, 1600.0), sample(lat + 0.00001, 1600.0));
        assert_eq!(sample(lat, 1600.0), sample(lat, 8000.0));
        assert_eq!(Candidates::for_frame(&frame(0.0, 0.0, 16)).0.len(), 0);
    }

    #[test]
    fn nearby_ocean_rivers_pools_and_other_lakes_retain_their_colors() {
        let base = [21, 71, 95];
        for entry in data::ENTRIES {
            let [lat, lon] = entry.origin_deg.map(f64::to_radians);
            let candidates = Candidates(vec![entry]);
            for kind in [
                WaterRegionKind::Ocean,
                WaterRegionKind::River,
                WaterRegionKind::Canal,
                WaterRegionKind::Decorative,
                WaterRegionKind::Playa,
                WaterRegionKind::Reef,
            ] {
                assert_eq!(candidates.apply(lat, lon, 8.0, 1.0, kind, base), base);
            }
        }
        let all = Candidates(data::ENTRIES.iter().collect());
        // Valley Lake beside Blue Lake, Lake Agnes beside Louise, and the
        // Indian Ocean alongside Hutt Lagoon must remain outside the masks.
        for [lat, lon] in [
            [-37.844, 140.760],
            [51.416, -116.244],
            [-28.19, 114.235],
            [0.0, 0.0],
        ] {
            assert_eq!(
                all.apply(
                    f64::to_radians(lat),
                    f64::to_radians(lon),
                    8.0,
                    1.0,
                    WaterRegionKind::Lake,
                    base
                ),
                base
            );
        }
    }

    #[test]
    fn pigments_do_not_turn_ocean_blue_at_depth_and_patterns_are_shared() {
        let all = Candidates(data::ENTRIES.iter().collect());
        for entry in &data::ENTRIES[..3] {
            let [lat, lon] = entry.origin_deg.map(f64::to_radians);
            for depth in [2.5, 16.0, 300.0, 4000.0] {
                let tint = all.apply(lat, lon, depth, 1.0, WaterRegionKind::Lake, [0; 3]);
                assert!(
                    u16::from(tint[0]) > u16::from(tint[1]) + 50,
                    "{} {tint:?}",
                    entry.name
                );
                assert!(tint[0] > tint[2]);
            }
            assert_eq!(
                all.apply(lat, lon, 16.0, 1.0, WaterRegionKind::Lake, [0; 3]),
                all.apply(
                    lat,
                    lon + std::f64::consts::TAU,
                    16.0,
                    1.0,
                    WaterRegionKind::Lake,
                    [0; 3]
                )
            );
        }
        for (x, y) in [(-2.0, 3.0), (0.0, 0.0), (1.2, -0.5)] {
            assert!((noise(x - 1e-6, y) - noise(x + 1e-6, y)).abs() < 1e-4);
        }
    }
}
