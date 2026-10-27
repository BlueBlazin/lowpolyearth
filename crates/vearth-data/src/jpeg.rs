use crate::DataError;
use zune_jpeg::zune_core::bytestream::ZCursor;
use zune_jpeg::zune_core::colorspace::ColorSpace;
use zune_jpeg::zune_core::options::DecoderOptions;

pub(crate) fn decode_rgb(bytes: &[u8]) -> Result<(usize, usize, Vec<u8>), DataError> {
    let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGB);
    let mut decoder = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(bytes), options);
    let rgb = decoder
        .decode()
        .map_err(|error| DataError::Decode(format!("JPEG: {error}")))?;
    let (width, height) = decoder
        .dimensions()
        .ok_or_else(|| DataError::Decode("JPEG has no dimensions".into()))?;
    let expected_len = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(3))
        .ok_or_else(|| DataError::Decode("JPEG dimensions overflow".into()))?;
    if rgb.len() != expected_len {
        return Err(DataError::Decode(format!(
            "JPEG decoded as {width}x{height}/{} bytes; expected {expected_len} RGB bytes",
            rgb.len()
        )));
    }
    Ok((width, height, rgb))
}
