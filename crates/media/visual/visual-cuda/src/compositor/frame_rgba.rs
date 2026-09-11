use shrimply_project_document::project::{Project, Time};

pub struct RgbaVideoFrame {
    pub width: i32,
    pub height: i32,
    pub pixels: Vec<u8>,
}

pub fn render_items_rgba(
    project: Project,
    position: Time,
    item_ids: &[uuid::Uuid],
) -> Result<RgbaVideoFrame, String> {
    let canvas_size = project.canvas_size;
    let mut renderer = super::VideoExportRenderer::new(48_000)?;
    let frame = renderer.render_items(&project, position, 0, item_ids)?;
    let mut rgba = ffmpeg_next::frame::Video::new(
        ffmpeg_next::format::Pixel::RGBA,
        canvas_size.width,
        canvas_size.height,
    );
    renderer.copy_to_rgba_frame(frame, &mut rgba)?;
    let width = i32::try_from(canvas_size.width)
        .map_err(|_| "selected frame width is too large".to_string())?;
    let height = i32::try_from(canvas_size.height)
        .map_err(|_| "selected frame height is too large".to_string())?;
    let row_bytes = canvas_size.width as usize * std::mem::size_of::<u32>();
    let stride = rgba.stride(0);
    let mut pixels = Vec::with_capacity(row_bytes * canvas_size.height as usize);
    for row in rgba
        .data(0)
        .chunks_exact(stride)
        .take(canvas_size.height as usize)
    {
        pixels.extend_from_slice(&row[..row_bytes]);
    }
    Ok(RgbaVideoFrame {
        width,
        height,
        pixels,
    })
}
