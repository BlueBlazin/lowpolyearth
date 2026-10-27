use crate::gfx::{PlayerDraw, VehicleEffects};
use crate::vehicles::VehicleKind;
use glam::{DMat4, DVec3, Vec3};

const MAX_INSTANCES: usize = 128;
const MAX_MOTES: usize = 64;
const MOTE_LIFETIME: f64 = 0.35;
const EMISSION_INTERVAL: f64 = 1.0 / 48.0;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct EffectInstance {
    start_radius: [f32; 4],
    end_radius: [f32; 4],
    color_energy: [f32; 4],
    shape: [f32; 4],
}

fn instances(player: &PlayerDraw, camera: DVec3, time: f64) -> Vec<EffectInstance> {
    let Some(VehicleEffects {
        kind,
        throttle,
        boost,
    }) = player.effects
    else {
        return Vec::new();
    };
    let throttle = throttle.clamp(0.0, 1.0);
    let boost = boost.clamp(0.0, 1.0);
    let power = 0.18 + throttle * 0.50 + boost * 0.70;
    let tint = if kind == crate::vehicles::VehicleKind::Sparrow {
        [1.0, 0.27, 0.055]
    } else {
        [0.055, 0.42, 1.0]
    };
    let mut result = Vec::with_capacity(32);
    for (index, port) in kind.exhaust_ports().iter().enumerate() {
        let origin = (player
            .model
            .transform_point3(DVec3::from_array(port.position.map(f64::from)))
            - camera)
            .as_vec3();
        let direction = player
            .model
            .transform_vector3(DVec3::from_array(port.direction.map(f64::from)))
            .as_vec3()
            .normalize();
        let radius = port.radius;
        let length = radius * (3.0 + throttle * 7.0 + boost * 17.0);
        let pulse = pulse_phase(time, index);
        let mut push = |start: Vec3,
                        end: Vec3,
                        widths: [f32; 2],
                        color: [f32; 3],
                        energy: f32,
                        kind: f32,
                        phase: f32| {
            result.push(EffectInstance {
                start_radius: start.extend(widths[0]).to_array(),
                end_radius: end.extend(widths[1]).to_array(),
                color_energy: [color[0], color[1], color[2], energy],
                shape: [kind, phase, radius, 0.0],
            });
        };
        // A short white core, a softer colored envelope, and a nozzle glow
        // share an anchor. They fade independently instead of swapping meshes.
        push(
            origin,
            origin + direction * length,
            [radius * 1.05, radius * 0.12],
            tint,
            power * 2.2,
            0.0,
            pulse,
        );
        push(
            origin,
            origin + direction * length * 0.48,
            [radius * 0.45, radius * 0.035],
            [0.72, 0.86, 1.0],
            power * 4.0,
            0.0,
            pulse,
        );
        push(
            origin,
            origin,
            [radius * 2.5, radius * 2.5],
            tint,
            power * 0.65,
            1.0,
            pulse,
        );
    }
    assert!(result.len() <= MAX_INSTANCES);
    result
}

fn pulse_phase(time: f64, port: usize) -> f32 {
    (time * 9.0 + port as f64 * 1.71).rem_euclid(std::f64::consts::TAU) as f32
}

struct Mote {
    position: DVec3,
    velocity: DVec3,
    age: f64,
    radius: f32,
    color: [f32; 3],
    energy: f32,
}

#[derive(Clone, Copy)]
struct EmissionFrame {
    model: DMat4,
    kind: VehicleKind,
    time: f64,
}

#[derive(Default)]
struct ExhaustTrail {
    motes: Vec<Mote>,
    previous: Option<EmissionFrame>,
    accumulator: f64,
    sequence: u32,
}

impl ExhaustTrail {
    fn update(&mut self, player: Option<&PlayerDraw>, time: f64) {
        let Some((player, effects)) = player
            .and_then(|p| p.effects.map(|effects| (p, effects)))
            .filter(|(_, effects)| effects.kind != VehicleKind::Kestrel)
        else {
            self.motes.clear();
            self.previous = None;
            self.accumulator = 0.0;
            return;
        };
        let frame = EmissionFrame {
            model: player.model,
            kind: effects.kind,
            time,
        };
        let previous = self.previous.replace(frame);
        let Some(previous) = previous else {
            return;
        };
        let elapsed = time - previous.time;
        let displacement = player.model.w_axis.truncate() - previous.model.w_axis.truncate();
        // Navigation/type changes and suspended rendering start a fresh emitter.
        // Ordinary high-speed flight retains the short trail in f64 ECEF.
        if previous.kind != effects.kind
            || elapsed <= 0.0
            || elapsed > MOTE_LIFETIME
            || displacement.length() > 200.0 + elapsed * 5000.0
        {
            self.motes.clear();
            self.accumulator = 0.0;
            return;
        }
        for mote in &mut self.motes {
            mote.position += mote.velocity * elapsed;
            mote.age += elapsed;
        }
        self.motes.retain(|mote| mote.age < MOTE_LIFETIME);
        let throttle = effects.throttle.clamp(0.0, 1.0);
        let boost = effects.boost.clamp(0.0, 1.0);
        let energy = (throttle + boost) * 1.2;
        let color = if effects.kind == VehicleKind::Sparrow {
            [1.0, 0.27, 0.055]
        } else {
            [0.055, 0.42, 1.0]
        };
        let inherited_velocity = displacement / elapsed;
        self.accumulator += elapsed;
        while self.accumulator >= EMISSION_INTERVAL {
            self.accumulator -= EMISSION_INTERVAL;
            let age = self.accumulator;
            let fraction = (1.0 - age / elapsed).clamp(0.0, 1.0);
            if energy <= 0.01 {
                continue;
            }
            for port in effects.kind.exhaust_ports() {
                if self.motes.len() == MAX_MOTES {
                    break;
                }
                let local_position = DVec3::from_array(port.position.map(f64::from));
                let local_direction = DVec3::from_array(port.direction.map(f64::from));
                let emitter = previous
                    .model
                    .transform_point3(local_position)
                    .lerp(player.model.transform_point3(local_position), fraction);
                let current_direction = player.model.transform_vector3(local_direction).normalize();
                let interpolated_direction = previous
                    .model
                    .transform_vector3(local_direction)
                    .lerp(current_direction, fraction);
                let direction = if interpolated_direction.length_squared() > 0.0001 {
                    interpolated_direction.normalize()
                } else {
                    current_direction
                };
                let side = direction.any_orthonormal_vector();
                let up = direction.cross(side);
                let angle = f64::from(self.sequence) * 2.399_963_229_7;
                self.sequence = self.sequence.wrapping_add(1);
                let spread = side * angle.cos() + up * angle.sin();
                let radius = f64::from(port.radius);
                let length = radius * f64::from(3.0 + throttle * 7.0 + boost * 17.0);
                let velocity = inherited_velocity
                    + direction * (length / MOTE_LIFETIME)
                    + spread * radius * 2.0;
                self.motes.push(Mote {
                    position: emitter + direction * length * 0.35 + velocity * age,
                    velocity,
                    age,
                    radius: port.radius * 0.095,
                    color,
                    energy,
                });
            }
        }
    }

    fn append_instances(&self, result: &mut Vec<EffectInstance>, camera: DVec3) {
        for mote in &self.motes {
            let age = (mote.age / MOTE_LIFETIME) as f32;
            let fade = (age * 8.0).min(1.0) * (1.0 - age).powi(2);
            let start = (mote.position - camera).as_vec3();
            let end = start + mote.velocity.normalize_or_zero().as_vec3() * mote.radius * 2.0;
            result.push(EffectInstance {
                start_radius: start.extend(mote.radius).to_array(),
                end_radius: end.extend(mote.radius).to_array(),
                color_energy: [
                    mote.color[0],
                    mote.color[1],
                    mote.color[2],
                    mote.energy * fade,
                ],
                shape: [2.0, 0.0, mote.radius, 0.0],
            });
        }
        debug_assert!(result.len() <= MAX_INSTANCES);
    }
}

pub(crate) struct VehicleEffectRenderer {
    pipeline: wgpu::RenderPipeline,
    buffer: wgpu::Buffer,
    epoch: vearth_platform::time::Instant,
    count: u32,
    trail: ExhaustTrail,
}

impl VehicleEffectRenderer {
    pub fn new(
        device: &wgpu::Device,
        globals: &wgpu::BindGroupLayout,
        depth: &wgpu::BindGroupLayout,
        format: wgpu::TextureFormat,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("soft vehicle exhaust"),
            source: wgpu::ShaderSource::Wgsl(include_str!("vehicle_effects.wgsl").into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("vehicle exhaust"),
            bind_group_layouts: &[Some(globals), Some(depth)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("soft additive vehicle exhaust"), layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader, entry_point: Some("vs_main"), compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<EffectInstance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader, entry_point: Some("fs_main"), compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
                        alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
                    }), write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(), depth_stencil: None,
            multisample: Default::default(), multiview_mask: None, cache: None,
        });
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bounded vehicle exhaust instances"),
            size: (MAX_INSTANCES * std::mem::size_of::<EffectInstance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            buffer,
            epoch: vearth_platform::time::Instant::now(),
            count: 0,
            trail: ExhaustTrail::default(),
        }
    }

    pub fn prepare(&mut self, queue: &wgpu::Queue, player: Option<&PlayerDraw>, camera: DVec3) {
        let time = self.epoch.elapsed().as_secs_f64();
        let mut instances = player.map_or_else(Vec::new, |p| instances(p, camera, time));
        self.trail.update(player, time);
        self.trail.append_instances(&mut instances, camera);
        self.count = instances.len() as u32;
        if self.count > 0 {
            queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(&instances));
        }
    }

    pub fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        globals: &wgpu::BindGroup,
        depth: &wgpu::BindGroup,
    ) -> u64 {
        if self.count == 0 {
            return 0;
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("vehicle exhaust after opaque craft"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, globals, &[]);
        pass.set_bind_group(1, depth, &[]);
        pass.set_vertex_buffer(0, self.buffer.slice(..));
        pass.draw(0..6, 0..self.count);
        self.count as u64 * 2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn player(kind: VehicleKind) -> PlayerDraw {
        PlayerDraw {
            model: DMat4::from_translation(DVec3::new(6_400_000.0, 0.0, 0.0)),
            axes: [DVec3::X, DVec3::Y, DVec3::Z],
            parts: vec![],
            effects: Some(VehicleEffects {
                kind,
                throttle: 0.6,
                boost: 0.3,
            }),
        }
    }

    #[test]
    fn pulse_is_continuous_across_long_runtime_and_phase_wraps() {
        for time in [std::f64::consts::TAU / 9.0, 3600.0, 1_000_000.0] {
            let before = pulse_phase(time - 0.0001, 0);
            let after = pulse_phase(time + 0.0001, 0);
            assert!((before.cos() - after.cos()).abs() < 0.002);
            assert!((before.sin() - after.sin()).abs() < 0.002);
            assert!((0.0..=std::f32::consts::TAU).contains(&after));
        }
    }

    #[test]
    fn motes_detach_from_turns_and_reset_at_navigation_boundaries() {
        let mut trail = ExhaustTrail::default();
        let mut player = player(VehicleKind::Sparrow);
        trail.update(Some(&player), 0.0);
        trail.update(Some(&player), 0.05);
        assert!(!trail.motes.is_empty());
        let old_position = trail.motes[0].position;
        let old_velocity = trail.motes[0].velocity;
        player.model = DMat4::from_translation(DVec3::new(6_400_005.0, 0.0, 0.0))
            * DMat4::from_rotation_y(0.9);
        trail.update(Some(&player), 0.06);
        let expected = old_position + old_velocity * 0.01;
        assert!(trail.motes[0].position.distance(expected) < 1.0e-6);
        assert_eq!(trail.motes[0].velocity, old_velocity);
        for tick in 1..=120 {
            trail.update(Some(&player), 0.06 + f64::from(tick) / 120.0);
            assert!(trail.motes.len() <= MAX_MOTES);
            assert!(trail.motes.iter().all(|mote| mote.age < MOTE_LIFETIME));
        }
        trail.update(None, 1.07);
        assert!(trail.motes.is_empty() && trail.previous.is_none());
        trail.update(Some(&player), 1.08);
        trail.update(Some(&player), 1.13);
        assert!(!trail.motes.is_empty());
        player.effects.as_mut().unwrap().kind = VehicleKind::PhaseManta;
        trail.update(Some(&player), 1.14);
        assert!(trail.motes.is_empty());
        trail.update(Some(&player), 1.19);
        assert!(!trail.motes.is_empty());
        player.model.w_axis.x += 10_000.0;
        trail.update(Some(&player), 1.20);
        assert!(trail.motes.is_empty());
        trail.update(Some(&player), 1.25);
        assert!(!trail.motes.is_empty());
        trail.update(Some(&player), 2.0);
        assert!(trail.motes.is_empty());
        trail.update(Some(&player), 2.05);
        assert!(!trail.motes.is_empty());
        player.effects.as_mut().unwrap().kind = VehicleKind::Kestrel;
        for (time, boost) in [(2.06, 0.0), (2.11, 0.0), (2.16, 1.0), (2.21, 1.0)] {
            player.effects.as_mut().unwrap().boost = boost;
            trail.update(Some(&player), time);
            assert!(trail.motes.is_empty() && trail.previous.is_none());
            let camera = player.model.w_axis.truncate();
            let mut exhaust = instances(&player, camera, time);
            let plume_count = exhaust.len();
            trail.append_instances(&mut exhaust, camera);
            assert!(plume_count > 0);
            assert_eq!(exhaust.len(), plume_count);
        }
    }

    #[test]
    fn exhaust_is_bounded_camera_relative_and_continuous() {
        for kind in [
            crate::vehicles::VehicleKind::Kestrel,
            crate::vehicles::VehicleKind::Sparrow,
            crate::vehicles::VehicleKind::PhaseManta,
        ] {
            let mut player = PlayerDraw {
                model: glam::DMat4::from_translation(DVec3::new(6_400_000.0, 0.0, 0.0)),
                axes: [DVec3::X, DVec3::Y, DVec3::Z],
                parts: vec![],
                effects: Some(VehicleEffects {
                    kind,
                    throttle: 0.6,
                    boost: 0.3,
                }),
            };
            let camera = DVec3::new(6_400_000.0, 0.0, -10.0);
            let a = instances(&player, camera, 1.0);
            let b = instances(&player, camera, 1.0001);
            assert!(!a.is_empty() && a.len() <= MAX_INSTANCES);
            assert_eq!(a.len(), b.len());
            for (a, b) in a.iter().zip(&b) {
                assert!(
                    a.start_radius
                        .iter()
                        .chain(&a.end_radius)
                        .chain(&a.color_energy)
                        .all(|x| x.is_finite())
                );
                assert!(Vec3::from_array(a.start_radius[..3].try_into().unwrap()).length() < 40.0);
                assert!((a.start_radius[0] - b.start_radius[0]).abs() < 0.01);
            }
            player.effects = None;
            assert!(instances(&player, camera, 1.0).is_empty());
        }
    }
}
