//! Bounded PNG decoding for watched voxel texture assets, independent of Skia.
use crate::Limits;
use std::{io::Cursor, sync::Arc};

pub fn decode_texture(bytes: &[u8], limits: Limits) -> Result<Arc<sanim_voxel::Texture>, String> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    decoder.set_limits(png::Limits {
        bytes: limits.max_bytes,
    });
    let mut reader = decoder
        .read_info()
        .map_err(|error| format!("voxel texture PNG: {error}"))?;
    let info = reader.info();
    let pixels = (info.width as usize)
        .checked_mul(info.height as usize)
        .ok_or("voxel texture dimensions overflow")?;
    let rgba_bytes = pixels.checked_mul(4).ok_or("voxel texture size overflow")?;
    if rgba_bytes > limits.max_bytes || pixels > limits.max_values {
        return Err("voxel texture exceeds the decoded pixel or byte budget".into());
    }
    let size = reader
        .output_buffer_size()
        .ok_or("voxel texture PNG output size overflow")?;
    if size > limits.max_bytes {
        return Err("voxel texture exceeds the decoded byte budget".into());
    }
    let mut decoded = vec![0; size];
    let frame = reader
        .next_frame(&mut decoded)
        .map_err(|error| format!("voxel texture PNG: {error}"))?;
    if reader.info().animation_control.is_some() {
        return Err("voxel textures require a static PNG; animated PNG is not supported".into());
    }
    decoded.truncate(frame.buffer_size());
    let rgba = match frame.color_type {
        png::ColorType::Rgba => decoded,
        png::ColorType::Rgb => decoded
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::Grayscale => decoded.iter().flat_map(|v| [*v, *v, *v, 255]).collect(),
        png::ColorType::GrayscaleAlpha => decoded
            .chunks_exact(2)
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Indexed => return Err("voxel texture PNG palette was not expanded".into()),
    };
    sanim_voxel::Texture::new(frame.width, frame.height, rgba.into()).map(Arc::new)
}
