use std::borrow::Cow;

pub(crate) struct WorldAa {
    pub samples: u32,
    pub layout: wgpu::BindGroupLayout,
    depth_resolve: wgpu::RenderPipeline,
}

pub(crate) struct Targets {
    pub color: wgpu::TextureView,
    pub depth: wgpu::TextureView,
    pub group: wgpu::BindGroup,
}

/// World sample count used when `VEARTH_WORLD_SAMPLES` is unset. Every
/// sub-pixel triangle that covers a sample shades its pixel again, so dense
/// distant geometry makes world shading cost grow with the sample count.
#[cfg(not(target_arch = "wasm32"))]
const DEFAULT_SAMPLES: u32 = 2;
/// WebGPU offers only one or four samples, and four more than doubles the
/// frame time of dense city views; browsers rely on the resolve's edge
/// filter instead.
#[cfg(target_arch = "wasm32")]
const DEFAULT_SAMPLES: u32 = 1;

/// Sample count requested through `VEARTH_WORLD_SAMPLES`.
///
/// `4` gives smoother edges and `1` is a performance control on the same
/// executable, source data, shaders and presentation settings.
pub fn requested_samples() -> u32 {
    samples_from(
        vearth_platform::env::var("VEARTH_WORLD_SAMPLES")
            .ok()
            .as_deref(),
    )
}

fn samples_from(value: Option<&str>) -> u32 {
    match value {
        None => DEFAULT_SAMPLES,
        Some("1") => 1,
        Some("2") => 2,
        Some("4") => 4,
        Some(value) => {
            log::warn!("invalid VEARTH_WORLD_SAMPLES={value:?}; using {DEFAULT_SAMPLES}");
            DEFAULT_SAMPLES
        }
    }
}

impl WorldAa {
    pub fn new(device: &wgpu::Device, depth_format: wgpu::TextureFormat) -> Self {
        let mut samples = requested_samples();
        if samples == 2
            && !device
                .features()
                .contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES)
        {
            log::warn!("two world samples need adapter-specific sample counts; using 4");
            samples = 4;
        }
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("world multisample depth"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: true,
                },
                count: None,
            }],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("world depth resolve"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!("world_aa.wgsl"))),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("world depth resolve"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let depth_resolve = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("world depth resolve"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[],
            }),
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: depth_format,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        Self {
            samples,
            layout,
            depth_resolve,
        }
    }

    pub fn state(&self) -> wgpu::MultisampleState {
        wgpu::MultisampleState {
            count: self.samples,
            ..Default::default()
        }
    }

    pub fn targets(
        &self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
        color_format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
    ) -> Option<Targets> {
        if self.samples == 1 {
            return None;
        }
        let target = |label, format, usage| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: width.max(1),
                        height: height.max(1),
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: self.samples,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let color = target(
            "world multisample color",
            color_format,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let depth = target(
            "world multisample depth",
            depth_format,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("world multisample depth"),
            layout: &self.layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(&depth),
            }],
        });
        Some(Targets {
            color,
            depth,
            group,
        })
    }

    pub fn resolve_depth(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        targets: &Targets,
        output: &wgpu::TextureView,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("world multisample depth resolve"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: output,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        pass.set_pipeline(&self.depth_resolve);
        pass.set_bind_group(0, &targets.group, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::samples_from;

    #[test]
    fn two_samples_are_the_default_and_other_counts_remain_selectable() {
        assert_eq!(samples_from(None), 2);
        assert_eq!(samples_from(Some("1")), 1);
        assert_eq!(samples_from(Some("2")), 2);
        assert_eq!(samples_from(Some("4")), 4);
        assert_eq!(samples_from(Some("8")), 2);
    }
}
