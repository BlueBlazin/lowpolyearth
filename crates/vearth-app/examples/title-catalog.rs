//! Render deterministic proofs through the production egui/wgpu title path.
//! Usage: cargo run -p vearth-app --example title-catalog -- OUTPUT [PROFILE [BACKGROUND.png]]

use egui::{Color32, Pos2, Rect, vec2};
use std::{
    error::Error,
    fs,
    io::BufReader,
    path::{Path, PathBuf},
};
use vearth_app::location_title::{
    self, ArrivalTitle,
    catalog::{self, Profile},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn specimen(p: &'static Profile) -> ArrivalTitle {
    let (primary, region, local) = match p.key {
        "jaipur" => ("Jaipur", "Rajasthan", Some(("hi", "जयपुर"))),
        "mumbai" => ("Mumbai", "Maharashtra", Some(("mr", "मुंबई"))),
        "kolkata" => ("Kolkata", "West Bengal", Some(("bn", "কলকাতা"))),
        "chennai" => ("Chennai", "Tamil Nadu", Some(("ta", "சென்னை"))),
        "hyderabad" => ("Hyderabad", "Telangana", Some(("te", "హైదరాబాద్"))),
        "kyoto" => ("Kyoto", "", Some(("ja", "京都"))),
        "tokyo" => ("Tokyo", "", Some(("ja", "東京"))),
        "seoul" => ("Seoul", "", Some(("ko", "서울"))),
        "bangkok" => ("Bangkok", "", Some(("th", "กรุงเทพมหานคร"))),
        "addis-ababa" => ("Addis Ababa", "", Some(("am", "አዲስ አበባ"))),
        "new-york" => ("New York", "", None),
        "swiss-alps" => ("Lauterbrunnen", "Bern", None),
        "rio" => ("Rio de Janeiro", "", None),
        "sao-paulo" => ("São Paulo", "", None),
        "lisbon" => ("Lisbon", "", Some(("pt", "Lisboa"))),
        "uluru" => ("Uluru", "Northern Territory", None),
        _ => ("", "", None),
    };
    let primary = if primary.is_empty() {
        p.key
            .split('-')
            .map(|word| {
                let mut chars = word.chars();
                chars.next().unwrap().to_uppercase().collect::<String>() + chars.as_str()
            })
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        primary.into()
    };
    ArrivalTitle {
        profile: p,
        primary,
        region: (!region.is_empty()).then(|| region.into()),
        localized_names: local
            .into_iter()
            .map(|(a, b)| (a.into(), b.into()))
            .collect(),
    }
}

fn read_image(path: &Path) -> Result<egui::ColorImage> {
    let mut decoder = png::Decoder::new(BufReader::new(fs::File::open(path)?));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info()?;
    let mut data = vec![0; reader.output_buffer_size().ok_or("image too big")?];
    let info = reader.next_frame(&mut data)?;
    let size = [info.width as usize, info.height as usize];
    match info.color_type {
        png::ColorType::Rgb => Ok(egui::ColorImage::from_rgb(
            size,
            &data[..info.buffer_size()],
        )),
        png::ColorType::Rgba => Ok(egui::ColorImage::from_rgba_unmultiplied(
            size,
            &data[..info.buffer_size()],
        )),
        _ => Err("background must be RGB or RGBA".into()),
    }
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let output = PathBuf::from(args.get(1).ok_or("expected output directory")?);
    fs::create_dir_all(&output)?;
    let profiles: Vec<_> = catalog::PROFILES
        .iter()
        .filter(|p| args.get(2).is_none_or(|key| key == p.key))
        .collect();
    if profiles.is_empty() {
        return Err("unknown profile".into());
    }
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))?;
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default()))?;
    let ctx = egui::Context::default();
    vearth_app::ui::configure_fonts(&ctx);
    let background = args
        .get(3)
        .map(|p| read_image(Path::new(p)))
        .transpose()?
        .map(|image| ctx.load_texture("current game capture", image, egui::TextureOptions::LINEAR));
    let [width, height] = if background.is_some() {
        [1600u32, 900u32]
    } else {
        [960, 540]
    };
    let viewport = Rect::from_min_size(Pos2::ZERO, vec2(width as f32, height as f32));
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let mut renderer =
        egui_wgpu::Renderer::new(&device, format, egui_wgpu::RendererOptions::default());
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("arrival typography proof"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let mut html = String::from(
        "<!doctype html><meta charset=utf-8><title>Native title proofs</title><style>body{background:#0b0c0d;color:#d8d8d0;font:15px system-ui;margin:32px}main{display:grid;grid-template-columns:repeat(auto-fit,minmax(420px,1fr));gap:30px}img{width:100%}a{color:inherit}h1{font-size:24px}p{color:#a4a5a0}figure{margin:0}figcaption{padding:10px 0}</style><h1>Arrival titles · native renderer proofs</h1><p>Same fonts, shaping, layout and GPU paint function as the game. Specimen names exercise the catalog; gameplay always uses resolved source names.</p><main>",
    );
    for p in profiles {
        let title = specimen(p);
        let frame = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(viewport),
                ..Default::default()
            },
            |ui| {
                if let Some(bg) = &background {
                    ui.ctx().layer_painter(egui::LayerId::background()).image(
                        bg.id(),
                        viewport,
                        Rect::from_min_max(Pos2::ZERO, egui::pos2(1., 1.)),
                        Color32::WHITE,
                    );
                }
                location_title::paint(ui.ctx(), &title, 2., 1.);
            },
        );
        let primitives = ctx.tessellate(frame.shapes, frame.pixels_per_point);
        for (id, delta) in &frame.textures_delta.set {
            renderer.update_texture(&device, &queue, *id, delta);
        }
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [width, height],
            pixels_per_point: frame.pixels_per_point,
        };
        let mut encoder = device.create_command_encoder(&Default::default());
        let extra = renderer.update_buffers(&device, &queue, &mut encoder, &primitives, &screen);
        {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.025,
                            g: 0.03,
                            b: 0.035,
                            a: 1.,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            renderer.render(&mut pass.forget_lifetime(), &primitives, &screen);
        }
        let stride = (width * 4).div_ceil(256) * 256;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (stride * height) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(extra.into_iter().chain([encoder.finish()]));
        let slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        device.poll(wgpu::PollType::wait_indefinitely())?;
        rx.recv()??;
        let mapped = slice.get_mapped_range()?;
        let rgba: Vec<_> = mapped
            .chunks_exact(stride as usize)
            .flat_map(|row| row[..(width * 4) as usize].iter().copied())
            .collect();
        vearth_app::capture::write_png(
            &output.join(format!("{}.png", p.key)),
            width,
            height,
            &rgba,
        )?;
        drop(mapped);
        buffer.unmap();
        for id in &frame.textures_delta.free {
            renderer.free_texture(id);
        }
        html.push_str(&format!("<figure><a href='{0}.png'><img loading=lazy src='{0}.png'></a><figcaption>{0} · {1:?} · {2:?}</figcaption></figure>",p.key,p.face,p.finish));
        println!("{}", p.key);
    }
    html.push_str("</main>");
    fs::write(output.join("index.html"), html)?;
    Ok(())
}
