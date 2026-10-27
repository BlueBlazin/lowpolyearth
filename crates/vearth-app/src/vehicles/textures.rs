use std::io::Cursor;

const SIDE: u32 = 2048;
const LEVELS: u32 = SIDE.ilog2() + 1;
pub(crate) const LAYERS: u32 = 2;
fn source(layer: usize) -> &'static [u8] {
    match layer {
        0 => vearth_platform::asset_bytes!(
            "../../../../assets/vehicles/refined/textures/kestrel.png"
        ),
        1 => vearth_platform::asset_bytes!(
            "../../../../assets/vehicles/refined/textures/phase_manta.png"
        ),
        _ => panic!("invalid vehicle texture layer {layer}"),
    }
}

fn decode(source: &[u8]) -> Vec<u8> {
    let mut decoder = png::Decoder::new(Cursor::new(source));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder
        .read_info()
        .expect("authored vehicle texture header");
    let mut bytes = vec![
        0;
        reader
            .output_buffer_size()
            .expect("bounded vehicle texture")
    ];
    let info = reader
        .next_frame(&mut bytes)
        .expect("authored vehicle texture pixels");
    assert_eq!((info.width, info.height), (SIDE, SIDE));
    match info.color_type {
        png::ColorType::Rgb => bytes[..info.buffer_size()]
            .chunks_exact(3)
            .flat_map(|rgb| [rgb[0], rgb[1], rgb[2], 255])
            .collect(),
        png::ColorType::Rgba => bytes[..info.buffer_size()].to_vec(),
        _ => panic!("vehicle base color must be RGB or RGBA"),
    }
}

fn linear(value: u8) -> f32 {
    let v = value as f32 / 255.0;
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

fn srgb(value: f32) -> u8 {
    let v = if value <= 0.0031308 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    };
    (v * 255.0).round().clamp(0.0, 255.0) as u8
}

fn reduce(source: &[u8], side: usize) -> Vec<u8> {
    let next = side / 2;
    let mut output = vec![255; next * next * 4];
    let lut: [f32; 256] = std::array::from_fn(|i| linear(i as u8));
    for y in 0..next {
        for x in 0..next {
            for c in 0..3 {
                let mut sum = 0.0;
                for dy in 0..2 {
                    for dx in 0..2 {
                        sum += lut[source[((y * 2 + dy) * side + x * 2 + dx) * 4 + c] as usize];
                    }
                }
                output[(y * next + x) * 4 + c] = srgb(sum * 0.25);
            }
        }
    }
    output
}

/// The base-color array, written by [`write_layer`].
pub(crate) fn create(device: &wgpu::Device) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("authored vehicle base colors"),
        size: wgpu::Extent3d {
            width: SIDE,
            height: SIDE,
            depth_or_array_layers: LAYERS,
        },
        mip_level_count: LEVELS,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

/// RGBA8 mip chain of one layer, largest first. CPU work only.
pub(crate) fn decode_layer(layer: u32) -> Vec<Vec<u8>> {
    let mut levels = vec![decode(source(layer as usize))];
    for mip in 0..LEVELS - 1 {
        let side = (SIDE >> mip) as usize;
        // Average light, not sRGB bytes: small distant panels retain
        // their mean brightness instead of turning into dark seams.
        levels.push(reduce(&levels[mip as usize], side));
    }
    levels
}

pub(crate) fn write_layer(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    layer: u32,
    levels: &[Vec<u8>],
) {
    for (mip, pixels) in (0..LEVELS).zip(levels) {
        let side = SIDE >> mip;
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: mip,
                origin: wgpu::Origin3d {
                    x: 0,
                    y: 0,
                    z: layer,
                },
                aspect: wgpu::TextureAspect::All,
            },
            pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(side * 4),
                rows_per_image: Some(side),
            },
            wgpu::Extent3d {
                width: side,
                height: side,
                depth_or_array_layers: 1,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authored_textures_decode_at_the_runtime_array_size() {
        for layer in 0..LAYERS {
            let levels = decode_layer(layer);
            assert_eq!(levels.len(), LEVELS as usize);
            for (mip, pixels) in levels.iter().enumerate() {
                assert_eq!(pixels.len(), ((SIDE >> mip) * (SIDE >> mip) * 4) as usize);
            }
            assert!(levels[0].chunks_exact(4).all(|p| p[3] == 255));
        }
    }

    #[test]
    fn mip_filter_preserves_linear_light_and_constant_colors() {
        let alternating = [0, 0, 0, 255, 255, 255, 255, 255].repeat(2);
        assert_eq!(reduce(&alternating, 2), [188, 188, 188, 255]);
        assert_eq!(
            reduce(&[83, 161, 209, 255].repeat(4), 2),
            [83, 161, 209, 255]
        );
    }
}
