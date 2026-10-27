use std::borrow::Cow;

pub(crate) const SIDE: u32 = 64;

fn compute_source() -> String {
    // Reuse the actual world uniform declaration so newly appended fields or
    // alignment changes cannot silently give the compute pass different data.
    let world = include_str!("shader.wgsl");
    let start = world.find("struct Globals {").expect("world Globals");
    let end = start + world[start..].find("\n};").expect("Globals end") + 3;
    format!(
        "{}\n@group(0) @binding(0) var<uniform> g: Globals;\n{}\n{}\n{}\n{}",
        &world[start..end],
        include_str!("sky_palette.wgsl"),
        include_str!("water.wgsl"),
        include_str!("sky_environment_common.wgsl"),
        include_str!("sky_environment.wgsl"),
    )
}

pub(crate) struct SkyEnvironment {
    pub(crate) view: wgpu::TextureView,
    pub(crate) sampler: wgpu::Sampler,
    pipeline: wgpu::ComputePipeline,
    inputs: wgpu::BindGroup,
    output: wgpu::BindGroup,
}

impl SkyEnvironment {
    pub(crate) fn new(
        device: &wgpu::Device,
        globals: &wgpu::Buffer,
        sky_model: &wgpu::Buffer,
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shared sky reflection environment (32 KiB)"),
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
            label: Some("bilinear octahedral sky environment"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shared sky environment generation"),
            source: wgpu::ShaderSource::Wgsl(Cow::Owned(compute_source())),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("shared sky environment generation"),
            layout: None,
            module: &shader,
            entry_point: Some("generate_sky_environment"),
            compilation_options: Default::default(),
            cache: None,
        });
        let inputs = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sky environment inputs"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: globals.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: sky_model.as_entire_binding(),
                },
            ],
        });
        let output = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sky environment output"),
            layout: &pipeline.get_bind_group_layout(1),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            }],
        });
        Self {
            view,
            sampler,
            pipeline,
            inputs,
            output,
        }
    }

    pub(crate) fn encode(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        timestamp_writes: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("shared sky reflection environment"),
            timestamp_writes,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.inputs, &[]);
        pass.set_bind_group(1, &self.output, &[]);
        pass.dispatch_workgroups(SIDE.div_ceil(8), SIDE.div_ceil(8), 1);
    }
}

#[cfg(test)]
#[path = "sky_environment_tests.rs"]
mod tests;
