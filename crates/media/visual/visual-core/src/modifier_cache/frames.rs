use super::{
    BakeRequest, Progress,
    encoder::{HevcBackend, RgbaAlpha, VideoCacheEncoder},
};
use shrimply_project_document::project::{ItemAddress, Project, Time};
use shrimply_resource_pipeline::JobContext;

/// Backends supply completed RGBA frames; core owns timing, cancellation and encoding.
pub fn render_and_encode(
    request: BakeRequest,
    context: &JobContext<Progress>,
    backend: HevcBackend,
    alpha: RgbaAlpha,
    render: impl FnMut(
        &Project,
        Time,
        &ItemAddress,
    ) -> Result<Option<ffmpeg_next::frame::Video>, String>,
) -> Result<(), String> {
    let mut encoder = VideoCacheEncoder::new(
        &request.output,
        request.coded_width,
        request.coded_height,
        request.project.fps,
        request.settings.quality.qp(),
        backend,
    )?;
    render_frames(&request, context, render, |frame, index| {
        encoder.write(&frame, index, alpha)
    })?;
    encoder.finish()
}

pub fn render_and_store_images(
    request: BakeRequest,
    context: &JobContext<Progress>,
    render: impl FnMut(&Project, Time, &ItemAddress) -> Result<Option<Vec<u8>>, String>,
) -> Result<(), String> {
    std::fs::create_dir(&request.output)
        .map_err(|error| format!("could not create cache frames folder: {error}"))?;
    render_frames(&request, context, render, |pixels, index| {
        let png = shrimply_drawing_skia::media::encode_rgba_png(
            i32::try_from(request.width).map_err(|_| "cache width is too large")?,
            i32::try_from(request.height).map_err(|_| "cache height is too large")?,
            &pixels,
        )?;
        std::fs::write(request.output.join(super::image_frame_name(index)), png)
            .map_err(|error| format!("could not write cache frame {index}: {error}"))
    })
}

fn render_frames<T>(
    request: &BakeRequest,
    context: &JobContext<Progress>,
    mut render: impl FnMut(&Project, Time, &ItemAddress) -> Result<Option<T>, String>,
    mut store: impl FnMut(T, u64) -> Result<(), String>,
) -> Result<(), String> {
    for frame_index in 0..request.total_frames {
        let position = shrimply_math_core::time_from_frame(
            request.first_frame.saturating_add(frame_index),
            request.project.fps,
        )
        .ok_or("cache frame rate must be positive")?;
        let frame = loop {
            if context.is_cancelled() {
                return Err("visual cache bake cancelled".into());
            }
            if let Some(frame) = render(&request.project, position, &request.address)? {
                break frame;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        };
        if context.is_cancelled() {
            return Err("visual cache bake cancelled".into());
        }
        store(frame, frame_index)?;
        if !context.report(Progress {
            completed: frame_index + 1,
            total: request.total_frames,
        }) {
            return Err("visual cache bake cancelled".into());
        }
    }
    Ok(())
}
