//! Screenshot-filled title, rasterized once from a bundled display font.

const WIDTH: usize = 1680;
const HEIGHT: usize = 756;
const OPACITY: f32 = 0.88;

pub(crate) fn texture(ctx: &egui::Context) -> egui::TextureHandle {
    let id = egui::Id::new("landscape title texture");
    if let Some(texture) = ctx.data(|data| data.get_temp::<egui::TextureHandle>(id)) {
        return texture;
    }
    let texture = ctx.load_texture("landscape title", image(), egui::TextureOptions::LINEAR);
    ctx.data_mut(|data| data.insert_temp(id, texture.clone()));
    texture
}

fn image() -> egui::ColorImage {
    let source = vearth_platform::asset_bytes!("../assets/launch-landscape.png");
    let decoder = png::Decoder::new(std::io::Cursor::new(source));
    let mut reader = decoder
        .read_info()
        .expect("embedded title landscape is PNG");
    let mut bytes = vec![
        0;
        reader
            .output_buffer_size()
            .expect("landscape fits in memory")
    ];
    let info = reader.next_frame(&mut bytes).expect("landscape decodes");
    let channels = match info.color_type {
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        other => panic!("unsupported landscape color type: {other:?}"),
    };
    assert_eq!(info.bit_depth, png::BitDepth::Eight);
    let source_width = info.width as usize;
    let source_height = info.height as usize;
    let scale = (source_width as f32 / WIDTH as f32).min(source_height as f32 / HEIGHT as f32);
    let crop_x = (source_width as f32 - WIDTH as f32 * scale) * 0.5;
    let crop_y = (source_height as f32 - HEIGHT as f32 * scale) * 0.5;

    // A dedicated 1:1 font context keeps the mask independent of monitor scale.
    // The oversampled texture retains antialiased glyph edges at Retina sizes.
    let fonts = egui::Context::default();
    let family = egui::FontFamily::Name("launch-title".into());
    let mut definitions = egui::FontDefinitions::empty();
    definitions.font_data.insert(
        "Archivo Black".into(),
        std::sync::Arc::new(egui::FontData::from_static(vearth_platform::asset_bytes!(
            "../assets/fonts/titles/ArchivoBlack-Regular.ttf"
        ))),
    );
    definitions
        .families
        .insert(family.clone(), vec!["Archivo Black".into()]);
    fonts.set_fonts(definitions);
    let mut lines = Vec::new();
    let _ = fonts.run_ui(egui::RawInput::default(), |ui| {
        for (text, size, center_y) in [
            ("Low Poly", WIDTH as f32 * 0.12, 0.22),
            ("Earth", WIDTH as f32 * 0.24, 0.65),
        ] {
            let galley = ui.painter().layout_no_wrap(
                text.to_owned(),
                egui::FontId::new(size, family.clone()),
                egui::Color32::WHITE,
            );
            let origin = egui::pos2(
                (WIDTH as f32 - galley.size().x) * 0.5,
                HEIGHT as f32 * center_y - galley.size().y * 0.5,
            );
            lines.push((origin, galley));
        }
    });
    let atlas = fonts.fonts(|fonts| fonts.image());
    let mut output = egui::ColorImage::filled([WIDTH, HEIGHT], egui::Color32::TRANSPARENT);
    for (origin, galley) in lines {
        for row in &galley.rows {
            // Unstyled egui glyphs are axis-aligned quads with texel-space UVs.
            let vertices = &row.visuals.mesh.vertices[row.visuals.glyph_vertex_range.clone()];
            for quad in vertices.chunks_exact(4) {
                let offset = origin.to_vec2() + row.pos.to_vec2();
                let bounds = egui::Rect::from_min_max(quad[0].pos + offset, quad[3].pos + offset);
                let uv = egui::Rect::from_min_max(quad[0].uv, quad[3].uv);
                for y in (bounds.top().ceil().max(0.0) as usize)
                    ..(bounds.bottom().ceil() as usize).min(HEIGHT)
                {
                    for x in (bounds.left().ceil().max(0.0) as usize)
                        ..(bounds.right().ceil() as usize).min(WIDTH)
                    {
                        let u = uv.left()
                            + (x as f32 + 0.5 - bounds.left()) / bounds.width() * uv.width();
                        let v = uv.top()
                            + (y as f32 + 0.5 - bounds.top()) / bounds.height() * uv.height();
                        let mask = atlas[(u as usize, v as usize)].a();
                        if mask == 0 {
                            continue;
                        }
                        let sx = ((x as f32 + 0.5) * scale + crop_x) as usize;
                        let sy = ((y as f32 + 0.5) * scale + crop_y) as usize;
                        let pixel = &bytes[(sy * source_width + sx) * channels..];
                        let source_alpha = if channels == 4 {
                            pixel[3] as f32 / 255.0
                        } else {
                            1.0
                        };
                        output[(x, y)] = egui::Color32::from_rgba_unmultiplied(
                            pixel[0],
                            pixel[1],
                            pixel[2],
                            (mask as f32 * OPACITY * source_alpha).round() as u8,
                        );
                    }
                }
            }
        }
    }
    output
}

#[cfg(test)]
mod tests {
    #[test]
    fn landscape_is_clipped_to_antialiased_translucent_glyphs() {
        let image = super::image();
        assert_eq!(image.size, [super::WIDTH, super::HEIGHT]);
        assert!(
            image.pixels.iter().filter(|pixel| pixel.a() == 0).count() > image.pixels.len() / 2
        );
        let max_alpha = image.pixels.iter().map(|pixel| pixel.a()).max().unwrap();
        assert_eq!(max_alpha, (255.0 * super::OPACITY).round() as u8);
        assert!(
            image
                .pixels
                .iter()
                .any(|pixel| pixel.a() > 0 && pixel.a() < max_alpha)
        );
        assert!(
            image
                .pixels
                .iter()
                .any(|pixel| pixel.g() > pixel.r() && pixel.a() == max_alpha)
        );
        assert!(
            image.pixels[..super::WIDTH]
                .iter()
                .all(|pixel| pixel.a() == 0)
        );
        assert!(
            image.pixels[super::WIDTH * (super::HEIGHT - 1)..]
                .iter()
                .all(|pixel| pixel.a() == 0)
        );
    }
}
