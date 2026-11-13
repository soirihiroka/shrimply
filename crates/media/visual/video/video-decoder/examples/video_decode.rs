//! Blocking, GPU-ready decode throughput and random-seek latency. No compositing or encoding.
use std::{
    hint::black_box,
    sync::Arc,
    time::{Duration, Instant},
};

use ffmpeg_next as ffmpeg;
use shrimply_gpu_cuda::CudaContext;
use shrimply_math_core::{Time, fraction_new, frame_index};
use shrimply_project_document::project::{Asset, CanvasSize, VideoItem, VideoItemContent};
use shrimply_video_decoder::{
    DecodeMode, DecodeOutcome, DecodeRequest, VideoDecoderPool, VideoPlane,
};
use shrimply_visual_frame::{GpuFrame, cuda::ImportedFrame};

const WARMUP_FRAMES: i64 = 30;
const SEEK_STRIDE: i64 = 97;
const DEFAULT_RUNS: usize = 5;
const DEFAULT_FRAMES: i64 = 240;
const DEFAULT_SEEKS: i64 = 40;

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .ok_or("usage: video_decode VIDEO [RUNS] [FRAMES] [SEEKS]")?;
    let runs = args
        .next()
        .map(|value| value.parse::<usize>().map_err(|error| error.to_string()))
        .transpose()?
        .unwrap_or(DEFAULT_RUNS);
    let frames = args
        .next()
        .map(|value| value.parse::<i64>().map_err(|error| error.to_string()))
        .transpose()?
        .unwrap_or(DEFAULT_FRAMES);
    let seeks = args
        .next()
        .map(|value| value.parse::<i64>().map_err(|error| error.to_string()))
        .transpose()?
        .unwrap_or(DEFAULT_SEEKS);
    if runs == 0 || frames <= 0 || seeks <= 0 || args.next().is_some() {
        return Err(
            "runs, frames, and seeks must be positive; no extra arguments are accepted".into(),
        );
    }
    ffmpeg::init().map_err(|error| error.to_string())?;
    let input = ffmpeg::format::input(&path).map_err(|error| error.to_string())?;
    let stream = input
        .streams()
        .find(|stream| {
            stream.parameters().medium() == ffmpeg::media::Type::Video
                && !stream
                    .disposition()
                    .contains(ffmpeg::format::stream::Disposition::ATTACHED_PIC)
        })
        .ok_or("input has no video track")?;
    let rate = stream.avg_frame_rate();
    if rate.numerator() <= 0 || rate.denominator() <= 0 || input.duration() <= 0 {
        return Err("benchmark requires a known duration and frame rate".into());
    }
    let fps = fraction_new(i64::from(rate.numerator()), i64::from(rate.denominator()));
    let duration = Time::from_fraction(input.duration(), i64::from(ffmpeg::ffi::AV_TIME_BASE));
    let source_frames = frame_index(duration, fps).ok_or("source frame count exceeds i64")?;
    if source_frames < frames + WARMUP_FRAMES {
        return Err(format!(
            "input needs at least {} frames, found {source_frames}",
            frames + WARMUP_FRAMES
        ));
    }
    let parameters = stream.parameters();
    let raw = unsafe { &*parameters.as_ptr() };
    let size = CanvasSize {
        width: u32::try_from(raw.width).map_err(|_| "invalid video width")?,
        height: u32::try_from(raw.height).map_err(|_| "invalid video height")?,
    };
    let mut item = VideoItem::background_item(size, Time::ZERO, duration);
    item.content = VideoItemContent::Media;
    item.file = Asset::new(path);
    item.source_width = size.width;
    item.source_height = size.height;
    item.source_duration = duration;
    item.playback_fps = fps;
    println!(
        "input={} size={}x{} fps={}/{} codec={:?} runs={runs} frames={frames} seeks={seeks} warmup={WARMUP_FRAMES}",
        item.file.display(),
        size.width,
        size.height,
        rate.numerator(),
        rate.denominator(),
        parameters.id()
    );
    drop(input);
    let device = shrimply_video_decoder::DecoderBackend::cuda()?;
    let mut context = CudaContext::new(shrimply_gpu_cuda::selected_device())
        .map_err(|error| error.to_string())?;
    drop(device);
    for run in 0..runs {
        let mut pool = VideoDecoderPool::new(1);
        let owner = pool.owner(&[], uuid::Uuid::new_v4(), item.id, VideoPlane::Color);
        let decoder = pool.decoder(&item, owner)?;
        let started = Instant::now();
        decode_ready(&decoder, &mut context, Time::ZERO, DecodeMode::Accurate)?;
        println!(
            "run={run} mode=open_first_frame ready_us={}",
            started.elapsed().as_micros()
        );
        for frame in 1..WARMUP_FRAMES {
            decode_ready(
                &decoder,
                &mut context,
                shrimply_math_core::time_from_frame(frame as u64, fps)
                    .ok_or("frame timestamp overflow")?,
                DecodeMode::Continuous,
            )?;
        }
        for (name, mode, count) in [
            ("sequential", DecodeMode::Continuous, frames),
            ("seek", DecodeMode::Accurate, seeks),
        ] {
            let mut decode = Vec::with_capacity(count as usize);
            let mut import = Vec::with_capacity(count as usize);
            let mut ready = Vec::with_capacity(count as usize);
            let started = Instant::now();
            for index in 0..count {
                let frame = if mode.continuous() {
                    WARMUP_FRAMES + index
                } else {
                    (index * SEEK_STRIDE) % source_frames
                };
                let timing = decode_ready(
                    &decoder,
                    &mut context,
                    shrimply_math_core::time_from_frame(frame as u64, fps)
                        .ok_or("frame timestamp overflow")?,
                    mode,
                )?;
                decode.push(timing.0);
                import.push(timing.1);
                ready.push(timing.0 + timing.1);
            }
            let elapsed = started.elapsed();
            ready.sort_unstable();
            println!(
                "run={run} mode={name} frames={count} wall_us={} ready_fps_milli={} decode_us={} import_ready_us={} ready_p50_us={} ready_p95_us={}",
                elapsed.as_micros(),
                count as u128 * 1_000_000_000_000 / elapsed.as_nanos(),
                decode.iter().sum::<Duration>().as_micros(),
                import.iter().sum::<Duration>().as_micros(),
                ready[ready.len() / 2].as_micros(),
                ready[(ready.len() * 95).div_ceil(100) - 1].as_micros()
            );
        }
        drop(decoder);
        pool.retain(std::iter::empty());
        pool.reclaim_idle()
            .recv()
            .map_err(|_| "decoder retirement worker stopped")?;
    }
    Ok(())
}

fn decode_ready(
    decoder: &shrimply_video_decoder::VideoDecoderHandle,
    context: &mut Arc<CudaContext>,
    time: Time,
    mode: DecodeMode,
) -> Result<(Duration, Duration), String> {
    let started = Instant::now();
    let DecodeOutcome::Frame(Some((_, source))) =
        decoder.request(DecodeRequest::new(time, mode))?.receive()?
    else {
        return Err("benchmark decode was superseded or produced no frame".into());
    };
    let decode = started.elapsed();
    let started = Instant::now();
    let imported = ImportedFrame::import(context, source)?;
    black_box(
        imported
            .frame
            .plane(0)
            .ok_or("decoded frame has no CUDA plane")?,
    );
    drop(imported);
    Ok((decode, started.elapsed()))
}
