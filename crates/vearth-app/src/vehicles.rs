//! Authored aircraft surfaces in metres, with +Y up and +Z forward.

mod asset;
pub(crate) mod textures;

use vearth_core::mesh::MeshBuffers;

// Shared by the Sparrow hull, exhaust ports, and collision bounds.
pub(crate) const SPARROW_SCALE: f32 = 0.6;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum VehicleKind {
    #[default]
    Kestrel,
    Sparrow,
    SplitArrow,
    PhaseManta,
}

#[derive(Clone, Copy, Debug)]
pub struct ExhaustPort {
    pub position: [f32; 3],
    pub direction: [f32; 3],
    pub radius: f32,
}

impl ExhaustPort {
    const fn scaled(self, scale: f32) -> Self {
        Self {
            position: [
                self.position[0] * scale,
                self.position[1] * scale,
                self.position[2] * scale,
            ],
            direction: self.direction,
            radius: self.radius * scale,
        }
    }
}

const KESTREL_PORTS: [ExhaustPort; 2] = [
    ExhaustPort {
        position: [-0.368493, -0.066312, -0.690323],
        direction: [0.0, 0.0, -1.0],
        radius: 0.161109,
    },
    ExhaustPort {
        position: [0.368493, -0.066312, -0.690323],
        direction: [0.0, 0.0, -1.0],
        radius: 0.161109,
    },
];
const SPARROW_PORTS: [ExhaustPort; 2] = [
    ExhaustPort {
        position: [-0.24, -0.085, -1.315],
        direction: [0.0, 0.0, -1.0],
        radius: 0.070,
    }
    .scaled(SPARROW_SCALE),
    ExhaustPort {
        position: [0.24, -0.085, -1.315],
        direction: [0.0, 0.0, -1.0],
        radius: 0.070,
    }
    .scaled(SPARROW_SCALE),
];
const MANTA_PORTS: [ExhaustPort; 4] = [
    ExhaustPort {
        position: [-3.506403, 0.004528, -3.666398],
        direction: [0.0, 0.0, -1.0],
        radius: 0.279382,
    },
    ExhaustPort {
        position: [3.506403, 0.004528, -3.666398],
        direction: [0.0, 0.0, -1.0],
        radius: 0.279382,
    },
    ExhaustPort {
        position: [-0.754331, -0.243484, -3.529900],
        direction: [0.0, 0.0, -1.0],
        radius: 0.279382,
    },
    ExhaustPort {
        position: [0.754331, -0.243484, -3.529900],
        direction: [0.0, 0.0, -1.0],
        radius: 0.279382,
    },
];

impl VehicleKind {
    pub const fn next(self) -> Self {
        match self {
            Self::Kestrel => Self::PhaseManta,
            Self::PhaseManta => Self::Sparrow,
            Self::Sparrow | Self::SplitArrow => Self::Kestrel,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Kestrel => "KESTREL",
            Self::Sparrow => "SPARROW",
            Self::SplitArrow => "SPLIT ARROW",
            Self::PhaseManta => "PHASE MANTA",
        }
    }

    pub const fn is_phased(self) -> bool {
        matches!(self, Self::SplitArrow | Self::PhaseManta)
    }

    pub const fn chase_boom_m(self) -> f64 {
        match self {
            Self::Kestrel => 9.0,
            Self::Sparrow => 5.5,
            Self::SplitArrow => 16.0,
            Self::PhaseManta => 18.0,
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "kestrel" => Some(Self::Kestrel),
            "sparrow" | "hoverbike" | "hover-bike" => Some(Self::Sparrow),
            "phase-manta" | "phase_manta" | "phasemanta" | "manta" => Some(Self::PhaseManta),
            _ => None,
        }
    }

    pub const fn exhaust_ports(self) -> &'static [ExhaustPort] {
        match self {
            Self::Kestrel => &KESTREL_PORTS,
            Self::Sparrow => &SPARROW_PORTS,
            Self::SplitArrow => &MANTA_PORTS,
            Self::PhaseManta => &MANTA_PORTS,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct VehicleRenderParts {
    kestrel: usize,
    sparrow: usize,
    phase_manta: usize,
}

impl VehicleRenderParts {
    pub fn draw_parts(self, kind: VehicleKind) -> Vec<usize> {
        vec![match kind {
            VehicleKind::Kestrel => self.kestrel,
            VehicleKind::Sparrow => self.sparrow,
            VehicleKind::PhaseManta | VehicleKind::SplitArrow => self.phase_manta,
        }]
    }
}

pub struct VehicleMeshSet {
    pub meshes: Vec<MeshBuffers>,
    pub uvs: Vec<Vec<[f32; 2]>>,
    pub parts: VehicleRenderParts,
}

/// The parts in the order [`build_mesh_set`] returns them.
pub const AUTHORED_PARTS: VehicleRenderParts = VehicleRenderParts {
    kestrel: 0,
    sparrow: 1,
    phase_manta: 2,
};

pub fn build_mesh_set() -> VehicleMeshSet {
    let mut meshes = Vec::new();
    let mut uvs = Vec::new();
    for (bytes, scale) in [
        (
            vearth_platform::asset_bytes!("../../../assets/vehicles/refined/kestrel.mesh"),
            1.0,
        ),
        (
            vearth_platform::asset_bytes!("../../../assets/vehicles/refined/sparrow.mesh"),
            SPARROW_SCALE,
        ),
        (
            vearth_platform::asset_bytes!("../../../assets/vehicles/refined/phase_manta.mesh"),
            1.0,
        ),
    ] {
        let mut asset = asset::decode(bytes).expect("validated authored vehicle asset");
        let mut body = asset.take_part("body").expect("authored vehicle body");
        assert_eq!(
            asset.part_names().count(),
            0,
            "all authored parts must be rendered"
        );
        for vertex in &mut body.mesh.vertices {
            vertex.pos = vertex.pos.map(|coordinate| coordinate * scale);
        }
        meshes.push(body.mesh);
        uvs.push(body.uvs);
    }
    VehicleMeshSet {
        meshes,
        uvs,
        parts: AUTHORED_PARTS,
    }
}

/// Vehicle meshes and base-color mip chains, loaded and decoded off the
/// render thread.
pub struct PreparedVehicles {
    pub set: VehicleMeshSet,
    pub texture_layers: Vec<Vec<Vec<u8>>>,
}

/// Preparation of the vehicle assets on a worker thread, started once they
/// are due, so it does not delay the first frame (and in the browser, so its
/// downloads do not compete with the first view). Until the assets are
/// ready the renderer skips the missing player parts.
pub enum DeferredVehicles {
    Waiting,
    Loading(std::sync::mpsc::Receiver<PreparedVehicles>),
    Ready,
}

impl DeferredVehicles {
    /// Starts preparation once `due`; returns the assets once, when ready.
    pub fn poll(&mut self, due: bool) -> Option<PreparedVehicles> {
        match self {
            Self::Waiting if due => {
                let (tx, rx) = std::sync::mpsc::channel();
                let spawned = vearth_platform::thread::Builder::new()
                    .name("vehicle-assets".into())
                    .spawn(move || {
                        let _ = tx.send(PreparedVehicles {
                            set: build_mesh_set(),
                            texture_layers: (0..textures::LAYERS)
                                .map(textures::decode_layer)
                                .collect(),
                        });
                    });
                *self = match spawned {
                    Ok(_) => Self::Loading(rx),
                    Err(error) => {
                        log::error!("vehicles: could not start the loader: {error}");
                        Self::Ready
                    }
                };
                None
            }
            Self::Loading(rx) => match rx.try_recv() {
                Ok(prepared) => {
                    *self = Self::Ready;
                    Some(prepared)
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    log::error!("vehicles: the loader stopped");
                    *self = Self::Ready;
                    None
                }
            },
            _ => None,
        }
    }
}

#[cfg(test)]
pub(crate) fn build_stage_mesh(kind: VehicleKind) -> MeshBuffers {
    build_stage_surface(kind).0
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn build_stage_surface(kind: VehicleKind) -> (MeshBuffers, Vec<[f32; 2]>) {
    let set = build_mesh_set();
    let index = set.parts.draw_parts(kind)[0];
    (set.meshes[index].clone(), set.uvs[index].clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;
    use vearth_core::mesh::decode_surface_normal;

    #[test]
    fn authored_aircraft_have_valid_triangles_normals_materials_and_meter_bounds() {
        let set = build_mesh_set();
        for (part, mesh) in set.meshes.iter().enumerate() {
            assert_eq!(mesh.vertices.len(), set.uvs[part].len());
            assert!(
                set.uvs[part]
                    .iter()
                    .flatten()
                    .all(|value| value.is_finite())
            );
            assert_eq!(mesh.indices.len() % 3, 0);
            for triangle in mesh.indices.chunks_exact(3) {
                let vertices = [triangle[0], triangle[1], triangle[2]]
                    .map(|index| mesh.vertices[index as usize]);
                let [a, b, c] = vertices.map(|vertex| Vec3::from_array(vertex.pos));
                let cross = (b - a).cross(c - a);
                assert!(
                    cross.length_squared() > 1.0e-16,
                    "degenerate triangle in part {part}"
                );
                let normal = cross.normalize();
                for vertex in vertices {
                    let point = Vec3::from_array(vertex.pos);
                    assert!(point.is_finite() && point.abs().max_element() < 6.0);
                    let packed = Vec3::from_array(
                        decode_surface_normal(vertex.face_ao).expect("authored normal"),
                    );
                    assert!(
                        packed.dot(normal) > 0.0,
                        "normal disagrees with winding in part {part}"
                    );
                    assert!((96..=102).contains(&((vertex.face_ao >> 5) & 255)));
                    assert!(((vertex.face_ao >> 13) & 7) <= textures::LAYERS);
                    assert_eq!(
                        vertex.rgba[3], 255,
                        "vehicle materials own their emission response"
                    );
                }
            }
        }
        for kind in [
            VehicleKind::Kestrel,
            VehicleKind::Sparrow,
            VehicleKind::PhaseManta,
            VehicleKind::SplitArrow,
        ] {
            let mesh = build_stage_mesh(kind);
            assert!(
                (500..100_000).contains(&(mesh.indices.len() / 3)),
                "unexpected authored mesh complexity for {kind:?}"
            );
            let materials: std::collections::HashSet<_> = mesh
                .vertices
                .iter()
                .map(|v| (v.face_ao >> 5) & 255)
                .collect();
            assert!(materials.contains(&96) && materials.contains(&97) && materials.contains(&99));
            for port in kind.exhaust_ports() {
                assert!((Vec3::from_array(port.direction).length() - 1.0).abs() < 1.0e-6);
                assert!(port.radius > 0.0 && port.radius < 0.5);
                assert!(
                    mesh.vertices.iter().any(|v| {
                        (Vec3::from_array(v.pos) - Vec3::from_array(port.position)).length()
                            < port.radius * 1.8
                            && ((v.face_ao >> 5) & 255) == 100
                    }),
                    "exhaust port is detached from emissive nozzle for {kind:?}"
                );
            }
        }
    }

    #[test]
    fn vehicle_sizes_remain_proportional() {
        let span = |kind| {
            let mesh = build_stage_mesh(kind);
            let min = mesh
                .vertices
                .iter()
                .map(|v| Vec3::from_array(v.pos))
                .fold(Vec3::splat(f32::INFINITY), Vec3::min);
            let max = mesh
                .vertices
                .iter()
                .map(|v| Vec3::from_array(v.pos))
                .fold(Vec3::splat(f32::NEG_INFINITY), Vec3::max);
            max - min
        };
        let sparrow = span(VehicleKind::Sparrow);
        assert!(
            (1.60..1.70).contains(&sparrow.z),
            "Sparrow length: {sparrow:?}"
        );
        assert!(
            (0.90..0.96).contains(&sparrow.x),
            "Sparrow width: {sparrow:?}"
        );
        assert!(
            (0.60..0.65).contains(&sparrow.y),
            "Sparrow height: {sparrow:?}"
        );
        assert!(sparrow.max_element() < span(VehicleKind::Kestrel).max_element() * 0.9);
        assert!(
            span(VehicleKind::PhaseManta).max_element()
                > span(VehicleKind::Kestrel).max_element() * 1.8
        );
    }

    #[test]
    fn cycle_has_three_playable_aircraft_and_no_watercraft_aliases() {
        assert_eq!(VehicleKind::Kestrel.next(), VehicleKind::PhaseManta);
        assert_eq!(VehicleKind::PhaseManta.next(), VehicleKind::Sparrow);
        assert_eq!(VehicleKind::Sparrow.next(), VehicleKind::Kestrel);
        assert_eq!(VehicleKind::SplitArrow.next(), VehicleKind::Kestrel);
        assert!(VehicleKind::SplitArrow.is_phased() && VehicleKind::PhaseManta.is_phased());
        assert!(!VehicleKind::Kestrel.is_phased() && !VehicleKind::Sparrow.is_phased());
        for removed in [
            "riptide",
            "watercraft",
            "hovercraft",
            "hover-watercraft",
            "split-arrow",
        ] {
            assert_eq!(VehicleKind::parse(removed), None);
        }
        assert_eq!(
            VehicleKind::parse("phase-manta"),
            Some(VehicleKind::PhaseManta)
        );
        assert_eq!(VehicleKind::parse("hover-bike"), Some(VehicleKind::Sparrow));
    }
}
