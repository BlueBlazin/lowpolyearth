use glam::DVec3;
use wgpu::util::DeviceExt;

const SIDE: u32 = 128;
const WIDTH_M: f64 = 48_000.0;

#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct Frame {
    origin: [f32; 4],
    right: [f32; 4],
    upward: [f32; 4],
}

fn frame(camera: DVec3, sun: DVec3, ground_r: f64, enabled: bool) -> Frame {
    let up = camera.normalize();
    let solar_sine = up.dot(sun);
    let altitude = camera.length() - ground_r;
    let fade = |start: f64, end: f64, value: f64| {
        let t = ((value - start) / (end - start)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    let strength = if enabled {
        fade(0.06, 0.16, solar_sine) * (1.0 - fade(30_000.0, 60_000.0, altitude))
    } else {
        0.0
    };
    let mut right = up.cross(sun);
    if right.length_squared() < 1e-8 {
        right = DVec3::Z.cross(sun);
        if right.length_squared() < 1e-8 {
            right = DVec3::X.cross(sun);
        }
    }
    right = right.normalize();
    let upward = sun.cross(right).normalize();
    let center = up * ground_r;
    let texel = WIDTH_M / f64::from(SIDE);
    // Snap in absolute double-precision light space. Camera rebasing must
    // not drag the sunlight pattern across stationary terrain.
    let snap = |axis: DVec3| {
        let coordinate = center.dot(axis);
        (coordinate / texel).round() * texel - coordinate
    };
    let origin = center + right * snap(right) + upward * snap(upward) - camera;
    Frame {
        origin: [
            origin.x as f32,
            origin.y as f32,
            origin.z as f32,
            strength as f32,
        ],
        right: [
            right.x as f32,
            right.y as f32,
            right.z as f32,
            (1.0 / WIDTH_M) as f32,
        ],
        upward: [upward.x as f32, upward.y as f32, upward.z as f32, 0.0],
    }
}

fn compute_source() -> String {
    let world = include_str!("shader.wgsl");
    let start = world.find("struct Globals {").expect("world Globals");
    let end = start + world[start..].find("\n};").expect("Globals end") + 3;
    format!(
        "{}\n@group(0) @binding(0) var<uniform> g: Globals;\n{}\n{}\n{}",
        &world[start..end],
        include_str!("cloud_common.wgsl"),
        include_str!("cloud_shadow_common.wgsl"),
        include_str!("cloud_shadow.wgsl"),
    )
}

pub(crate) struct CloudShadows {
    pub(crate) view: wgpu::TextureView,
    pub(crate) sampler: wgpu::Sampler,
    pub(crate) uniform: wgpu::Buffer,
    pipeline: wgpu::ComputePipeline,
    inputs: wgpu::BindGroup,
    output: wgpu::BindGroup,
    configured: bool,
    active: bool,
}

impl CloudShadows {
    pub(crate) fn new(
        device: &wgpu::Device,
        globals: &wgpu::Buffer,
        shape: &wgpu::TextureView,
        shape_sampler: &wgpu::Sampler,
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cloud sunlight columns (128 KiB)"),
            size: wgpu::Extent3d {
                width: SIDE,
                height: SIDE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("filtered cloud sunlight"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("cloud sunlight frame"),
            contents: bytemuck::bytes_of(&Frame::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cloud sunlight generation"),
            source: wgpu::ShaderSource::Wgsl(compute_source().into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("cloud sunlight generation"),
            layout: None,
            module: &shader,
            entry_point: Some("generate_cloud_sunlight"),
            compilation_options: Default::default(),
            cache: None,
        });
        let inputs = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cloud sunlight inputs"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: globals.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(shape),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(shape_sampler),
                },
            ],
        });
        let output = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cloud sunlight output"),
            layout: &pipeline.get_bind_group_layout(1),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            }],
        });
        Self {
            view,
            sampler,
            uniform,
            pipeline,
            inputs,
            output,
            configured: !vearth_platform::env::var("VEARTH_DISABLE_CLOUD_SHADOWS")
                .is_ok_and(|value| matches!(value.as_str(), "1" | "true" | "yes")),
            active: false,
        }
    }

    pub(crate) fn update(
        &mut self,
        queue: &wgpu::Queue,
        camera: DVec3,
        sun: DVec3,
        ground_r: f64,
        clouds_enabled: bool,
        coverage: f32,
    ) {
        let frame = frame(
            camera,
            sun,
            ground_r,
            self.configured && clouds_enabled && coverage > 0.001,
        );
        self.active = frame.origin[3] > 0.0;
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&frame));
    }

    pub(crate) fn encode(&self, encoder: &mut wgpu::CommandEncoder) {
        if !self.active {
            return;
        }
        // One bounded map per rendered frame also follows accelerated weather
        // and time scrubbing without stale shadows or temporal-history trails.
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("cloud sunlight columns"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.inputs, &[]);
        pass.set_bind_group(1, &self.output, &[]);
        pass.dispatch_workgroups(SIDE.div_ceil(8), SIDE.div_ceil(8), 1);
    }
}

#[cfg(test)]
#[path = "cloud_shadows_tests.rs"]
mod tests;
