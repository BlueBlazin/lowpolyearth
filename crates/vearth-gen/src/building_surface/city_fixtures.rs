//! Finite facade attachments: source identity owns placement and geometric LOD.
use super::{Projection, SurfaceAppearance, emit_quad};
use glam::{DVec2, DVec3};
use std::{collections::HashMap, sync::OnceLock};
use vearth_core::mesh::MeshBuffers;

mod generated;

struct Fixture {
    seed: u64,
    lat: f64,
    lon: f64,
    width: f64,
    height: f64,
    bottom: f64,
    color: [u8; 3],
    design: u8,
    outward: [f64; 2],
}

fn fixtures(seed: u64) -> &'static [&'static Fixture] {
    static INDEX: OnceLock<HashMap<u64, Vec<&'static Fixture>>> = OnceLock::new();
    INDEX
        .get_or_init(|| {
            let mut result: HashMap<u64, Vec<&'static Fixture>> = HashMap::new();
            for f in generated::FIXTURES {
                result.entry(f.seed).or_default().push(f);
            }
            result
        })
        .get(&seed)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

pub(super) fn has_fixtures(seed: u64) -> bool {
    !fixtures(seed).is_empty()
}

pub(super) fn attachment_radius_m(seed: u64) -> f64 {
    fixtures(seed)
        .iter()
        .map(|f| {
            // Projecting housings reach width + 0.48 m out from the source wall;
            // their 0.22 m half-thickness also contributes at oblique cell edges.
            if f.design == 3 {
                (f.width + 0.48).hypot(0.22)
            } else {
                1.2
            }
        })
        .fold(0.0, f64::max)
}

pub(super) fn append(
    projection: &Projection<'_>,
    mesh: &mut MeshBuffers,
    seed: u64,
    boundary: &[[DVec2; 2]],
    floor: f64,
    top: f64,
) {
    // At coarser pitches the mapped facade geometry is intentionally replaced
    // by its massing. Never leave detached lights over that representation.
    if projection.frame.addr.cell.level < 16 {
        return;
    }
    for f in fixtures(seed) {
        let anchor = DVec2::new(
            (f.lon.to_radians() - projection.lon)
                * vearth_core::ellipsoid::MEAN_RADIUS
                * projection.lat.cos(),
            (f.lat.to_radians() - projection.lat) * vearth_core::ellipsoid::MEAN_RADIUS,
        );
        let candidate = boundary
            .iter()
            .filter_map(|edge| {
                let v = edge[1] - edge[0];
                let length = v.length();
                if length < 1.0 {
                    return None;
                }
                let t = ((anchor - edge[0]).dot(v) / (length * length)).clamp(0.0, 1.0);
                let point = edge[0] + v * t;
                Some((
                    (point - anchor).length_squared(),
                    point,
                    v / length,
                    length,
                    t,
                ))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0));
        let Some((distance, center, tangent, length, t)) = candidate else {
            continue;
        };
        // A source update or an occluded parent face must fail closed, rather
        // than relocating a sign to another wall or into a neighbouring block.
        if distance > 9.0 {
            continue;
        }
        let width = f.width.min(length * 0.94);
        if width < 1.0 {
            continue;
        }
        let half = width * 0.5;
        let center =
            center + tangent * ((t * length).clamp(half + 0.02, length - half - 0.02) - t * length);
        let mut normal = DVec2::new(-tangent.y, tangent.x);
        if normal.dot(DVec2::from_array(f.outward)) < 0.0 {
            normal = -normal;
        }
        let height = f.height.min(top - floor - f.bottom - 0.25);
        if height < 0.5 {
            continue;
        }
        let bottom = floor + f.bottom;
        let shade = SurfaceAppearance {
            material: crate::material::STEEL,
            tint: [24, 28, 34],
        };
        let glow = SurfaceAppearance {
            material: crate::material::CITY_LIGHT,
            tint: f.color,
        };
        let projecting = f.design == 3;
        let mut panel = if projecting {
            Panel {
                projection,
                mesh,
                center: center + normal * (half + 0.3),
                tangent: normal,
                normal: tangent,
                bottom,
            }
        } else {
            Panel {
                projection,
                mesh,
                center,
                tangent,
                normal,
                bottom,
            }
        };
        if f.design == 4 {
            // Architectural wash follows a selected building's major datums.
            for y in [0.0, height * 0.5, height] {
                panel.rect(-half, y, half, y + 0.30, 1.05, glow);
            }
            let bays = (width / 5.0).round().max(2.0) as usize;
            for bay in 0..=bays {
                let x = -half + width * bay as f64 / bays as f64;
                panel.rect(x - 0.16, 0.0, x + 0.16, height, 1.08, glow);
            }
            continue;
        }
        if projecting {
            panel.slab(
                -half - 0.18,
                -0.18,
                half + 0.18,
                height + 0.18,
                -0.20,
                0.20,
                shade,
            );
            for side in [-1.0, 1.0] {
                panel.normal = tangent * side;
                panel.rect(-half, 0.0, -half + 0.18, height, 0.22, glow);
                panel.rect(half - 0.18, 0.0, half, height, 0.22, glow);
                panel.rect(-half, 0.0, half, 0.18, 0.22, glow);
                panel.rect(-half, height - 0.18, half, height, 0.22, glow);
                for row in 0..4 {
                    let y = height * (row as f64 + 0.15) / 4.0;
                    panel.rect(
                        -half * 0.65,
                        y + 0.25,
                        half * 0.65,
                        y + height * 0.15,
                        0.22,
                        glow,
                    );
                }
            }
            panel.normal = tangent;
            for y in [0.0, height * 0.8] {
                panel.slab(-half - 0.4, y, -half, y + 0.15, -0.12, 0.12, shade);
            }
            continue;
        }
        panel.slab(
            -half - 0.18,
            -0.18,
            half + 0.18,
            height + 0.18,
            0.90,
            1.08,
            shade,
        );
        let rim = 0.16_f64.min(width * 0.06);
        if f.design == 0 {
            panel.rect(-half, 0.0, half, height, 1.10, glow);
            // Broad original color fields remain readable after downsampling.
            let secondary = SurfaceAppearance {
                material: glow.material,
                tint: f.color.map(|c| (f32::from(c) * 0.56) as u8),
            };
            panel.rect(-half, height * 0.70, half, height * 0.82, 1.12, secondary);
        } else {
            panel.rect(-half, 0.0, -half + rim, height, 1.12, glow);
            panel.rect(half - rim, 0.0, half, height, 1.12, glow);
            panel.rect(-half, 0.0, half, rim, 1.12, glow);
            panel.rect(-half, height - rim, half, height, 1.12, glow);
            let count = if f.design == 2 { 5 } else { 3 };
            for row in 0..count {
                let y = height * (row as f64 + 0.2) / count as f64;
                panel.rect(
                    -half * 0.65,
                    y + rim,
                    half * 0.65,
                    y + height * 0.58 / count as f64,
                    1.12,
                    glow,
                );
            }
        }
        // Physical stand-offs connect the housing to the source wall.
        for x in [-half * 0.7, half * 0.7] {
            panel.slab(x - 0.08, 0.0, x + 0.08, height, 0.0, 0.90, shade);
        }
    }
}

struct Panel<'a, 'b> {
    projection: &'a Projection<'b>,
    mesh: &'a mut MeshBuffers,
    center: DVec2,
    tangent: DVec2,
    normal: DVec2,
    bottom: f64,
}
impl Panel<'_, '_> {
    fn point(&self, x: f64, y: f64, depth: f64) -> DVec3 {
        self.projection.point(
            self.center + self.tangent * x + self.normal * depth,
            self.bottom + y,
        )
    }
    fn face(&mut self, coordinates: [[f64; 3]; 4], appearance: SurfaceAppearance) {
        let points = coordinates.map(|[x, y, d]| self.point(x, y, d));
        // Metric ENU is left-handed; choose winding from the actual projected
        // outward direction so signs remain visible with normal backface culling.
        let mut points = points;
        let expected = self.point(0.0, 0.0, 2.0) - self.point(0.0, 0.0, 1.0);
        if (points[1] - points[0])
            .cross(points[2] - points[0])
            .dot(expected)
            < 0.0
        {
            points.reverse();
        }
        emit_quad(self.mesh, points, appearance, false);
    }
    fn rect(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, d: f64, a: SurfaceAppearance) {
        self.face([[x0, y0, d], [x1, y0, d], [x1, y1, d], [x0, y1, d]], a);
    }
    fn slab(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, d0: f64, d1: f64, a: SurfaceAppearance) {
        self.rect(x0, y0, x1, y1, d1, a);
        let mut back = [
            self.point(x0, y0, d0),
            self.point(x1, y0, d0),
            self.point(x1, y1, d0),
            self.point(x0, y1, d0),
        ];
        let expected = self.point(0.0, 0.0, d0 - 1.0) - self.point(0.0, 0.0, d0);
        if (back[1] - back[0]).cross(back[2] - back[0]).dot(expected) < 0.0 {
            back.reverse();
        }
        emit_quad(self.mesh, back, a, false);
        // Side housings are emitted in both windings; they are narrow opaque
        // closure faces and must read from either side of a projecting sign.
        for coords in [
            [[x0, y0, d0], [x0, y0, d1], [x0, y1, d1], [x0, y1, d0]],
            [[x1, y0, d1], [x1, y0, d0], [x1, y1, d0], [x1, y1, d1]],
            [[x0, y0, d0], [x1, y0, d0], [x1, y0, d1], [x0, y0, d1]],
            [[x0, y1, d1], [x1, y1, d1], [x1, y1, d0], [x0, y1, d0]],
        ] {
            let p = coords.map(|[x, y, d]| self.point(x, y, d));
            emit_quad(self.mesh, p, a, false);
            emit_quad(self.mesh, [p[3], p[2], p[1], p[0]], a, false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vearth_core::{
        cell::{CellId, ChunkAddr},
        ellipsoid,
        frame::ChunkFrame,
    };

    fn frame(f: &Fixture, level: u8) -> ChunkFrame {
        let cell = CellId::from_dir(
            ellipsoid::geodetic_to_ecef(f.lat.to_radians(), f.lon.to_radians(), 0.0).normalize(),
            level,
        );
        ChunkFrame::of(ChunkAddr { cell, k: 0 })
    }

    #[test]
    fn catalog_attachments_are_owned_static_and_clipped() {
        for design in 0..=4 {
            let fixture = generated::FIXTURES
                .iter()
                .find(|f| f.design == design)
                .unwrap();
            let frame = frame(fixture, 16);
            let projection = Projection {
                frame: &frame,
                lat: fixture.lat.to_radians(),
                lon: fixture.lon.to_radians(),
            };
            let outward = DVec2::from_array(fixture.outward).normalize();
            let tangent = DVec2::new(outward.y, -outward.x);
            let boundary = [[-tangent * 100.0, tangent * 100.0]];
            let generate = |seed, boundary: &[[DVec2; 2]]| {
                let mut mesh = MeshBuffers::default();
                append(&projection, &mut mesh, seed, boundary, 0.0, 200.0);
                mesh
            };
            let mesh = generate(fixture.seed, &boundary);
            assert!(
                mesh.vertices
                    .iter()
                    .any(|v| (v.face_ao >> 5) & 255 == u32::from(crate::material::CITY_LIGHT)),
                "design {design} emits no light"
            );
            assert!(
                mesh.vertices
                    .iter()
                    .all(|v| v.pos.iter().all(|p| p.is_finite())
                        && (0.0..=32.0).contains(&v.pos[0])
                        && (0.0..=32.0).contains(&v.pos[2]))
            );
            let repeat = generate(fixture.seed, &boundary);
            assert_eq!(mesh.vertices, repeat.vertices);
            assert_eq!(mesh.indices, repeat.indices);
            assert!(
                generate(0, &boundary).indices.is_empty(),
                "unrelated source got a sign"
            );
            let moved = boundary.map(|edge| edge.map(|p| p + outward * 1000.0));
            assert!(
                generate(fixture.seed, &moved).indices.is_empty(),
                "relocated source kept a detached sign"
            );
            let mut hidden = MeshBuffers::default();
            append(&projection, &mut hidden, fixture.seed, &[], 0.0, 200.0);
            assert!(hidden.indices.is_empty(), "occluded source kept a sign");
        }
    }

    #[test]
    fn coarse_source_massing_cannot_leave_floating_signs() {
        let fixture = &generated::FIXTURES[0];
        let frame = frame(fixture, 15);
        let projection = Projection {
            frame: &frame,
            lat: fixture.lat.to_radians(),
            lon: fixture.lon.to_radians(),
        };
        let mut mesh = MeshBuffers::default();
        append(
            &projection,
            &mut mesh,
            fixture.seed,
            &[[DVec2::new(-100.0, 0.0), DVec2::new(100.0, 0.0)]],
            0.0,
            200.0,
        );
        assert!(mesh.indices.is_empty());
    }

    #[test]
    fn projecting_sign_crosses_cell_seam_without_its_parent_footprint() {
        use crate::{N, building_surface, tier0};
        use vearth_core::ellipsoid::MEAN_RADIUS;
        use vearth_data::vectors::{Building, BuildingKind, FeaturePack};

        // Find a real authored fixture whose outward tip is in the next cell.
        // A narrow synthetic source wall isolates admission from neighbouring
        // city geometry while retaining the catalog's real anchor and art.
        for level in [19, 20] {
            for f in generated::FIXTURES.iter().filter(|f| f.design == 3) {
                let outward = DVec2::from_array(f.outward).normalize();
                let tangent = DVec2::new(outward.y, -outward.x);
                let lat = f.lat.to_radians();
                let lon = f.lon.to_radians();
                let locate = |p: DVec2| {
                    (
                        lat + p.y / MEAN_RADIUS,
                        lon + p.x / (MEAN_RADIUS * lat.cos()),
                    )
                };
                let (tip_lat, tip_lon) = locate(outward * 2.3);
                let cell = CellId::from_dir(
                    ellipsoid::geodetic_to_ecef(tip_lat, tip_lon, 0.0).normalize(),
                    level,
                );
                let frame = ChunkFrame::of(ChunkAddr { cell, k: 0 });
                let (center_lat, center_lon) = frame.column_latlon(0.5, 0.5);
                let source_points = [
                    -tangent * 1.25,
                    tangent * 1.25,
                    tangent * 1.25 - outward,
                    -tangent * 1.25 - outward,
                ];
                let ring: Vec<_> = source_points
                    .map(|p| {
                        let (lat, lon) = locate(p);
                        [
                            ((lon - center_lon) * MEAN_RADIUS * center_lat.cos()) as f32,
                            ((lat - center_lat) * MEAN_RADIUS) as f32,
                        ]
                    })
                    .into();
                let projection = tier0::GridProj::of(&frame, center_lat, center_lon);
                let cols: Vec<_> = ring.iter().map(|p| projection.to_cols(*p)).collect();
                if !(cols.iter().all(|p| p[0] < 0.0)
                    || cols.iter().all(|p| p[0] > N as f32)
                    || cols.iter().all(|p| p[1] < 0.0)
                    || cols.iter().all(|p| p[1] > N as f32))
                {
                    continue;
                }
                let mut pack = FeaturePack::empty();
                pack.buildings.push(Building {
                    location_rad: None,
                    roof_shape: None,
                    use_kind: Default::default(),
                    rings: vec![ring],
                    height_m: 200.0,
                    min_height_m: 0.0,
                    seed: f.seed,
                    kind: BuildingKind::Building,
                });
                let overlay =
                    tier0::build_overlay(cell, &frame, &pack, &|_, _| 10.0, 0, None, None, None);
                assert_eq!(
                    overlay.source_buildings.len(),
                    1,
                    "catalog source outside adjacent cell was discarded"
                );
                assert!(
                    overlay.cols.iter().flatten().all(|c| c.building.is_none()),
                    "source must not stamp outside its footprint"
                );
                let footprint = building_surface::datum_cover(&frame, &pack);
                assert!(
                    !footprint.is_empty(),
                    "adjacent cell needs source datum coverage"
                );
                for p in source_points {
                    let (lat, lon) = locate(p);
                    assert!(footprint.contains(lat, lon));
                }
                let surface = building_surface::generate(
                    &frame,
                    &pack,
                    &overlay,
                    &[u32::MAX; N],
                    &|_, _, _| Some(10.0),
                    &|_, _, _| None,
                )
                .expect("admitted attachment source produces a surface batch");
                assert!(
                    surface.complete,
                    "adjacent cell source datum or envelope failed"
                );
                assert!(
                    surface
                        .mesh
                        .vertices
                        .iter()
                        .any(|v| (v.face_ao >> 5) & 255 == u32::from(crate::material::CITY_LIGHT)),
                    "source was retained but projecting light disappeared at the seam"
                );
                // Removing catalog ownership restores exact source admission.
                pack.buildings[0].seed = 0;
                let ordinary =
                    tier0::build_overlay(cell, &frame, &pack, &|_, _| 10.0, 0, None, None, None);
                assert!(ordinary.source_buildings.is_empty());
                assert!(building_surface::datum_cover(&frame, &pack).is_empty());
                return;
            }
        }
        panic!("catalog must supply a projecting fixture crossing a tested cell seam");
    }
}
