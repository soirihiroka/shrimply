//! Bounded, watched PNG/JPEG decoding for the shared immutable Texture value.
use crate::Limits;
use image::{
    DynamicImage, ImageDecoder, ImageFormat,
    codecs::{jpeg::JpegDecoder, png::PngDecoder},
};
use std::{io::Cursor, sync::Arc};

pub fn decode(bytes: &[u8], limits: Limits) -> Result<Arc<sanim_voxel::Texture>, String> {
    let format = image::guess_format(bytes).map_err(|error| format!("texture format: {error}"))?;
    // image's JPEG wrapper copies its encoded input before returning a decoder;
    // header parsing may also retain metadata. Reserve these before construction.
    let encoded_peak = bytes
        .len()
        .checked_mul(if format == ImageFormat::Jpeg { 4 } else { 1 })
        .ok_or("texture encoded byte count overflow")?;
    if encoded_peak >= limits.max_bytes {
        return Err("texture encoded decoder buffers exceed the host byte budget".into());
    }
    let mut decoder_limits = image::Limits::default();
    decoder_limits.max_alloc = Some((limits.max_bytes - encoded_peak) as u64);
    match format {
        ImageFormat::Png => {
            // Probe the fixed IHDR dimensions before the decoder may allocate
            // metadata or pixel buffers. The actual decoder validates the file.
            let header = bytes.get(12..24).ok_or("texture PNG header is truncated")?;
            if &header[..4] != b"IHDR" {
                return Err("texture PNG has no initial IHDR chunk".into());
            }
            let width = u32::from_be_bytes(header[4..8].try_into().expect("IHDR width"));
            let height = u32::from_be_bytes(header[8..12].try_into().expect("IHDR height"));
            let rgba = sanim_voxel::texture_bytes(width, height)?;
            let reserved = rgba
                .checked_mul(4)
                .and_then(|value| value.checked_add(encoded_peak))
                .ok_or("texture decoding byte count overflow")?;
            if reserved >= limits.max_bytes {
                return Err(
                    "texture decoder pixels and encoded input exceed the host byte budget".into(),
                );
            }
            decoder_limits.max_alloc = Some((limits.max_bytes - reserved) as u64);
            let decoder = PngDecoder::with_limits(Cursor::new(bytes), decoder_limits.clone())
                .map_err(|error| format!("texture PNG: {error}"))?;
            if decoder
                .is_apng()
                .map_err(|error| format!("texture PNG: {error}"))?
            {
                return Err(
                    "Texture requires a static image; animated PNG is not supported".into(),
                );
            }
            pixels(decoder, encoded_peak, limits, decoder_limits)
        }
        ImageFormat::Jpeg => pixels(
            JpegDecoder::new(Cursor::new(bytes))
                .map_err(|error| format!("texture JPEG: {error}"))?,
            encoded_peak,
            limits,
            decoder_limits,
        ),
        format => Err(format!(
            "read<Texture> supports static PNG or JPEG, not {format:?}"
        )),
    }
}

fn pixels(
    mut decoder: impl ImageDecoder,
    encoded_bytes: usize,
    limits: Limits,
    decoder_limits: image::Limits,
) -> Result<Arc<sanim_voxel::Texture>, String> {
    let (width, height) = decoder.dimensions();
    let rgba_bytes = sanim_voxel::texture_bytes(width, height)?;
    // 16-bit PNG may retain an eight-byte source and four-byte conversion per
    // pixel. Vec-to-Arc publication also briefly duplicates the RGBA8 output.
    let peak = rgba_bytes
        .checked_mul(4)
        .and_then(|bytes| bytes.checked_add(encoded_bytes))
        .ok_or("texture decoding byte count overflow")?;
    if peak > limits.max_bytes || rgba_bytes / 4 > limits.max_values {
        return Err(format!(
            "texture decoding requires {peak} peak bytes and {} pixels, exceeding the host decoded byte or pixel budget",
            rgba_bytes / 4
        ));
    }
    decoder
        .set_limits(decoder_limits)
        .map_err(|error| format!("texture decoding limits: {error}"))?;
    let rgba = DynamicImage::from_decoder(decoder)
        .map_err(|error| format!("texture decoding: {error}"))?
        .into_rgba8();
    sanim_voxel::Texture::new(width, height, rgba.into_raw().into()).map(Arc::new)
}
