use objc2_metal::MTLBuffer;
use shrimply_project_document::project::CanvasSize;
use skia_safe::Image;

/// Materialize CPU pixels only for an explicit capture/export request. Callers
/// hold a completed compositor frame; ordinary preview presentation stays on GPU.
pub(super) fn image(
    buffer: &shrimply_render_metal::Buffer,
    size: (u32, u32),
) -> Result<Image, String> {
    let info = skia_safe::ImageInfo::new(
        (size.0 as i32, size.1 as i32),
        skia_safe::ColorType::RGBA8888,
        skia_safe::AlphaType::Unpremul,
        None,
    );
    // Published frames have completed all GPU writes before reaching this path.
    let pixels = unsafe {
        std::slice::from_raw_parts(
            buffer.metal().contents().as_ptr().cast::<u8>(),
            buffer.metal().length(),
        )
    };
    skia_safe::images::raster_from_data(
        &info,
        skia_safe::Data::new_copy(pixels),
        info.min_row_bytes(),
    )
    .ok_or_else(|| "Could not read the completed Metal frame".into())
}

/// Read a completed compositor image as tightly packed, straight-alpha RGBA.
pub(super) fn rgba(image: &Image, size: CanvasSize) -> Result<Vec<u8>, String> {
    if image.width() != size.width as i32 || image.height() != size.height as i32 {
        return Err("Captured image does not match the requested canvas size".into());
    }
    let info = skia_safe::ImageInfo::new(
        image.dimensions(),
        skia_safe::ColorType::RGBA8888,
        skia_safe::AlphaType::Unpremul,
        None,
    );
    let stride = info.min_row_bytes();
    let mut pixels = vec![0_u8; info.compute_byte_size(stride)];
    if !image.read_pixels(
        &info,
        &mut pixels,
        stride,
        (0, 0),
        skia_safe::image::CachingHint::Disallow,
    ) {
        return Err("Could not read the captured frame".into());
    }
    Ok(pixels)
}
