//! End-to-end verification and timeline compositor benchmark for experimental NVIDIA-only
//! frame-accurate scrubbing vs existing approximate scrubbing.
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use ffmpeg_next::{self as ffmpeg, sys};
use shrimply_math_core::{Fraction, Time, fraction_new, frame_index, time_from_frame};
use shrimply_preview_render_core::worker::{VideoCommand, VideoCommandSender, VideoEvent};
use shrimply_project_document::project::{
    Asset, CanvasSize, Project, VideoItem, VideoItemContent, VideoSampleMethod, VisualTrack,
};
use shrimply_video_decoder::{
    DecodeMode, DecodeOutcome, DecodeRequest, VideoDecoderPool, VideoPlane, exact_scrub_backend,
    exact_scrub_enabled, reset_scrub_telemetry, scrub_telemetry,
};
use shrimply_visual_cuda::compositor::{
    CompositeAccuracy, spawn_worker_with_resources_and_observer,
};
use shrimply_visual_cuda::gpu::CompositedVideoFrame;

const DEADLINE_US: u128 = 33_333;
const WORKER_TIMEOUT: Duration = Duration::from_secs(120);

fn wait_frame(
    events: &Receiver<VideoEvent<CompositedVideoFrame>>,
    expected_position: Option<Time>,
    label: &str,
) -> Result<CompositedVideoFrame, String> {
    loop {
        match events
            .recv_timeout(WORKER_TIMEOUT)
            .map_err(|e| format!("{label}: {e}"))?
        {
            VideoEvent::Frame {
                frame, position, ..
            } if expected_position.is_none_or(|want| position == want) => return Ok(frame),
            VideoEvent::Error(e) => return Err(e),
            _ => {}
        }
    }
}

fn stop_worker(commands: VideoCommandSender, events: Receiver<VideoEvent<CompositedVideoFrame>>) {
    let _ = commands.send(VideoCommand::Stop);
    drop(commands);
    while events.recv_timeout(Duration::from_secs(5)).is_ok() {}
}

struct VideoMeta {
    path: PathBuf,
    size: CanvasSize,
    fps: Fraction,
    duration: Time,
    total_frames: u64,
    keyframe_frames: Vec<u64>,
}

fn probe_video(path: &PathBuf) -> Result<VideoMeta, String> {
    ffmpeg::init().map_err(|e| e.to_string())?;
    let input = ffmpeg::format::input(path).map_err(|e| e.to_string())?;
    let stream = input
        .streams()
        .find(|s| {
            s.parameters().medium() == ffmpeg::media::Type::Video
                && !s
                    .disposition()
                    .contains(ffmpeg::format::stream::Disposition::ATTACHED_PIC)
        })
        .ok_or("input has no video stream")?;
    let rate = stream.avg_frame_rate();
    if rate.numerator() <= 0 || rate.denominator() <= 0 || input.duration() <= 0 {
        return Err("video requires known frame rate and duration".into());
    }
    let fps = fraction_new(i64::from(rate.numerator()), i64::from(rate.denominator()));
    let raw_duration = Time::from_fraction(input.duration(), i64::from(sys::AV_TIME_BASE));
    let total_frames = frame_index(raw_duration, fps).ok_or("frame count overflow")? as u64;
    let duration = time_from_frame(total_frames, fps).ok_or("aligned duration overflow")?;
    let params = stream.parameters();
    let raw = unsafe { &*params.as_ptr() };
    let size = CanvasSize {
        width: u32::try_from(raw.width).map_err(|_| "invalid width")?,
        height: u32::try_from(raw.height).map_err(|_| "invalid height")?,
    };
    let time_base = stream.time_base();
    let start_time = if stream.start_time() == sys::AV_NOPTS_VALUE {
        0
    } else {
        stream.start_time()
    };

    let mut keyframe_frames = Vec::new();
    let stream_ptr = unsafe { *(*input.as_ptr()).streams.add(stream.index()) };
    let idx_count = unsafe { sys::avformat_index_get_entries_count(stream_ptr) };
    for i in 0..idx_count {
        let entry = unsafe { sys::avformat_index_get_entry(stream_ptr, i) };
        if entry.is_null() {
            continue;
        }
        let e = unsafe { &*entry };
        if (e.flags() as u32 & sys::AVINDEX_KEYFRAME as u32) != 0
            && e.timestamp != sys::AV_NOPTS_VALUE
        {
            let ts = e.timestamp.saturating_sub(start_time).max(0);
            let t = Time::from_fraction(
                ts.saturating_mul(i64::from(time_base.numerator())),
                i64::from(time_base.denominator()),
            );
            if let Some(f) = frame_index(t, fps) {
                let f = f.max(0) as u64;
                if f < total_frames {
                    keyframe_frames.push(f);
                }
            }
        }
    }
    keyframe_frames.sort_unstable();
    keyframe_frames.dedup();

    Ok(VideoMeta {
        path: path.clone(),
        size,
        fps,
        duration,
        total_frames,
        keyframe_frames,
    })
}

fn build_project(meta: &VideoMeta) -> Result<Project, String> {
    let mut item = VideoItem::background_item(meta.size, Time::ZERO, meta.duration);
    item.content = VideoItemContent::Media;
    item.file = Asset::new(meta.path.clone());
    item.source_width = meta.size.width;
    item.source_height = meta.size.height;
    item.source_duration = meta.duration;
    item.playback_fps = meta.fps;
    item.sample_method = shrimply_property_model::timeline_value::TimelineValue::new_const(
        VideoSampleMethod::Bilinear,
    );

    let mut project = Project::default();
    project.canvas_size = meta.size;
    project.fps = meta.fps;
    project.video_tracks = vec![VisualTrack {
        items: vec![item],
        ..Default::default()
    }];
    project.ensure_ids();
    project.validate()?;
    Ok(project)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FrameSignature {
    pts: i64,
    width: i32,
    height: i32,
    color_range: i32,
    color_space: i32,
    color_primaries: i32,
    color_trc: i32,
    chroma_location: i32,
    pixel_hash: [u8; 16],
}

fn md5_nv12_frame(frame: &ffmpeg::frame::Video) -> Result<FrameSignature, String> {
    let raw = unsafe { &*frame.as_ptr() };
    let mut cpu_frame = ffmpeg::frame::Video::empty();
    let src = if !raw.hw_frames_ctx.is_null() {
        let status =
            unsafe { sys::av_hwframe_transfer_data(cpu_frame.as_mut_ptr(), frame.as_ptr(), 0) };
        if status < 0 {
            return Err(format!(
                "download hw frame: {}",
                ffmpeg::Error::from(status)
            ));
        }
        unsafe {
            sys::av_frame_copy_props(cpu_frame.as_mut_ptr(), frame.as_ptr());
        }
        unsafe { &*cpu_frame.as_ptr() }
    } else {
        raw
    };

    let w = src.width as usize;
    let h = src.height as usize;
    const FMT_NV12: i32 = sys::AVPixelFormat::AV_PIX_FMT_NV12 as i32;
    const FMT_YUV420P: i32 = sys::AVPixelFormat::AV_PIX_FMT_YUV420P as i32;
    const FMT_YUVJ420P: i32 = sys::AVPixelFormat::AV_PIX_FMT_YUVJ420P as i32;
    if !matches!(src.format, FMT_NV12 | FMT_YUV420P | FMT_YUVJ420P) {
        return Err(format!(
            "unsupported verification pixel format {}",
            src.format
        ));
    }

    let md5 = unsafe { sys::av_md5_alloc() };
    if md5.is_null() {
        return Err("av_md5_alloc failed".into());
    }
    unsafe { sys::av_md5_init(md5) };

    for y in 0..h {
        let row = unsafe { src.data[0].add(y * src.linesize[0] as usize) };
        unsafe { sys::av_md5_update(md5, row, w) };
    }
    if src.format == FMT_NV12 {
        for y in 0..h.div_ceil(2) {
            let row = unsafe { src.data[1].add(y * src.linesize[1] as usize) };
            unsafe { sys::av_md5_update(md5, row, w) };
        }
    } else {
        // Interleave U and V into NV12 order so CPU yuv420p matches NVDEC NV12 bit-for-bit.
        let cw = w.div_ceil(2);
        let mut uv_row = vec![0u8; cw * 2];
        for y in 0..h.div_ceil(2) {
            let u_ptr = unsafe { src.data[1].add(y * src.linesize[1] as usize) };
            let v_ptr = unsafe { src.data[2].add(y * src.linesize[2] as usize) };
            for x in 0..cw {
                uv_row[x * 2] = unsafe { *u_ptr.add(x) };
                uv_row[x * 2 + 1] = unsafe { *v_ptr.add(x) };
            }
            unsafe { sys::av_md5_update(md5, uv_row.as_ptr(), uv_row.len()) };
        }
    }

    let mut digest = [0u8; 16];
    unsafe {
        sys::av_md5_final(md5, digest.as_mut_ptr());
        sys::av_free(md5.cast());
    }

    Ok(FrameSignature {
        pts: raw.pts,
        width: raw.width,
        height: raw.height,
        color_range: raw.color_range as i32,
        color_space: raw.colorspace as i32,
        color_primaries: raw.color_primaries as i32,
        color_trc: raw.color_trc as i32,
        chroma_location: raw.chroma_location as i32,
        pixel_hash: digest,
    })
}

fn generate_workloads(meta: &VideoMeta) -> Vec<(&'static str, Vec<u64>, CompositeAccuracy)> {
    let mut rng: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next_u64 = || {
        rng = rng
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        rng
    };

    // 1. First-time random jumps spread across unique GOPs throughout the hour (128 requests).
    let mut used_gops = HashSet::new();
    let mut random_jumps = Vec::with_capacity(128);
    let max_frame = meta.total_frames.saturating_sub(2).max(1);
    while random_jumps.len() < 128 {
        let f = (next_u64() >> 16) % max_frame;
        let gop = f / 60;
        if used_gops.insert(gop) {
            random_jumps.push(f);
        }
    }

    // 2. GOP ends throughout the hour (64 unique GOPs not touched by random_jumps).
    let mut gop_ends = Vec::with_capacity(64);
    let mut gop_candidates: Vec<u64> = meta
        .keyframe_frames
        .iter()
        .skip(1)
        .filter_map(|&k| k.checked_sub(1))
        .filter(|&end_f| !used_gops.contains(&(end_f / 60)) && end_f < max_frame)
        .collect();
    if gop_candidates.is_empty() {
        for g in 1..(max_frame / 60) {
            let end_f = g * 60 - 1;
            if !used_gops.contains(&(end_f / 60)) {
                gop_candidates.push(end_f);
            }
        }
    }
    let stride = (gop_candidates.len() / 64).max(1);
    for &end_f in gop_candidates.iter().step_by(stride).take(64) {
        used_gops.insert(end_f / 60);
        gop_ends.push(end_f);
    }

    // 3. Forward sweep across a fresh GOP boundary (64 consecutive frames).
    let fwd_start = (meta.total_frames / 3).saturating_sub(30);
    let forward_sweep: Vec<u64> = (fwd_start..(fwd_start + 64).min(max_frame)).collect();

    // 4. Reverse sweep across a fresh GOP boundary (64 consecutive frames backward).
    let rev_start = ((meta.total_frames * 2) / 3).min(max_frame);
    let reverse_sweep: Vec<u64> = (0..64).map(|i| rev_start.saturating_sub(i)).collect();

    // 5. Repeated seeks revisiting 32 frames from earlier jumps (4 passes = 128 requests).
    let mut repeated = Vec::with_capacity(128);
    let base: Vec<u64> = random_jumps.iter().take(32).copied().collect();
    for _ in 0..4 {
        repeated.extend_from_slice(&base);
    }

    vec![
        ("random_jumps", random_jumps, CompositeAccuracy::BEST_EFFORT),
        ("gop_ends", gop_ends, CompositeAccuracy::BEST_EFFORT),
        (
            "forward_sweep",
            forward_sweep,
            CompositeAccuracy::LOCAL_TIME_ACCURATE,
        ),
        (
            "reverse_sweep",
            reverse_sweep,
            CompositeAccuracy::LOCAL_TIME_ACCURATE,
        ),
        ("repeated_seeks", repeated, CompositeAccuracy::BEST_EFFORT),
    ]
}

fn summarize(label: &str, samples_us: &mut [u128]) {
    if samples_us.is_empty() {
        println!("  {label}: count=0");
        return;
    }
    samples_us.sort_unstable();
    let n = samples_us.len();
    let p50 = samples_us[n / 2];
    let p95 = samples_us[(n * 95).div_ceil(100).saturating_sub(1)];
    let p99 = samples_us[(n * 99).div_ceil(100).saturating_sub(1)];
    let max = samples_us[n - 1];
    let misses = samples_us.iter().filter(|&&us| us > DEADLINE_US).count();
    println!(
        "  {label}: count={n} p50={:.2}ms p95={:.2}ms p99={:.2}ms max={:.2}ms deadline_misses(>33.33ms)={misses}",
        p50 as f64 / 1000.0,
        p95 as f64 / 1000.0,
        p99 as f64 / 1000.0,
        max as f64 / 1000.0,
    );
}

fn process_peak_rss_kib() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|text| {
            text.lines()
                .find(|l| l.starts_with("VmHWM:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|v| v.parse::<u64>().ok())
        })
        .unwrap_or(0)
}

fn run_bench(meta: &VideoMeta, force_accuracy: Option<CompositeAccuracy>) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    shrimply_visual_cuda::gpu::configure_gtk_backend()?;

    let project = build_project(meta)?;
    let workloads = generate_workloads(meta);
    let exact_flag = exact_scrub_enabled();
    let backend = exact_scrub_backend();

    println!(
        "=== COMPOSITOR BENCHMARK: file={} {}x{} fps={} frames={} exact_scrub={} backend={:?} force_accuracy={:?} ===",
        meta.path.display(),
        meta.size.width,
        meta.size.height,
        meta.fps,
        meta.total_frames,
        exact_flag,
        backend,
        force_accuracy
    );

    reset_scrub_telemetry();
    let startup_t0 = Instant::now();
    let (commands, events) =
        spawn_worker_with_resources_and_observer(project, Default::default(), None);

    // Startup & warmup request (settled preview frame 0) measured separately.
    commands
        .send(VideoCommand::Render {
            position: Time::ZERO,
            accuracy: force_accuracy.unwrap_or(CompositeAccuracy::TIME_ACCURATE),
        })
        .map_err(|e| e.to_string())?;
    let first_frame = wait_frame(&events, None, "startup frame 0")?;
    if let Ok(buf) = first_frame.buffer.cuda() {
        buf.context().synchronize().map_err(|e| e.to_string())?;
    }
    let startup_elapsed = startup_t0.elapsed();
    let startup_telem = scrub_telemetry();
    println!(
        "startup: first_frame={:.2}ms prep_wall={:.2}ms prep_pictures_reconstructed=0 index_bytes={} staging_bytes={}",
        startup_elapsed.as_secs_f64() * 1000.0,
        startup_telem.prep_wall_us as f64 / 1000.0,
        startup_telem.index_bytes,
        startup_telem.staging_bytes,
    );

    reset_scrub_telemetry();

    let mut all_overall = Vec::new();
    let mut all_hits = Vec::new();
    let mut all_misses = Vec::new();

    for (name, frames, default_acc) in &workloads {
        let accuracy = force_accuracy.unwrap_or(*default_acc);
        let mut overall = Vec::with_capacity(frames.len());
        let mut hits = Vec::new();
        let mut misses = Vec::new();

        let mut outliers: Vec<(u128, u64, u64, u64, u64, u64, u64)> = Vec::new();
        for &f in frames {
            let position = time_from_frame(f, meta.fps).ok_or("frame time overflow")?;
            let before = scrub_telemetry();
            let req_t0 = Instant::now();
            commands
                .send(VideoCommand::Render { position, accuracy })
                .map_err(|e| e.to_string())?;
            let frame = wait_frame(&events, Some(position), &format!("{name} frame {f}"))?;
            if let Ok(buf) = frame.buffer.cuda() {
                buf.context().synchronize().map_err(|e| e.to_string())?;
            }
            let elapsed_us = req_t0.elapsed().as_micros();
            let telem_after = scrub_telemetry();
            if elapsed_us > DEADLINE_US {
                outliers.push((
                    elapsed_us,
                    f,
                    telem_after
                        .pictures_decoded
                        .saturating_sub(before.pictures_decoded),
                    telem_after.io_us.saturating_sub(before.io_us),
                    telem_after.parse_us.saturating_sub(before.parse_us),
                    telem_after
                        .nvdec_submit_us
                        .saturating_sub(before.nvdec_submit_us),
                    telem_after.map_copy_us.saturating_sub(before.map_copy_us),
                ));
            }
            overall.push(elapsed_us);
            all_overall.push(elapsed_us);
            if telem_after.cache_hits > before.cache_hits {
                hits.push(elapsed_us);
                all_hits.push(elapsed_us);
            } else if telem_after.cache_misses > before.cache_misses {
                misses.push(elapsed_us);
                all_misses.push(elapsed_us);
            } else if exact_scrub_enabled() {
                // VideoElement single-frame current hit inside exact scrub mode.
                hits.push(elapsed_us);
                all_hits.push(elapsed_us);
            }
        }

        println!("workload={name} accuracy={accuracy:?}:");
        summarize("overall", &mut overall);
        summarize("cache_hits", &mut hits);
        summarize("cache_misses", &mut misses);
        if !outliers.is_empty() {
            outliers.sort_unstable_by(|a, b| b.0.cmp(&a.0));
            for &(tot, f, pics, io, parse, dec, map) in outliers.iter().take(5) {
                let other = (tot as i64 - (io + parse + dec + map) as i64).max(0);
                println!(
                    "    outlier frame={f}: total={:.2}ms pics={pics} io={:.2}ms parse={:.2}ms nvdec_submit={:.2}ms map_sync={:.2}ms comp_other={:.2}ms",
                    tot as f64 / 1000.0,
                    io as f64 / 1000.0,
                    parse as f64 / 1000.0,
                    dec as f64 / 1000.0,
                    map as f64 / 1000.0,
                    other as f64 / 1000.0,
                );
            }
        }
    }

    let telem = scrub_telemetry();
    println!("=== SUMMARY ACROSS ALL WORKLOADS ===");
    summarize("ALL overall", &mut all_overall);
    summarize("ALL cache_hits", &mut all_hits);
    summarize("ALL cache_misses", &mut all_misses);
    println!(
        "cache_accounting: vram_frames={} B ram_packets={} B index={} B staging={} B total={} B peak={} B budget={} B (<=1GB: {})",
        telem.vram_frames_bytes,
        telem.ram_packets_bytes,
        telem.index_bytes,
        telem.staging_bytes,
        telem.total_bytes,
        telem.peak_total_bytes,
        telem.budget_bytes,
        telem.peak_total_bytes <= shrimply_video_decoder::MAX_SCRUB_CACHE_BYTES,
    );
    println!(
        "decode_stats: cached_frames={} cached_gops={} hits={} misses={} live_surface_hits={} live_dpb_hits={} forward_cont={} nvdec_decodes={} ffmpeg_fallbacks={} pictures_decoded={} process_peak_rss_kib={}",
        telem.cached_frames,
        telem.cached_gops,
        telem.cache_hits,
        telem.cache_misses,
        telem.live_surface_hits,
        telem.live_dpb_hits,
        telem.forward_continuations,
        telem.nvdec_decodes,
        telem.ffmpeg_fallbacks,
        telem.pictures_decoded,
        process_peak_rss_kib(),
    );

    stop_worker(commands, events);
    Ok(())
}

fn run_verify(meta: &VideoMeta, max_sample_seconds: u64) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    shrimply_visual_cuda::gpu::configure_gtk_backend()?;

    let project = build_project(meta)?;
    let item = &project.video_tracks[0].items[0];
    let limit_frame = frame_index(Time::from_seconds(max_sample_seconds as i64), meta.fps)
        .unwrap_or(meta.total_frames as i64)
        .min(meta.total_frames as i64 - 1) as u64;

    // Build a comprehensive target list spanning random jumps, GOP ends, forward & reverse sweeps, and edges.
    let mut target_set = BTreeSet::<u64>::new();
    target_set.insert(0);
    target_set.insert(1);
    target_set.insert(limit_frame);
    target_set.insert(limit_frame.saturating_sub(1));

    let mut rng: u64 = 0x517C_C1B7_2722_0A95;
    for _ in 0..96 {
        rng = rng.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        let f = (rng >> 16) % limit_frame.max(1);
        target_set.insert(f);
    }
    for &k in &meta.keyframe_frames {
        if k > 0 && k - 1 <= limit_frame {
            target_set.insert(k - 1);
            if k <= limit_frame {
                target_set.insert(k);
            }
        }
        if target_set.len() >= 180 {
            break;
        }
    }
    let sweep_base = (limit_frame / 2).saturating_sub(16);
    for f in sweep_base..(sweep_base + 32).min(limit_frame) {
        target_set.insert(f);
    }

    // Stream sequentially with an INDEPENDENT CPU FFmpeg decoder without retaining frames in RAM.
    println!(
        "=== VERIFICATION: streaming independent sequential FFmpeg decoder up to frame {limit_frame} ({} target frames) ===",
        target_set.len()
    );
    let mut seq_input = ffmpeg::format::input(&meta.path).map_err(|e| e.to_string())?;
    let stream_idx = seq_input
        .streams()
        .best(ffmpeg::media::Type::Video)
        .ok_or("no video stream")?
        .index();
    let seq_params = seq_input.stream(stream_idx).unwrap().parameters();
    let mut seq_dec = ffmpeg::codec::Context::from_parameters(seq_params)
        .map_err(|e| e.to_string())?
        .decoder()
        .video()
        .map_err(|e| e.to_string())?;

    let mut reference_sigs: BTreeMap<u64, FrameSignature> = BTreeMap::new();
    let mut decoded_frame = ffmpeg::frame::Video::empty();
    let mut frame_ordinal: u64 = 0;

    let mut drain_reference = |dec: &mut ffmpeg::decoder::Video| -> Result<bool, String> {
        while dec.receive_frame(&mut decoded_frame).is_ok() {
            if target_set.contains(&frame_ordinal) {
                reference_sigs.insert(frame_ordinal, md5_nv12_frame(&decoded_frame)?);
            }
            frame_ordinal += 1;
            if frame_ordinal > limit_frame {
                return Ok(true);
            }
        }
        Ok(false)
    };

    for (stream, packet) in seq_input.packets() {
        if stream.index() != stream_idx {
            continue;
        }
        seq_dec.send_packet(&packet).map_err(|e| e.to_string())?;
        if drain_reference(&mut seq_dec)? {
            break;
        }
    }
    if frame_ordinal <= limit_frame {
        let _ = seq_dec.send_eof();
        let _ = drain_reference(&mut seq_dec)?;
    }

    // Now test our experimental ExactScrub decoder on out-of-order jumps, reverse sweep, forward sweep, and repeats!
    let mut test_order: Vec<u64> = reference_sigs.keys().copied().collect();
    // Deterministic Fisher-Yates shuffle for true arbitrary random-access stress + append reverse sweep.
    for i in (1..test_order.len()).rev() {
        rng = rng.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        let j = (rng as usize) % (i + 1);
        test_order.swap(i, j);
    }
    for f in (sweep_base..(sweep_base + 32).min(limit_frame)).rev() {
        if reference_sigs.contains_key(&f) {
            test_order.push(f);
        }
    }
    for f in sweep_base..(sweep_base + 32).min(limit_frame) {
        if reference_sigs.contains_key(&f) {
            test_order.push(f);
        }
    }

    reset_scrub_telemetry();
    let mut pool = VideoDecoderPool::new(1);
    let owner = pool.owner(&[], uuid::Uuid::new_v4(), item.id, VideoPlane::Color);
    let decoder = pool.decoder(item, owner)?;

    let mut pts_mismatches = 0usize;
    let mut pixel_mismatches = 0usize;
    let mut color_mismatches = 0usize;
    let mut verified = 0usize;

    for &f in &test_order {
        let position = time_from_frame(f, meta.fps).ok_or("frame time overflow")?;
        let DecodeOutcome::Frame(Some((_, video_frame))) = decoder
            .request(DecodeRequest::new(position, DecodeMode::ExactScrub))?
            .receive()?
        else {
            return Err(format!("ExactScrub returned no frame for frame {f}"));
        };
        let got = md5_nv12_frame(video_frame.native())?;
        let expected = &reference_sigs[&f];
        verified += 1;
        if got.pts != expected.pts {
            if pts_mismatches < 5 {
                eprintln!(
                    "PTS MISMATCH at frame {f}: expected={} got={}",
                    expected.pts, got.pts
                );
            }
            pts_mismatches += 1;
        }
        if got.pixel_hash != expected.pixel_hash {
            if pixel_mismatches < 5 {
                eprintln!(
                    "PIXEL MISMATCH at frame {f}: expected={:02x?} got={:02x?}",
                    expected.pixel_hash, got.pixel_hash
                );
            }
            pixel_mismatches += 1;
        }
        if (
            got.width,
            got.height,
            got.color_range,
            got.color_space,
            got.color_primaries,
            got.color_trc,
            got.chroma_location,
        ) != (
            expected.width,
            expected.height,
            expected.color_range,
            expected.color_space,
            expected.color_primaries,
            expected.color_trc,
            expected.chroma_location,
        ) {
            if color_mismatches < 5 {
                eprintln!(
                    "COLOR MISMATCH at frame {f}: expected=({}x{}, range={}, spc={}, pri={}, trc={}, loc={}) got=({}x{}, range={}, spc={}, pri={}, trc={}, loc={})",
                    expected.width,
                    expected.height,
                    expected.color_range,
                    expected.color_space,
                    expected.color_primaries,
                    expected.color_trc,
                    expected.chroma_location,
                    got.width,
                    got.height,
                    got.color_range,
                    got.color_space,
                    got.color_primaries,
                    got.color_trc,
                    got.chroma_location,
                );
            }
            color_mismatches += 1;
        }
    }

    drop(decoder);
    pool.retain(std::iter::empty());
    let _ = pool.reclaim_idle().recv();

    // Also verify end-to-end composited preview RGBA pixels on 16 sampled frames against a
    // separate baseline preview worker running unmodified DecodeMode::Accurate (CONTENT_ACCURATE)!
    let (ref_commands, ref_events) =
        spawn_worker_with_resources_and_observer(project.clone(), Default::default(), None);
    let (commands, events) =
        spawn_worker_with_resources_and_observer(project.clone(), Default::default(), None);
    let mut comp_mismatches = 0usize;
    let comp_samples: Vec<u64> = reference_sigs
        .keys()
        .step_by((reference_sigs.len() / 16).max(1))
        .take(16)
        .copied()
        .collect();
    for &f in &comp_samples {
        let position = time_from_frame(f, meta.fps).ok_or("frame time overflow")?;
        ref_commands
            .send(VideoCommand::Render {
                position,
                accuracy: CompositeAccuracy::FULLY_ACCURATE,
            })
            .map_err(|e| e.to_string())?;
        let expected_rgba =
            wait_frame(&ref_events, Some(position), &format!("ref frame {f}"))?.rgba_bytes()?;
        commands
            .send(VideoCommand::Render {
                position,
                accuracy: CompositeAccuracy::BEST_EFFORT,
            })
            .map_err(|e| e.to_string())?;
        let got_rgba =
            wait_frame(&events, Some(position), &format!("scrub frame {f}"))?.rgba_bytes()?;
        if got_rgba != expected_rgba {
            let mut diff_count = 0usize;
            let mut max_diff = 0u8;
            let mut first = None;
            for (idx, (&a, &b)) in expected_rgba.iter().zip(got_rgba.iter()).enumerate() {
                if a != b {
                    diff_count += 1;
                    let d = a.abs_diff(b);
                    if d > max_diff {
                        max_diff = d;
                    }
                    if first.is_none() {
                        first = Some((idx, a, b));
                    }
                }
            }
            eprintln!(
                "COMPOSITOR RGBA MISMATCH at frame {f}: len=({},{}) diffs={diff_count} max_diff={max_diff} first={first:?}",
                expected_rgba.len(),
                got_rgba.len(),
            );
            comp_mismatches += 1;
        }
    }
    stop_worker(ref_commands, ref_events);
    stop_worker(commands, events);

    let telem = scrub_telemetry();
    println!(
        "VERIFICATION RESULT: checked={verified} unique={} pts_mismatches={pts_mismatches} pixel_mismatches={pixel_mismatches} color_mismatches={color_mismatches} compositor_rgba_mismatches={comp_mismatches}/{}",
        reference_sigs.len(),
        comp_samples.len(),
    );
    println!(
        "verify_telemetry: nvdec_decodes={} ffmpeg_fallbacks={} live_surface_hits={} live_dpb_hits={} forward_cont={} hits={} misses={} peak_cache_bytes={}",
        telem.nvdec_decodes,
        telem.ffmpeg_fallbacks,
        telem.live_surface_hits,
        telem.live_dpb_hits,
        telem.forward_continuations,
        telem.cache_hits,
        telem.cache_misses,
        telem.peak_total_bytes,
    );
    if pts_mismatches > 0 || pixel_mismatches > 0 || color_mismatches > 0 || comp_mismatches > 0 {
        return Err("Verification failed with mismatches".into());
    }
    Ok(())
}

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let mode = args.next().ok_or(
        "usage: scrub_benchmark <verify|bench|bench-exact-baseline> VIDEO [VERIFY_SECONDS]",
    )?;
    let path = PathBuf::from(args.next().ok_or("missing VIDEO path")?)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let verify_seconds = args
        .next()
        .map(|v| v.parse::<u64>().map_err(|e| e.to_string()))
        .transpose()?
        .unwrap_or(180);

    let meta = probe_video(&path)?;
    match mode.as_str() {
        "verify" => run_verify(&meta, verify_seconds),
        "bench" => run_bench(&meta, None),
        "bench-exact-baseline" => run_bench(&meta, Some(CompositeAccuracy::TIME_ACCURATE)),
        other => Err(format!("unknown mode: {other}")),
    }
}
