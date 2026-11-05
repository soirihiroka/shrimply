use shrimply_project_document::project::{Project, Time};
use std::sync::atomic::{AtomicBool, Ordering};

const FRAME_RENDER_AUDIO_SAMPLE_RATE: u32 = 48_000;

pub struct RgbaVideoFrame {
    pub width: i32,
    pub height: i32,
    pub pixels: Vec<u8>,
}

pub fn render_project_rgba(
    project: &Project,
    position: Time,
    cancelled: &AtomicBool,
) -> Result<RgbaVideoFrame, String> {
    #[cfg(target_os = "linux")]
    if crate::gpu::intel_backend_selected() {
        return read_intel_frame(project, position, None, cancelled);
    }
    let mut renderer = super::VideoExportRenderer::new(FRAME_RENDER_AUDIO_SAMPLE_RATE)?;
    let frame = loop {
        if cancelled.load(Ordering::Acquire) {
            return Err("Frame render cancelled".into());
        }
        match renderer.render(project, position, 0) {
            Ok(frame) => break frame,
            Err(error) if error == super::EXPORT_ASSETS_LOADING => std::thread::yield_now(),
            Err(error) => return Err(error),
        }
    };
    read_cuda_frame(&mut renderer, frame, project.canvas_size)
}

pub fn render_items_rgba(
    project: Project,
    position: Time,
    item_ids: &[uuid::Uuid],
) -> Result<RgbaVideoFrame, String> {
    if item_ids.is_empty() {
        return Err("no visual items were selected".into());
    }
    #[cfg(target_os = "linux")]
    if crate::gpu::intel_backend_selected() {
        return read_intel_frame(
            &project,
            position,
            Some(shrimply_preview_render_core::CaptureTarget::Items(
                item_ids.to_vec(),
            )),
            &AtomicBool::new(false),
        );
    }
    let canvas_size = project.canvas_size;
    let mut renderer = super::VideoExportRenderer::new(FRAME_RENDER_AUDIO_SAMPLE_RATE)?;
    let frame = renderer.render_items(&project, position, 0, item_ids)?;
    read_cuda_frame(&mut renderer, frame, canvas_size)
}

#[cfg(target_os = "linux")]
fn read_intel_frame(
    project: &Project,
    position: Time,
    target: Option<shrimply_preview_render_core::CaptureTarget>,
    cancelled: &AtomicBool,
) -> Result<RgbaVideoFrame, String> {
    let mut compositor = shrimply_preview_render_vulkan::Compositor::new()?;
    compositor.set_capture_target(target);
    let rendered = compositor.render_accurate(project, position, 0, cancelled)?;
    Ok(RgbaVideoFrame {
        width: i32::try_from(rendered.width).map_err(|_| "frame width is too large")?,
        height: i32::try_from(rendered.height).map_err(|_| "frame height is too large")?,
        pixels: compositor.renderer.readback(&rendered.buffer)?,
    })
}

fn read_cuda_frame(
    renderer: &mut super::VideoExportRenderer,
    frame: crate::gpu::CompositedVideoFrame,
    canvas_size: shrimply_project_document::project::CanvasSize,
) -> Result<RgbaVideoFrame, String> {
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
