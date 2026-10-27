//! Object-local tree surfaces with a shared silhouette across geometric LODs.
//!
//! Crowns are opaque volumes. Their cost follows their visible forms, rather
//! than the number of occupied voxels inside a canopy. Profile material ids
//! retain the renderer's seasonal and wind response.

use crate::tier0::NearTreeMesh;
use crate::vegetation::{
    CrownGrammar, FoliageSeasonClass, RootStyle, TreeProfile, TreeProfileId, tree_profile,
};
use glam::Vec3;
use std::f32::consts::{PI, TAU};
use vearth_core::mesh::{MeshBuffers, SurfaceVertex, encode_surface_normal};

#[derive(Clone, Copy)]
struct Lobe {
    centre: Vec3,
    radius: Vec3,
    phase: f32,
    shade: f32,
}

struct Builder {
    mesh: MeshBuffers,
    profile: &'static TreeProfile,
    lod: u8,
    height: f32,
    radius: f32,
    trunk: f32,
    phase: f32,
    bark: [f32; 3],
    leaves: [f32; 3],
}

fn palette(profile: &TreeProfile) -> ([f32; 3], [f32; 3]) {
    use FoliageSeasonClass as Season;
    let (leaf_anchor, leaf_mix) = match profile.foliage.season {
        Season::EvergreenNeedle => ([76.0, 103.0, 78.0], 0.74),
        Season::DeciduousConifer => ([111.0, 136.0, 89.0], 0.70),
        Season::EvergreenBroadleaf => ([95.0, 123.0, 78.0], 0.68),
        Season::TropicalBroadleaf => ([107.0, 137.0, 78.0], 0.70),
        Season::DroughtDeciduous => ([142.0, 145.0, 93.0], 0.68),
        Season::Palm => ([137.0, 148.0, 78.0], 0.72),
        Season::DeciduousYellow
        | Season::DeciduousOrange
        | Season::DeciduousRed
        | Season::SpringFlowering
        | Season::OrchardFlowering => ([125.0, 145.0, 80.0], 0.76),
    };
    let light_bark = profile.bark.iter().map(|&v| u32::from(v)).sum::<u32>() > 470;
    let (bark_anchor, bark_mix) = if light_bark {
        ([191.0, 190.0, 173.0], 0.24)
    } else {
        ([109.0, 95.0, 77.0], 0.68)
    };
    let blend = |source: [u8; 3], anchor: [f32; 3], mix: f32| {
        std::array::from_fn(|i| f32::from(source[i]) * (1.0 - mix) + anchor[i] * mix)
    };
    (
        blend(profile.bark, bark_anchor, bark_mix),
        blend(profile.leaves, leaf_anchor, leaf_mix),
    )
}

fn hash01(mut value: u32) -> f32 {
    value ^= value >> 16;
    value = value.wrapping_mul(0x7feb_352d);
    value ^= value >> 15;
    value = value.wrapping_mul(0x846c_a68b);
    value ^= value >> 16;
    (value >> 8) as f32 / 16_777_216.0
}

fn radial(angle: f32) -> Vec3 {
    Vec3::new(angle.cos(), 0.0, angle.sin())
}

impl Builder {
    fn triangle(
        &mut self,
        a: Vec3,
        mut b: Vec3,
        mut c: Vec3,
        outward: Vec3,
        leaf: bool,
        shade: f32,
    ) {
        let mut normal = (b - a).cross(c - a);
        if normal.length_squared() < 1.0e-12 {
            return;
        }
        if normal.dot(outward) < 0.0 {
            std::mem::swap(&mut b, &mut c);
            normal = -normal;
        }
        let normal = normal.normalize();
        let centre = (a + b + c) / 3.0;
        let height = (centre.y / self.height).clamp(0.0, 1.0);
        let source = if leaf { self.leaves } else { self.bark };
        let value = shade
            * if leaf {
                0.91 + 0.12 * height
            } else {
                0.94 + 0.06 * height
            };
        let color =
            std::array::from_fn::<_, 3, _>(|i| (source[i] * value).round().clamp(0.0, 255.0) as u8);
        let axis = normal.abs().max_element();
        let face = if normal.x.abs() == axis {
            u32::from(normal.x < 0.0)
        } else if normal.y.abs() == axis {
            2 + u32::from(normal.y < 0.0)
        } else {
            4 + u32::from(normal.z < 0.0)
        };
        // Only sheltered undersides and trunk contact receive local occlusion;
        // sunlight is evaluated from the authored polygon normal at runtime.
        let ao = if (leaf && normal.y < -0.25) || (!leaf && height < 0.045) {
            2
        } else {
            3
        };
        let material = if leaf {
            crate::material::foliage(self.profile.foliage)
        } else {
            crate::material::TRUNK
        };
        let face_ao = encode_surface_normal(
            face | (ao << 3) | (u32::from(material) << 5),
            normal.to_array(),
        );
        let first = self.mesh.vertices.len() as u32;
        self.mesh
            .vertices
            .extend([a, b, c].map(|point| SurfaceVertex {
                pos: point.to_array(),
                face_ao,
                rgba: [color[0], color[1], color[2], 255],
            }));
        self.mesh.indices.extend([first, first + 1, first + 2]);
    }

    fn tube(&mut self, path: &[(Vec3, f32)], sides: usize, shade: f32) {
        let mut rings = Vec::with_capacity(path.len());
        for (i, &(centre, radius)) in path.iter().enumerate() {
            let previous = path[i.saturating_sub(1)].0;
            let next = path[(i + 1).min(path.len() - 1)].0;
            let tangent = (next - previous).normalize_or_zero();
            let reference = if tangent.x.abs() > 0.9 {
                Vec3::Z
            } else {
                Vec3::X
            };
            let u = tangent.cross(reference).normalize();
            let v = tangent.cross(u);
            rings.push(
                (0..sides)
                    .map(|j| {
                        let angle = TAU * j as f32 / sides as f32 + self.phase;
                        let mut p = centre + radius * (u * angle.cos() + v * angle.sin());
                        p.y = p.y.max(0.0);
                        p
                    })
                    .collect::<Vec<_>>(),
            );
        }
        for i in 0..path.len() - 1 {
            for j in 0..sides {
                let k = (j + 1) % sides;
                let [a, b, c, d] = [rings[i][j], rings[i][k], rings[i + 1][k], rings[i + 1][j]];
                let outward = (a + b + c + d) * 0.25 - (path[i].0 + path[i + 1].0) * 0.5;
                self.triangle(a, b, c, outward, false, shade);
                self.triangle(a, c, d, outward, false, shade);
            }
        }
        for (index, other) in [(0, 1), (path.len() - 1, path.len() - 2)] {
            let outward = path[index].0 - path[other].0;
            for j in 0..sides {
                self.triangle(
                    path[index].0,
                    rings[index][j],
                    rings[index][(j + 1) % sides],
                    outward,
                    false,
                    shade,
                );
            }
        }
    }

    fn stem_at(&self, fraction: f32) -> Vec3 {
        let palm = self.profile.grammar == CrownGrammar::Palm;
        let lean = if palm && self.profile.variant == 0 {
            0.21 * self.height
        } else if self.forked_crown() {
            0.27 * self.radius
        } else {
            0.13 * self.radius
        };
        let direction = radial(self.phase + 0.5);
        let curve = if palm {
            fraction * fraction
        } else {
            fraction + 0.32 * (fraction * PI * 1.6).sin()
        };
        direction * (lean * curve) + Vec3::Y * (fraction * self.height)
    }

    fn stem(&mut self, top: f32) {
        let segments = [6, 3, 2][self.lod as usize];
        let sides = [8, 6, 4][self.lod as usize];
        let palm = self.profile.grammar == CrownGrammar::Palm;
        let baobab = self.profile.grammar == CrownGrammar::Baobab;
        let path = (0..=segments)
            .map(|i| {
                let t = i as f32 / segments as f32;
                let flare = (1.0 - t * 5.0).max(0.0) * if baobab { 0.14 } else { 0.48 };
                let taper = if palm {
                    0.94 - t * 0.26
                } else if baobab {
                    1.10 - t * 0.44
                } else if self.forked_crown() {
                    1.0 - t * 0.65
                } else {
                    1.0 - t * 0.79
                };
                (self.stem_at(t * top), self.trunk * (taper + flare))
            })
            .collect::<Vec<_>>();
        self.tube(&path, sides, 1.0);
        if self.lod == 0 && !palm {
            self.buttresses();
        }
    }

    fn buttresses(&mut self) {
        let count = if self.profile.grammar == CrownGrammar::Emergent {
            6
        } else {
            4
        };
        for i in 0..count {
            let direction = radial(self.phase + TAU * i as f32 / count as f32);
            let side = Vec3::new(-direction.z, 0.0, direction.x);
            let reach = self.trunk
                * if self.profile.grammar == CrownGrammar::Emergent {
                    3.6
                } else {
                    2.5
                };
            let a = side * self.trunk * 0.65;
            let b = -a;
            let c = direction * reach - side * self.trunk * 0.11;
            let d = direction * reach + side * self.trunk * 0.11;
            let e = direction * self.trunk * 0.3 + Vec3::Y * self.trunk * 1.9;
            self.triangle(a, b, e, -direction, false, 0.96);
            self.triangle(b, c, e, -side, false, 0.96);
            self.triangle(c, d, e, direction, false, 1.0);
            self.triangle(d, a, e, side, false, 1.0);
            self.triangle(a, d, c, -Vec3::Y, false, 0.96);
            self.triangle(a, c, b, -Vec3::Y, false, 0.96);
        }
    }

    fn branch(&mut self, end: Vec3, index: usize, fork: f32) {
        let start = self.stem_at(fork);
        let delta = end - start;
        let forked = self.forked_crown();
        let middle = start
            + Vec3::new(
                delta.x * 0.55,
                delta.y * if forked { 0.46 } else { 0.34 },
                delta.z * 0.55,
            );
        let thickness = self.trunk
            * (if forked { 0.63 } else { 0.42 }
                + hash01(index as u32 + self.profile.id as u32 * 19) * 0.12);
        let sides = [6, 5, 3][self.lod as usize];
        if self.lod == 2 {
            self.tube(&[(start, thickness), (end, thickness * 0.20)], sides, 1.0);
        } else {
            self.tube(
                &[
                    (start, thickness),
                    (middle, thickness * if forked { 0.78 } else { 0.66 }),
                    (end, thickness * 0.16),
                ],
                sides,
                1.0,
            );
        }
        if self.lod == 0 {
            let tangent = Vec3::new(-delta.z, 0.0, delta.x).normalize_or_zero();
            let twig = end + tangent * self.radius * 0.12 + Vec3::Y * self.height * 0.065;
            self.tube(
                &[
                    (middle.lerp(end, 0.62), thickness * 0.23),
                    (twig, thickness * 0.035),
                ],
                4,
                1.03,
            );
        }
    }

    fn lobe(&mut self, lobe: Lobe) {
        let golden = (1.0 + 5.0_f32.sqrt()) / 2.0;
        let points = [
            Vec3::new(-1.0, golden, 0.0),
            Vec3::new(1.0, golden, 0.0),
            Vec3::new(-1.0, -golden, 0.0),
            Vec3::new(1.0, -golden, 0.0),
            Vec3::new(0.0, -1.0, golden),
            Vec3::new(0.0, 1.0, golden),
            Vec3::new(0.0, -1.0, -golden),
            Vec3::new(0.0, 1.0, -golden),
            Vec3::new(golden, 0.0, -1.0),
            Vec3::new(golden, 0.0, 1.0),
            Vec3::new(-golden, 0.0, -1.0),
            Vec3::new(-golden, 0.0, 1.0),
        ]
        .map(Vec3::normalize);
        let faces = [
            [0, 11, 5],
            [0, 5, 1],
            [0, 1, 7],
            [0, 7, 10],
            [0, 10, 11],
            [1, 5, 9],
            [5, 11, 4],
            [11, 10, 2],
            [10, 7, 6],
            [7, 1, 8],
            [3, 9, 4],
            [3, 4, 2],
            [3, 2, 6],
            [3, 6, 8],
            [3, 8, 9],
            [4, 9, 5],
            [2, 4, 11],
            [6, 2, 10],
            [8, 6, 7],
            [9, 8, 1],
        ];
        for [a, b, c] in faces.map(|indices| indices.map(|i| points[i])) {
            if self.lod < 2 {
                let ab = (a + b).normalize();
                let bc = (b + c).normalize();
                let ca = (c + a).normalize();
                for triangle in [[a, ab, ca], [b, bc, ab], [c, ca, bc], [ab, bc, ca]] {
                    self.lobe_triangle(lobe, triangle);
                }
            } else {
                // Expand the inscribed icosahedron to retain crown coverage at
                // distance; lobe centres and branch endpoints never move.
                self.lobe_triangle(
                    Lobe {
                        radius: lobe.radius * 1.055,
                        ..lobe
                    },
                    [a, b, c],
                );
            }
        }
    }

    fn lobe_triangle(&mut self, lobe: Lobe, points: [Vec3; 3]) {
        let (sin, cos) = lobe.phase.sin_cos();
        let rotate = |v: Vec3| Vec3::new(v.x * cos - v.z * sin, v.y, v.x * sin + v.z * cos);
        let world = points.map(|p| {
            // Broad deformations form a few irregular planes, without tiny
            // vertex noise or independently moving patches of foliage.
            let shape = if self.forked_crown() {
                1.0 + 0.11 * p.x + 0.055 * p.y * p.z
            } else {
                1.0 + 0.035 * (p.x * 3.0 + lobe.phase).sin() * p.z
            };
            let drape = if self.profile.grammar == CrownGrammar::Weeping {
                0.85 + 0.15 * p.y
            } else {
                1.0
            };
            let skew = if self.forked_crown() {
                Vec3::new(0.17 * p.y, 0.055 * p.x, 0.12 * p.x * p.y) * lobe.radius
            } else {
                Vec3::ZERO
            };
            lobe.centre + rotate((p * lobe.radius * Vec3::new(drape, 1.0, drape) + skew) * shape)
        });
        let centre = (world[0] + world[1] + world[2]) / 3.0;
        self.triangle(
            world[0],
            world[1],
            world[2],
            centre - lobe.centre,
            true,
            lobe.shade,
        );
    }

    fn broadleaf(&mut self) {
        if self.forked_crown() {
            self.forked_broadleaf();
            return;
        }
        let grammar = self.profile.grammar;
        let h = self.height;
        let r = self.radius;
        let base = self.profile.crown_base;
        let columnar = grammar == CrownGrammar::Columnar;
        let umbrella = grammar == CrownGrammar::Umbrella;
        let open = matches!(
            grammar,
            CrownGrammar::Open | CrownGrammar::Baobab | CrownGrammar::Pine
        );
        let weeping = grammar == CrownGrammar::Weeping;
        let crown_height = 1.0 - base;
        let fork = if columnar {
            0.30
        } else {
            (base * 0.78).clamp(0.22, 0.56)
        };
        self.stem(if columnar { 0.94 } else { 0.85 });
        let mut lobes = Vec::new();
        let top_ry = if umbrella {
            0.105
        } else if columnar {
            crown_height * 0.29
        } else {
            crown_height * 0.28
        };
        lobes.push(Lobe {
            centre: self.stem_at(0.98 - top_ry),
            radius: Vec3::new(
                r * if columnar { 0.83 } else { 0.54 },
                h * top_ry,
                r * if columnar { 0.77 } else { 0.52 },
            ),
            phase: self.phase,
            shade: 1.04,
        });
        let count = if columnar { 4 } else { 5 };
        for i in 0..count {
            let seed = self.profile.id as u32 * 37 + i as u32 * 11;
            let phase = self.phase + TAU * i as f32 / count as f32 + (hash01(seed) - 0.5) * 0.25;
            let t = i as f32 / count as f32;
            let elevation = if columnar {
                base + crown_height * (0.24 + 0.35 * t)
            } else if umbrella {
                0.80 + 0.035 * (phase * 2.0).sin()
            } else if open {
                base + crown_height * (0.46 + 0.26 * hash01(seed + 7))
            } else {
                base + crown_height * (0.42 + 0.17 * hash01(seed + 7))
            };
            let offset = r * if columnar {
                0.32
            } else if open {
                0.66
            } else {
                0.57
            };
            let centre = self.stem_at(elevation) + radial(phase) * offset;
            let width = if columnar {
                0.61
            } else if open {
                0.40
            } else if grammar == CrownGrammar::Spreading {
                0.47 + hash01(seed + 9) * 0.06
            } else {
                0.53
            };
            let ry = if columnar {
                crown_height * 0.29
            } else if umbrella {
                0.12
            } else if weeping {
                0.34
            } else {
                crown_height * 0.27
            };
            let lobe = Lobe {
                centre,
                radius: Vec3::new(
                    r * width,
                    h * ry,
                    r * width * (0.89 + hash01(seed + 3) * 0.13),
                ),
                phase,
                shade: 0.96 + hash01(seed + 2) * 0.08,
            };
            self.branch(
                centre - Vec3::Y * (lobe.radius.y * 0.22),
                i,
                fork + t * 0.08,
            );
            lobes.push(lobe);
        }
        if !columnar && !umbrella && !open {
            let direction = radial(self.phase + 2.2);
            lobes.push(Lobe {
                centre: self.stem_at(base + crown_height * 0.72) + direction * r * 0.46,
                radius: Vec3::new(r * 0.48, h * crown_height * 0.22, r * 0.43),
                phase: self.phase + 0.7,
                shade: 1.035,
            });
        }
        for lobe in &lobes {
            self.lobe(*lobe);
        }
        self.support_roots(&lobes);
    }

    fn forked_crown(&self) -> bool {
        matches!(
            self.profile.grammar,
            CrownGrammar::Round | CrownGrammar::Spreading
        )
    }

    fn forked_broadleaf(&mut self) {
        let h = self.height;
        let r = self.radius;
        let base = self.profile.crown_base;
        let crown_height = 1.0 - base;
        let spreading = self.profile.grammar == CrownGrammar::Spreading;
        let fork = (base * 0.95).clamp(0.28, 0.44);
        self.stem(0.84);
        let right = radial(self.phase - PI * 0.5);
        let depth = radial(self.phase);
        let top_ry = crown_height * 0.295;
        let mut lobes = vec![Lobe {
            centre: self.stem_at(0.985 - top_ry) - right * r * 0.10 - depth * r * 0.06,
            radius: Vec3::new(r * 0.58, h * top_ry, r * 0.46),
            phase: self.phase - PI * 0.5,
            shade: 1.04,
        }];
        // One broad low limb balances two unequal opposite limbs. The smaller
        // rear mass adds depth without turning the crown into a ring of orbs.
        // Coordinates and attachment points are shared by LODs and collision.
        let layout = [
            // side, depth, elevation within crown, width, height, breadth
            (-0.75, 0.02, 0.36, 0.48, 0.20, 0.78),
            (0.65, -0.18, 0.54, 0.41, 0.19, 0.90),
            (0.91, 0.17, 0.31, 0.32, 0.16, 0.91),
            (-0.29, -0.32, 0.56, 0.30, 0.21, 0.89),
        ];
        for (i, (side, back, level, width, ry, breadth)) in layout.into_iter().enumerate() {
            let seed = self.profile.id as u32 * 37 + i as u32 * 11;
            let phase = self.phase - PI * 0.5 + (hash01(seed) - 0.5) * 0.65;
            let elevation = base + crown_height * (level + (hash01(seed + 7) - 0.5) * 0.035);
            let centre = self.stem_at(elevation)
                + right * r * side * if spreading { 1.0 } else { 0.86 }
                + depth * r * back;
            let width = width * (0.96 + hash01(seed + 9) * 0.08);
            let lobe = Lobe {
                centre,
                radius: Vec3::new(r * width, h * crown_height * ry, r * width * breadth),
                phase,
                shade: 0.96 + hash01(seed + 2) * 0.08,
            };
            self.branch(
                centre - Vec3::Y * (lobe.radius.y * 0.18),
                i,
                fork + (i % 2) as f32 * 0.075,
            );
            lobes.push(lobe);
        }
        for lobe in &lobes {
            self.lobe(*lobe);
        }
        self.support_roots(&lobes);
    }

    fn support_roots(&mut self, lobes: &[Lobe]) {
        let count = match self.profile.root_style {
            RootStyle::None => return,
            RootStyle::Banyan => [5, 3, 2][self.lod as usize],
            RootStyle::Mangrove => [6, 4, 3][self.lod as usize],
        };
        for i in 0..count {
            let direction = radial(self.phase + TAU * i as f32 / count as f32);
            let radius = self.trunk * 0.19;
            let path = if self.profile.root_style == RootStyle::Banyan {
                let top = lobes[1 + i % (lobes.len() - 1)].centre;
                [
                    (Vec3::new(top.x * 0.96, 0.0, top.z * 0.96), radius * 1.25),
                    (top, radius),
                ]
            } else {
                [
                    (direction * self.trunk * 3.8, radius * 0.55),
                    (
                        self.stem_at(0.17) + direction * self.trunk * 0.7,
                        radius * 1.9,
                    ),
                ]
            };
            self.tube(&path, if self.lod == 2 { 3 } else { 5 }, 0.95);
        }
    }

    fn conifer(&mut self) {
        self.stem(0.965);
        let tiers = if self.profile.id == TreeProfileId::Redwood {
            8
        } else {
            7
        };
        let base = self.profile.crown_base;
        for tier in 0..tiers {
            let t = tier as f32 / tiers as f32;
            let y = self.height * (base + (0.94 - base) * t);
            let radius = self.radius * (1.0 - t).powf(0.88);
            let length = if tier + 1 == tiers {
                self.height * 0.995 - y
            } else {
                self.height * (0.24 - 0.13 * t)
            };
            let centre = self.stem_at(y / self.height);
            let phase = self.phase + tier as f32 * 0.61;
            let sides = [12, 10, 8][self.lod as usize];
            let top = centre + Vec3::Y * length;
            let bottom = centre + Vec3::Y * length * 0.16;
            let edge = |j: usize| {
                let angle = phase + TAU * j as f32 / sides as f32;
                let r = radius * if j % 2 == 0 { 1.0 } else { 0.77 };
                centre + radial(angle) * r + Vec3::Y * length * if j % 2 == 0 { 0.0 } else { 0.12 }
            };
            for j in 0..sides {
                let next = (j + 1) % sides;
                let a = edge(j);
                let b = edge(next);
                let shade = 0.96 + tier as f32 / tiers as f32 * 0.09;
                if self.lod == 2 {
                    self.triangle(top, a, b, Vec3::Y + (a - centre) / radius, true, shade);
                } else {
                    let inner_a = centre + (a - centre) * 0.41 + Vec3::Y * length * 0.48;
                    let inner_b = centre + (b - centre) * 0.41 + Vec3::Y * length * 0.48;
                    self.triangle(
                        top,
                        inner_a,
                        inner_b,
                        Vec3::Y + (a - centre) / radius,
                        true,
                        shade,
                    );
                    self.triangle(inner_a, a, b, Vec3::Y + (a - centre) / radius, true, shade);
                    self.triangle(
                        inner_a,
                        b,
                        inner_b,
                        Vec3::Y + (a - centre) / radius,
                        true,
                        shade,
                    );
                }
                self.triangle(bottom, b, a, -Vec3::Y, true, shade);
            }
        }
    }

    fn palm(&mut self) {
        let crown_base = [0.75, 0.78, 0.78, 0.81][self.profile.variant as usize];
        self.stem(crown_base);
        let centre = self.stem_at(crown_base);
        let count = if self.profile.variant == 1 { 11 } else { 9 };
        for i in 0..count {
            let angle = self.phase + TAU * i as f32 / count as f32;
            let direction = radial(angle);
            let side = radial(angle + PI * 0.5);
            let rise = self.height
                * ([0.30, 0.27, 0.25, 0.235][self.profile.variant as usize]
                    + 0.035 * (i as f32 * 2.1).sin());
            let droop = if self.profile.variant == 1 {
                0.11
            } else {
                0.18
            };
            let length = self.radius * (1.08 + 0.10 * (i as f32 * 1.7).cos());
            if self.profile.variant == 2 {
                let attach = centre + direction * length * 0.38 + Vec3::Y * rise * 0.46;
                self.tube(
                    &[(centre, self.trunk * 0.16), (attach, self.trunk * 0.08)],
                    [5, 4, 3][self.lod as usize],
                    0.98,
                );
                let pleats = [10, 6, 4][self.lod as usize];
                for j in 0..pleats {
                    let make_point = |k: usize| {
                        let sweep = (k as f32 / pleats as f32 - 0.5) * PI * 0.9;
                        attach
                            + direction * (length * 0.62 * sweep.cos())
                            + side * (length * 0.55 * sweep.sin())
                            + Vec3::Y
                                * (rise * 0.4 - if k % 2 == 0 { 0.0 } else { self.height * 0.025 })
                    };
                    let a = make_point(j);
                    let b = make_point(j + 1);
                    self.triangle(attach, a, b, Vec3::Y, true, 1.02);
                    self.triangle(attach, b, a, -Vec3::Y, true, 0.98);
                }
                continue;
            }
            let segments = [6, 4, 2][self.lod as usize];
            let width = length
                * if self.profile.variant == 1 {
                    0.20
                } else {
                    0.17
                };
            let height = self.height;
            let section = |j: usize| {
                let t = j as f32 / segments as f32;
                let p = centre
                    + direction * (length * t)
                    + Vec3::Y * (rise * (PI * t * 0.92).sin() - height * droop * t * t);
                let width = width * (PI * t).sin().max(0.0).powf(0.72);
                let fold = width * 0.20;
                [
                    p - side * width - Vec3::Y * fold,
                    p + Vec3::Y * fold,
                    p + side * width - Vec3::Y * fold,
                ]
            };
            for j in 0..segments {
                let a = section(j);
                let b = section(j + 1);
                for k in 0..2 {
                    let triangles = [[a[k], b[k], b[k + 1]], [a[k], b[k + 1], a[k + 1]]];
                    for [x, y, z] in triangles {
                        self.triangle(x, y, z, Vec3::Y, true, 1.0 + (i % 3) as f32 * 0.025);
                        // Fronds are opaque thin leaves with separately lit
                        // upper/lower surfaces, so back-face culling is safe.
                        self.triangle(x, z, y, -Vec3::Y, true, 0.97);
                    }
                }
            }
        }
    }
}

/// Build a reusable tree in metres, with LOD 0 finest and LOD 2 coarsest.
/// All levels retain the same authored branch layout and crown centres.
pub fn build_tree_mesh(profile: TreeProfileId, lod: u8) -> NearTreeMesh {
    let mesh = canonical_mesh(profile, lod);
    NearTreeMesh {
        mesh: mesh.mesh.clone(),
        height_m: mesh.height_m,
        radius_m: mesh.radius_m,
    }
}

pub(crate) fn canonical_mesh(profile: TreeProfileId, lod: u8) -> &'static NearTreeMesh {
    canonical_geometry(profile, lod)
}

pub(crate) fn canonical_height(profile: TreeProfileId) -> f32 {
    canonical_geometry(profile, 0).height_m
}

fn canonical_geometry(profile: TreeProfileId, lod: u8) -> &'static NearTreeMesh {
    type Geometry = std::sync::OnceLock<NearTreeMesh>;
    static SHAPES: std::sync::OnceLock<Vec<[Geometry; 3]>> = std::sync::OnceLock::new();
    let shapes = SHAPES.get_or_init(|| {
        crate::vegetation::TREE_PROFILES
            .iter()
            .map(|_| std::array::from_fn(|_| Geometry::new()))
            .collect()
    });
    shapes[profile as usize][usize::from(lod.min(2))]
        .get_or_init(|| build_tree_geometry(profile, lod))
}

fn build_tree_geometry(profile: TreeProfileId, lod: u8) -> NearTreeMesh {
    let profile = tree_profile(profile);
    let height = (profile.height_m[0] + profile.height_m[1]) * 0.5;
    let radius = height * (profile.radius_ratio[0] + profile.radius_ratio[1]) * 0.5;
    let (bark, leaves) = palette(profile);
    let mut builder = Builder {
        mesh: MeshBuffers::default(),
        profile,
        lod: lod.min(2),
        height,
        radius,
        trunk: height
            * profile.trunk_ratio
            * if matches!(
                profile.grammar,
                CrownGrammar::Baobab | CrownGrammar::Round | CrownGrammar::Spreading
            ) {
                1.0
            } else {
                0.78
            },
        phase: hash01(profile.id as u32 * 43 + 17) * TAU,
        bark,
        leaves,
    };
    match profile.grammar {
        CrownGrammar::Cone => builder.conifer(),
        CrownGrammar::Palm => builder.palm(),
        _ => builder.broadleaf(),
    }
    let height_m = builder
        .mesh
        .vertices
        .iter()
        .map(|v| v.pos[1])
        .fold(0.0, f32::max);
    let radius_m = builder
        .mesh
        .vertices
        .iter()
        .map(|v| v.pos[0].hypot(v.pos[2]))
        .fold(0.0, f32::max);
    NearTreeMesh {
        mesh: builder.mesh,
        height_m,
        radius_m,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vegetation::TREE_PROFILES;

    fn silhouette(tree: &NearTreeMesh, scale: f32, angle: f32) -> [bool; 64 * 64] {
        let eye = (radial(angle) + Vec3::Y * 0.36).normalize();
        let right = Vec3::Y.cross(eye).normalize();
        let up = eye.cross(right);
        let project = |p: [f32; 3]| {
            let p = Vec3::from_array(p);
            glam::Vec2::new(32.0 + p.dot(right) * scale, 54.0 - p.dot(up) * scale)
        };
        let mut covered = [false; 64 * 64];
        for indices in tree.mesh.indices.chunks_exact(3) {
            let [a, b, c] = [indices[0], indices[1], indices[2]]
                .map(|i| project(tree.mesh.vertices[i as usize].pos));
            let cross = |a: glam::Vec2, b: glam::Vec2| a.x * b.y - a.y * b.x;
            let area = cross(b - a, c - a);
            if area.abs() < 1.0e-5 {
                continue;
            }
            let min = a.min(b).min(c).floor().max(glam::Vec2::ZERO);
            let max = a.max(b).max(c).ceil().min(glam::Vec2::splat(64.0));
            for y in min.y as usize..max.y as usize {
                for x in min.x as usize..max.x as usize {
                    let p = glam::Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                    let u = cross(b - p, c - p) / area;
                    let v = cross(c - p, a - p) / area;
                    if u >= 0.0 && v >= 0.0 && u + v <= 1.0 {
                        covered[y * 64 + x] = true;
                    }
                }
            }
        }
        covered
    }

    #[test]
    fn lods_preserve_projected_canopy_coverage_from_multiple_directions() {
        for profile in TREE_PROFILES {
            let fine = build_tree_mesh(profile.id, 0);
            let scale = (48.0 / fine.height_m).min(27.0 / fine.radius_m);
            for view in 0..3 {
                let angle = view as f32 * TAU / 3.0;
                let reference = silhouette(&fine, scale, angle);
                for lod in 1..=2 {
                    let other = silhouette(&build_tree_mesh(profile.id, lod), scale, angle);
                    let intersection = reference
                        .iter()
                        .zip(&other)
                        .filter(|(a, b)| **a && **b)
                        .count();
                    let union = reference
                        .iter()
                        .zip(&other)
                        .filter(|(a, b)| **a || **b)
                        .count();
                    let overlap = intersection as f32 / union as f32;
                    assert!(
                        overlap > if lod == 1 { 0.77 } else { 0.65 },
                        "{}: LOD {lod}, view {view}, silhouette overlap {overlap}",
                        profile.name
                    );
                }
            }
        }
    }

    #[test]
    fn all_profiles_are_bounded_deterministic_surface_meshes() {
        for profile in TREE_PROFILES {
            let fine = build_tree_mesh(profile.id, 0);
            for lod in 0..=2 {
                let tree = build_tree_mesh(profile.id, lod);
                let repeated = build_tree_mesh(profile.id, lod);
                assert_eq!(
                    tree.mesh.vertices, repeated.mesh.vertices,
                    "{} LOD {lod}",
                    profile.name
                );
                assert_eq!(tree.mesh.indices, repeated.mesh.indices);
                assert!(
                    tree.height_m > profile.height_m[0] * 0.75
                        && tree.height_m < profile.height_m[1] * 1.2,
                    "{}: height {}",
                    profile.name,
                    tree.height_m
                );
                assert!(tree.radius_m > 0.5 && tree.radius_m.is_finite());
                assert!(
                    (tree.height_m / fine.height_m - 1.0).abs() < 0.12,
                    "{}: LOD {lod} height changed",
                    profile.name
                );
                assert!(
                    (tree.radius_m / fine.radius_m - 1.0).abs() < 0.17,
                    "{}: LOD {lod} radius changed",
                    profile.name
                );
                let triangles = tree.mesh.indices.len() / 3;
                assert!(
                    triangles > 20 && triangles <= [3000, 1000, 300][lod as usize],
                    "{}: LOD {lod} has {triangles} triangles",
                    profile.name
                );
                let mut trunk = false;
                let mut foliage = false;
                for vertex in &tree.mesh.vertices {
                    assert!(vertex.pos.into_iter().all(f32::is_finite));
                    assert!(vertex.pos[1] >= 0.0 && vertex.pos[1] <= tree.height_m);
                    assert!(vertex.pos[0].hypot(vertex.pos[2]) <= tree.radius_m + 1.0e-5);
                    assert!(
                        vertex.face_ao > u16::MAX as u32,
                        "polygon normal is missing"
                    );
                    let material = ((vertex.face_ao >> 5) & 255) as u8;
                    trunk |= material == crate::material::TRUNK;
                    foliage |= material == crate::material::foliage(profile.foliage);
                    assert!(
                        material == crate::material::TRUNK
                            || material == crate::material::foliage(profile.foliage)
                    );
                }
                assert!(trunk && foliage);
                for triangle in tree.mesh.indices.chunks_exact(3) {
                    let [a, b, c] = [triangle[0], triangle[1], triangle[2]].map(|i| {
                        assert!((i as usize) < tree.mesh.vertices.len());
                        Vec3::from_array(tree.mesh.vertices[i as usize].pos)
                    });
                    assert!((b - a).cross(c - a).length_squared() > 1.0e-12);
                    let geometric = (b - a).cross(c - a).normalize();
                    for &index in triangle {
                        let payload = tree.mesh.vertices[index as usize].face_ao;
                        let normal = Vec3::from_array(
                            vearth_core::mesh::decode_surface_normal(payload).unwrap(),
                        );
                        assert!(normal.is_finite());
                        assert!(
                            normal.dot(geometric) > 0.998,
                            "{}: polygon normal disagrees with winding",
                            profile.name
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn coarse_lod_clamps_and_preserves_species_identity() {
        for profile in TREE_PROFILES {
            let coarse = build_tree_mesh(profile.id, 2);
            let clamped = build_tree_mesh(profile.id, u8::MAX);
            assert_eq!(coarse.mesh.vertices, clamped.mesh.vertices);
            assert_eq!(coarse.mesh.indices, clamped.mesh.indices);
        }
        let oak = build_tree_mesh(TreeProfileId::EnglishOak, 0);
        let spruce = build_tree_mesh(TreeProfileId::NorwaySpruce, 0);
        assert!(oak.radius_m / oak.height_m > spruce.radius_m / spruce.height_m);
    }
}
