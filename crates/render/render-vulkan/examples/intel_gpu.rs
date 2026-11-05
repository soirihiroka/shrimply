#[cfg(target_os = "linux")]
fn main() -> Result<(), String> {
    use ffmpeg_next::{self as ffmpeg, media};
    use shrimply_render_core::{LayerKind, Nv12LayerParams, TextureAddressMode, VideoSampleMethod};
    use shrimply_render_vulkan::{Renderer, VideoFrame};
    use std::{sync::Arc, time::Instant};

    let mut arguments = std::env::args().skip(1);
    let path = arguments
        .next()
        .ok_or("Pass a video path and optional frame limit")?;
    let limit = arguments
        .next()
        .map(|value| value.parse::<usize>().map_err(|e| e.to_string()))
        .transpose()?
        .unwrap_or(120);
    if limit == 0 {
        return Err("Frame limit must be positive".into());
    }
    ffmpeg::init().map_err(|e| e.to_string())?;
    let mut input = ffmpeg::format::input(&path).map_err(|e| e.to_string())?;
    let stream = input
        .streams()
        .best(media::Type::Video)
        .ok_or("No video stream")?;
    let index = stream.index();
    let mut decoder = shrimply_render_vulkan::decoder(&stream.parameters(), stream.time_base())?;
    let mut renderer = Renderer::new()?;
    renderer.warmup()?;
    let started = Instant::now();
    let mut count = 0;
    let mut render = |frame: ffmpeg::frame::Video, count: &mut usize| -> Result<(), String> {
        let width = frame.width();
        let height = frame.height();
        let source = Arc::new(VideoFrame { frame });
        let (rgba, decoded) = renderer.video(&source)?;
        let parameters = Nv12LayerParams {
            crop: [0.0; 4],
            padding: [0.0; 4],
            y_plane: std::ptr::null(),
            uv_plane: std::ptr::null(),
            rgba: std::ptr::null(),
            y_pitch: 0,
            uv_pitch: 0,
            rgba_pitch: width as usize * size_of::<u32>(),
            source_width: width,
            source_height: height,
            canvas_width: width,
            inverse: shrimply_render_core::math::Mat3::IDENTITY,
            motion_transform_offset: 0,
            motion_transform_count: 0,
            motion_sample_count: 0,
            opacity: 1.0,
            blend_mode: shrimply_render_core::LayerBlendMode::Normal,
            sample_method: VideoSampleMethod::Nearest,
            address_mode: TextureAddressMode::Transparent,
            kind: LayerKind::Rgba,
            _padding_0: [0; 4],
        };
        let composite = renderer.composite_buffers(&[(parameters, rgba)], width, height, 0)?;
        let (texture, presentation) =
            renderer.presentation_texture(&composite.output, width, height)?;
        presentation.wait()?;
        // Exercise external-memory/semaphore export without downloading playback pixels.
        drop(texture.export()?);
        if *count == 0 {
            let (output, submission) = composite.into_parts();
            let pixels = renderer.readback(&output)?;
            println!(
                "First frame: {width}x{height}, RGBA checksum {}",
                pixels.iter().map(|&v| u64::from(v)).sum::<u64>()
            );
            drop(submission);
        }
        drop(decoded);
        *count += 1;
        Ok(())
    };
    'packets: for (stream, packet) in input.packets() {
        if stream.index() != index {
            continue;
        }
        decoder.send_packet(&packet).map_err(|e| e.to_string())?;
        loop {
            let mut frame = ffmpeg::frame::Video::empty();
            match decoder.receive_frame(&mut frame) {
                Ok(()) => render(frame, &mut count)?,
                Err(ffmpeg::Error::Other { errno }) if errno == ffmpeg::error::EAGAIN => break,
                Err(error) => return Err(error.to_string()),
            }
            if count >= limit {
                break 'packets;
            }
        }
    }
    if count < limit {
        decoder.send_eof().map_err(|e| e.to_string())?;
        while count < limit {
            let mut frame = ffmpeg::frame::Video::empty();
            match decoder.receive_frame(&mut frame) {
                Ok(()) => render(frame, &mut count)?,
                Err(ffmpeg::Error::Eof) => break,
                Err(error) => return Err(error.to_string()),
            }
        }
    }
    if count == 0 {
        return Err("Video produced no frames".into());
    }
    let elapsed = started.elapsed();
    println!(
        "{count} frames in {elapsed:?}; {} fps (decode, Slang composite, texture export)",
        count as u128 * 1_000_000_000 / elapsed.as_nanos().max(1)
    );
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn main() {
    panic!("The Intel GPU benchmark requires Linux");
}
