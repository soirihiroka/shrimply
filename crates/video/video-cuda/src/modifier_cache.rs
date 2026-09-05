use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::thread;

use ffmpeg_next as ffmpeg;
use ffmpeg_next::format::Pixel;
use libc::EAGAIN;
use serde::{Deserialize, Serialize};
use shrimply_math_core::Fraction;
use shrimply_project::project::{
    CanvasSize, ItemAddress, Project, RepeatStrategy, Time, Transform, VideoItem, VideoItemContent,
    default_playback_speed, fraction_denominator, fraction_numerator,
};
use shrimply_resource_pipeline::{
    Event, JobContext, Pipeline, Processor, RequestDisposition, Subscription, TryNext,
};
use shrimply_video_modifiers::{ModifierEffect, RasterModifierEffect, cache::CacheModifier};
use uuid::Uuid;

use crate::compositor::{EXPORT_ASSETS_LOADING, VideoExportRenderer};

const MANIFEST_NAME: &str = "manifest.json";
const MEDIA_NAME: &str = "visual.mkv";
const CACHE_VERSION: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Status {
    Missing,
    Baking { completed: u64, total: u64 },
    Ready,
    Failed(String),
}

struct Job {
    status: Status,
    subscription: Option<Subscription<Uuid, Progress, ()>>,
}

#[derive(Clone, Copy)]
struct Progress {
    completed: u64,
    total: u64,
}

struct BakeInput {
    project: Project,
    address: ItemAddress,
    modifier_id: Uuid,
    start: Time,
    duration: Time,
    settings: CacheModifier,
}

struct BakeProcessor {
    inputs: Arc<Mutex<HashMap<Uuid, BakeInput>>>,
}

struct Runtime {
    pipeline: Pipeline<Uuid, BakeProcessor>,
    inputs: Arc<Mutex<HashMap<Uuid, BakeInput>>>,
}

#[derive(Serialize, Deserialize)]
struct Manifest {
    version: u32,
    kind: String,
    width: u32,
    height: u32,
    coded_width: u32,
    coded_height: u32,
    duration: Time,
    fps_numerator: i64,
    fps_denominator: i64,
}

#[derive(Clone)]
struct ReadyEntry {
    path: PathBuf,
    width: u32,
    height: u32,
    duration: Time,
    fps: Fraction,
}

static JOBS: LazyLock<Mutex<HashMap<Uuid, Job>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
static READY: LazyLock<Mutex<HashMap<Uuid, ReadyEntry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static CACHE_OPERATIONS: Mutex<()> = Mutex::new(());
static RUNTIME: LazyLock<Runtime> = LazyLock::new(Runtime::new);

impl Job {
    fn refresh(&mut self) -> bool {
        let Some(subscription) = self.subscription.as_mut() else {
            return false;
        };
        let terminal = loop {
            match subscription.try_next() {
                TryNext::Event(Event::Progress(progress)) => {
                    self.status = Status::Baking {
                        completed: progress.completed,
                        total: progress.total,
                    };
                }
                TryNext::Event(Event::Finished(_)) => break Some(Status::Ready),
                TryNext::Event(Event::Failed(error)) => {
                    break Some(Status::Failed(error.to_string()));
                }
                TryNext::Event(Event::Cancelled) => break Some(Status::Missing),
                TryNext::Empty => break None,
                TryNext::Closed => {
                    break Some(Status::Failed(
                        "visual cache job closed without a terminal event".to_string(),
                    ));
                }
            }
        };
        let Some(status) = terminal else {
            return false;
        };
        self.status = status;
        self.subscription = None;
        true
    }
}

impl Runtime {
    fn new() -> Self {
        let inputs = Arc::new(Mutex::new(HashMap::new()));
        Self {
            pipeline: Pipeline::new(
                BakeProcessor {
                    inputs: inputs.clone(),
                },
                |job| {
                    let _ = thread::spawn(job);
                },
            ),
            inputs,
        }
    }

    fn request(&self, input: BakeInput) -> (RequestDisposition, Subscription<Uuid, Progress, ()>) {
        let modifier_id = input.modifier_id;
        let mut inputs = self
            .inputs
            .lock()
            .expect("visual cache input lock poisoned");
        assert!(
            !inputs.contains_key(&modifier_id),
            "visual cache input already exists"
        );
        inputs.insert(modifier_id, input);
        drop(inputs);
        let request = self.pipeline.request(modifier_id);
        if request.0 == RequestDisposition::Joined {
            self.discard_input(modifier_id);
        }
        request
    }

    fn cancel(&self, modifier_id: Uuid) {
        self.pipeline.cancel(&modifier_id);
        self.discard_input(modifier_id);
    }

    fn discard_input(&self, modifier_id: Uuid) {
        self.inputs
            .lock()
            .expect("visual cache input lock poisoned")
            .remove(&modifier_id);
    }
}

impl Processor<Uuid> for BakeProcessor {
    type Progress = Progress;
    type Output = ();

    fn process(
        &self,
        modifier_id: Uuid,
        context: &JobContext<Self::Progress>,
    ) -> Result<Self::Output, String> {
        let input = self
            .inputs
            .lock()
            .expect("visual cache input lock poisoned")
            .remove(&modifier_id)
            .ok_or_else(|| "visual cache bake input disappeared".to_string())?;
        bake_inner(
            input.project,
            input.address,
            modifier_id,
            input.start,
            input.duration,
            &input.settings,
            context,
        )
    }
}

pub fn status(modifier_id: Uuid) -> Status {
    let job = {
        let mut jobs = JOBS
            .lock()
            .expect("visual modifier cache job lock poisoned");
        jobs.get_mut(&modifier_id).map(|job| {
            let terminal = job.refresh();
            (job.status.clone(), terminal)
        })
    };
    if let Some((status, terminal)) = job {
        if terminal {
            RUNTIME.discard_input(modifier_id);
        }
        return status;
    }
    match ready_entry(modifier_id) {
        Ok(_) => Status::Ready,
        Err(error) if cache_directory(modifier_id).exists() => Status::Failed(error),
        Err(_) => Status::Missing,
    }
}

pub fn bake(project: Project, address: ItemAddress, modifier_id: Uuid) -> Result<(), String> {
    let _operation = CACHE_OPERATIONS
        .lock()
        .expect("visual cache operation lock poisoned");
    if matches!(status(modifier_id), Status::Baking { .. }) {
        return Err("this cache is already baking".to_string());
    }
    let (project, address, start, duration, settings) =
        bake_project(project, &address, modifier_id)?;
    let (first_frame, end_frame) = frame_range(start, duration, project.fps)?;
    let total = end_frame - first_frame;
    invalidate_inner(modifier_id)?;
    let (disposition, subscription) = RUNTIME.request(BakeInput {
        project,
        address,
        modifier_id,
        start,
        duration,
        settings,
    });
    if disposition == RequestDisposition::Joined {
        subscription.cancel();
        return Err("this cache is already baking".to_string());
    }
    JOBS.lock()
        .expect("visual modifier cache job lock poisoned")
        .insert(
            modifier_id,
            Job {
                status: Status::Baking {
                    completed: 0,
                    total,
                },
                subscription: Some(subscription),
            },
        );
    Ok(())
}

pub fn invalidate(modifier_id: Uuid) -> Result<(), String> {
    let _operation = CACHE_OPERATIONS
        .lock()
        .expect("visual cache operation lock poisoned");
    invalidate_inner(modifier_id)
}

fn invalidate_inner(modifier_id: Uuid) -> Result<(), String> {
    RUNTIME.cancel(modifier_id);
    JOBS.lock()
        .expect("visual modifier cache job lock poisoned")
        .remove(&modifier_id);
    READY
        .lock()
        .expect("visual modifier ready cache lock poisoned")
        .remove(&modifier_id);
    match fs::remove_dir_all(cache_directory(modifier_id)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("could not invalidate visual cache: {error}")),
    }
}

pub(crate) fn effective_item(
    item: &VideoItem,
    canvas: CanvasSize,
) -> Result<Option<VideoItem>, String> {
    let mut ready = None;
    for (index, modifier) in item.modifiers.iter().enumerate().rev() {
        if !modifier.enabled
            || !matches!(
                modifier.effect,
                ModifierEffect::Raster(ref effect)
                    if matches!(&**effect, RasterModifierEffect::Cache(_))
            )
            || !cache_directory(modifier.id).exists()
        {
            continue;
        }
        ready = Some((index, ready_entry(modifier.id)?));
        break;
    }
    let Some((index, entry)) = ready else {
        return Ok(None);
    };
    let mut effective = item.clone();
    effective.file = entry.path.into();
    effective.content = VideoItemContent::Media;
    effective.track_id = 0;
    effective.alpha_mask_video = Some(1);
    effective.source_width = entry.width;
    effective.source_height = entry.height;
    effective.source_duration = entry.duration;
    effective.time_offset = Time::ZERO;
    effective.playback_speed = default_playback_speed();
    effective.playback_fps = entry.fps;
    effective.repeat_strategy = RepeatStrategy::Empty;
    effective.transform = Transform::fill(canvas);
    effective.default_transform = None;
    effective.motion_blur.enabled = false;
    effective.render_canvas_size = Some(canvas);
    effective.modifiers = item.modifiers[index + 1..].to_vec();
    Ok(Some(effective))
}

fn bake_project(
    mut project: Project,
    address: &ItemAddress,
    modifier_id: Uuid,
) -> Result<(Project, ItemAddress, Time, Time, CacheModifier), String> {
    let (start, end) = project
        .projected_item_times(address)
        .ok_or_else(|| "visual cache item is outside its folded-sequence hosts".to_string())?;
    let item = project
        .video_item(address)
        .ok_or_else(|| "visual cache item no longer exists".to_string())?;
    let index = item
        .modifiers
        .iter()
        .position(|modifier| modifier.id == modifier_id)
        .ok_or_else(|| "visual cache modifier no longer exists".to_string())?;
    let ModifierEffect::Raster(effect) = &item.modifiers[index].effect else {
        return Err("selected visual modifier is not a cache".to_string());
    };
    let RasterModifierEffect::Cache(settings) = &**effect else {
        return Err("selected visual modifier is not a cache".to_string());
    };
    let settings = settings.clone();
    let duration = end.saturating_sub(start);
    if duration == Time::ZERO {
        return Err("cannot cache an empty visual item".to_string());
    }
    let item_id = item.id;
    let shrimply_project::project::TrackMut::Video(track) = project
        .track_mut(&address.track())
        .ok_or_else(|| "visual cache track no longer exists".to_string())?
    else {
        return Err("visual cache requires a video track".to_string());
    };
    for item in &mut track.items {
        if item
            .transitions
            .to_next
            .as_ref()
            .is_some_and(|transition| transition.target_item_id == item_id)
        {
            item.transitions.to_next = None;
        }
    }
    let item = project
        .video_item_mut(address)
        .expect("visual cache item disappeared from cloned project");
    item.modifiers.truncate(index);
    Ok((project, address.clone(), start, duration, settings))
}

fn bake_inner(
    project: Project,
    address: ItemAddress,
    modifier_id: Uuid,
    start: Time,
    duration: Time,
    settings: &CacheModifier,
    context: &JobContext<Progress>,
) -> Result<(), String> {
    if context.is_cancelled() {
        return Err("visual cache bake cancelled".to_string());
    }
    let root = cache_root();
    fs::create_dir_all(&root).map_err(|error| format!("could not create cache folder: {error}"))?;
    let temporary = root.join(format!(
        ".{}-{}",
        modifier_id.simple(),
        Uuid::new_v4().simple()
    ));
    fs::create_dir(&temporary)
        .map_err(|error| format!("could not create temporary cache folder: {error}"))?;
    let result = (|| {
        let width = project.canvas_size.width.max(1);
        let height = project.canvas_size.height.max(1);
        let coded_width = even(width);
        let coded_height = even(height);
        let mut encoder = VideoCacheEncoder::new(
            &temporary.join(MEDIA_NAME),
            coded_width,
            coded_height,
            project.fps,
            settings.quality.qp(),
        )?;
        let mut renderer = VideoExportRenderer::new(48_000)?;
        let (first_frame, end_frame) = frame_range(start, duration, project.fps)?;
        let total = end_frame - first_frame;
        for frame_index in 0..total {
            if context.is_cancelled() {
                return Err("visual cache bake cancelled".to_string());
            }
            let position = shrimply_math_core::time_from_frame(
                first_frame.saturating_add(frame_index),
                project.fps,
            )
            .ok_or("cache frame rate must be positive")?;
            let composited = loop {
                match renderer.render_cache_item(&project, position, &address) {
                    Ok(frame) => break frame,
                    Err(error) if error == EXPORT_ASSETS_LOADING => {
                        if context.is_cancelled() {
                            return Err("visual cache bake cancelled".to_string());
                        }
                        thread::yield_now();
                    }
                    Err(error) => return Err(error),
                }
            };
            let mut rgba = ffmpeg::frame::Video::new(Pixel::RGBA, width, height);
            renderer.copy_to_rgba_frame(composited, &mut rgba)?;
            encoder.write(&rgba, frame_index)?;
            if !context.report(Progress {
                completed: frame_index + 1,
                total,
            }) {
                return Err("visual cache bake cancelled".to_string());
            }
        }
        encoder.finish()?;
        let manifest = Manifest {
            version: CACHE_VERSION,
            kind: "visual".to_string(),
            width,
            height,
            coded_width,
            coded_height,
            duration,
            fps_numerator: fraction_numerator(project.fps),
            fps_denominator: fraction_denominator(project.fps),
        };
        fs::write(
            temporary.join(MANIFEST_NAME),
            serde_json::to_vec(&manifest)
                .map_err(|error| format!("could not encode visual cache manifest: {error}"))?,
        )
        .map_err(|error| format!("could not write visual cache manifest: {error}"))?;
        let destination = cache_directory(modifier_id);
        let _operation = CACHE_OPERATIONS
            .lock()
            .expect("visual cache operation lock poisoned");
        if context.is_cancelled() {
            return Err("visual cache bake cancelled".to_string());
        }
        fs::rename(&temporary, &destination)
            .map_err(|error| format!("could not finish visual cache: {error}"))?;
        READY
            .lock()
            .expect("visual modifier ready cache lock poisoned")
            .remove(&modifier_id);
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&temporary);
    }
    result
}

fn ready_entry(modifier_id: Uuid) -> Result<ReadyEntry, String> {
    if let Some(entry) = READY
        .lock()
        .expect("visual modifier ready cache lock poisoned")
        .get(&modifier_id)
        .cloned()
    {
        return Ok(entry);
    }
    let directory = cache_directory(modifier_id);
    let bytes = fs::read(directory.join(MANIFEST_NAME))
        .map_err(|error| format!("visual cache is missing its manifest: {error}"))?;
    let manifest: Manifest = serde_json::from_slice(&bytes)
        .map_err(|error| format!("visual cache manifest is invalid: {error}"))?;
    if manifest.version != CACHE_VERSION || manifest.kind != "visual" {
        return Err("visual cache version is unsupported; invalidate and rebake it".to_string());
    }
    if manifest.coded_width != even(manifest.width)
        || manifest.coded_height != even(manifest.height)
        || manifest.fps_numerator <= 0
        || manifest.fps_denominator <= 0
    {
        return Err("visual cache manifest has an unsupported layout".to_string());
    }
    let path = directory.join(MEDIA_NAME);
    if !path.is_file() {
        return Err("visual cache media is missing".to_string());
    }
    let entry = ReadyEntry {
        path,
        width: manifest.width,
        height: manifest.height,
        duration: manifest.duration,
        fps: shrimply_project::project::fraction_new(
            manifest.fps_numerator,
            manifest.fps_denominator,
        ),
    };
    READY
        .lock()
        .expect("visual modifier ready cache lock poisoned")
        .insert(modifier_id, entry.clone());
    Ok(entry)
}

fn cache_directory(modifier_id: Uuid) -> PathBuf {
    cache_root().join(modifier_id.simple().to_string())
}

fn cache_root() -> PathBuf {
    let directory = shrimply_project::project::project_directory();
    let root = if directory
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == "shrimp")
    {
        directory
            .parent()
            .expect("shrimp project directory must have a parent")
    } else {
        &directory
    };
    root.join("media/.cache")
}

const fn even(value: u32) -> u32 {
    value.saturating_add(value % 2)
}

fn frame_range(start: Time, duration: Time, fps: Fraction) -> Result<(u64, u64), String> {
    let first =
        shrimply_math_core::frame_count(start, fps).ok_or("cache frame rate must be positive")?;
    let end = shrimply_math_core::frame_count(start.saturating_add(duration), fps)
        .ok_or("cache frame rate must be positive")?;
    if first >= end {
        return Err("cannot cache an item shorter than one project frame".to_string());
    }
    Ok((first, end))
}

struct VideoCacheEncoder {
    output: ffmpeg::format::context::Output,
    color: ffmpeg::codec::encoder::video::Encoder,
    alpha: ffmpeg::codec::encoder::video::Encoder,
    color_stream: usize,
    alpha_stream: usize,
    color_time_base: ffmpeg::Rational,
    alpha_time_base: ffmpeg::Rational,
    scaler: ffmpeg::software::scaling::context::Context,
    width: u32,
    height: u32,
}

impl VideoCacheEncoder {
    fn new(
        path: &Path,
        width: u32,
        height: u32,
        fps: Fraction,
        color_qp: u32,
    ) -> Result<Self, String> {
        ffmpeg::init().map_err(|error| format!("could not initialize FFmpeg: {error}"))?;
        let time_base = ffmpeg::Rational(
            i32::try_from(fraction_denominator(fps))
                .map_err(|_| "cache FPS denominator is too large".to_string())?,
            i32::try_from(fraction_numerator(fps))
                .map_err(|_| "cache FPS numerator is too large".to_string())?,
        );
        let frame_rate = ffmpeg::Rational(time_base.1, time_base.0);
        let mut output = ffmpeg::format::output(path)
            .map_err(|error| format!("could not create visual cache media: {error}"))?;
        let global_header = output
            .format()
            .flags()
            .contains(ffmpeg::format::Flags::GLOBAL_HEADER);
        let color = open_encoder(
            width,
            height,
            time_base,
            frame_rate,
            color_qp,
            global_header,
        )?;
        let alpha = open_encoder(width, height, time_base, frame_rate, 0, global_header)?;
        let color_stream = {
            let mut stream = output
                .add_stream_with(color.as_ref())
                .map_err(|error| format!("could not add visual cache color stream: {error}"))?;
            stream.set_time_base(time_base);
            stream.index()
        };
        let alpha_stream = {
            let mut stream = output
                .add_stream_with(alpha.as_ref())
                .map_err(|error| format!("could not add visual cache alpha stream: {error}"))?;
            stream.set_time_base(time_base);
            stream.index()
        };
        output
            .write_header()
            .map_err(|error| format!("could not write visual cache header: {error}"))?;
        let color_time_base = output
            .stream(color_stream)
            .expect("visual color stream disappeared")
            .time_base();
        let alpha_time_base = output
            .stream(alpha_stream)
            .expect("visual alpha stream disappeared")
            .time_base();
        let scaler = ffmpeg::software::scaling::context::Context::get(
            Pixel::RGBA,
            width,
            height,
            Pixel::NV12,
            width,
            height,
            ffmpeg::software::scaling::flag::Flags::BILINEAR,
        )
        .map_err(|error| format!("could not create visual cache color converter: {error}"))?;
        Ok(Self {
            output,
            color,
            alpha,
            color_stream,
            alpha_stream,
            color_time_base,
            alpha_time_base,
            scaler,
            width,
            height,
        })
    }

    fn write(&mut self, source: &ffmpeg::frame::Video, frame_index: u64) -> Result<(), String> {
        let mut rgba = ffmpeg::frame::Video::new(Pixel::RGBA, self.width, self.height);
        rgba.data_mut(0).fill(0);
        let source_stride = source.stride(0);
        let destination_stride = rgba.stride(0);
        let source_width = source.width() as usize;
        for row in 0..source.height() as usize {
            let source_row = &source.data(0)[row * source_stride..][..source_width * 4];
            let destination_row =
                &mut rgba.data_mut(0)[row * destination_stride..][..source_width * 4];
            for (source, destination) in source_row
                .chunks_exact(4)
                .zip(destination_row.chunks_exact_mut(4))
            {
                let alpha = u16::from(source[3]);
                destination[3] = source[3];
                destination[..3].fill(0);
                if alpha != 0 {
                    for channel in 0..3 {
                        destination[channel] = (u16::from(source[channel]) * 255 + alpha / 2)
                            .checked_div(alpha)
                            .expect("nonzero alpha must divide")
                            .min(255) as u8;
                    }
                }
            }
        }
        let mut color = ffmpeg::frame::Video::new(Pixel::NV12, self.width, self.height);
        self.scaler
            .run(&rgba, &mut color)
            .map_err(|error| format!("could not convert visual cache color frame: {error}"))?;
        let mut alpha = ffmpeg::frame::Video::new(Pixel::NV12, self.width, self.height);
        alpha.data_mut(0).fill(0);
        alpha.data_mut(1).fill(128);
        let rgba_stride = rgba.stride(0);
        let alpha_stride = alpha.stride(0);
        for row in 0..self.height as usize {
            for column in 0..self.width as usize {
                alpha.data_mut(0)[row * alpha_stride + column] =
                    rgba.data(0)[row * rgba_stride + column * 4 + 3];
            }
        }
        let pts = i64::try_from(frame_index).map_err(|_| "cache is too long".to_string())?;
        color.set_pts(Some(pts));
        color.set_kind(ffmpeg::util::picture::Type::I);
        alpha.set_pts(Some(pts));
        alpha.set_kind(ffmpeg::util::picture::Type::I);
        self.color
            .send_frame(&color)
            .map_err(|error| format!("could not encode visual cache color: {error}"))?;
        receive_packets(
            &mut self.color,
            &mut self.output,
            self.color_stream,
            self.color_time_base,
        )?;
        self.alpha
            .send_frame(&alpha)
            .map_err(|error| format!("could not encode visual cache alpha: {error}"))?;
        receive_packets(
            &mut self.alpha,
            &mut self.output,
            self.alpha_stream,
            self.alpha_time_base,
        )
    }

    fn finish(mut self) -> Result<(), String> {
        self.color
            .send_eof()
            .map_err(|error| format!("could not finalize visual cache color: {error}"))?;
        receive_packets(
            &mut self.color,
            &mut self.output,
            self.color_stream,
            self.color_time_base,
        )?;
        self.alpha
            .send_eof()
            .map_err(|error| format!("could not finalize visual cache alpha: {error}"))?;
        receive_packets(
            &mut self.alpha,
            &mut self.output,
            self.alpha_stream,
            self.alpha_time_base,
        )?;
        self.output
            .write_trailer()
            .map_err(|error| format!("could not finalize visual cache media: {error}"))
    }
}

fn open_encoder(
    width: u32,
    height: u32,
    time_base: ffmpeg::Rational,
    frame_rate: ffmpeg::Rational,
    qp: u32,
    global_header: bool,
) -> Result<ffmpeg::codec::encoder::video::Encoder, String> {
    let codec = ffmpeg::codec::encoder::find_by_name("hevc_nvenc")
        .ok_or_else(|| "FFmpeg encoder hevc_nvenc was not found".to_string())?;
    let mut encoder = ffmpeg::codec::Context::new_with_codec(codec)
        .encoder()
        .video()
        .map_err(|error| format!("could not configure hevc_nvenc: {error}"))?;
    encoder.set_width(width);
    encoder.set_height(height);
    encoder.set_time_base(time_base);
    encoder.set_frame_rate(Some(frame_rate));
    encoder.set_gop(2);
    encoder.set_max_b_frames(0);
    encoder.set_format(Pixel::NV12);
    if global_header {
        unsafe {
            (*encoder.as_mut_ptr()).flags |= ffmpeg::sys::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
        }
    }
    let mut options = ffmpeg::Dictionary::new();
    options.set("preset", if qp == 0 { "lossless" } else { "p4" });
    options.set("rc", "constqp");
    options.set("qp", &qp.to_string());
    options.set("bf", "0");
    options.set("forced-idr", "1");
    options.set("rc-lookahead", "0");
    options.set("tune", if qp == 0 { "lossless" } else { "hq" });
    encoder
        .open_as_with(codec, options)
        .map_err(|error| format!("could not open hevc_nvenc: {error}"))
}

fn receive_packets(
    encoder: &mut ffmpeg::codec::encoder::video::Encoder,
    output: &mut ffmpeg::format::context::Output,
    stream_index: usize,
    stream_time_base: ffmpeg::Rational,
) -> Result<(), String> {
    loop {
        let mut packet = ffmpeg::Packet::empty();
        match encoder.receive_packet(&mut packet) {
            Ok(()) => {
                packet.set_stream(stream_index);
                packet.rescale_ts(encoder.time_base(), stream_time_base);
                packet
                    .write_interleaved(output)
                    .map_err(|error| format!("could not write visual cache packet: {error}"))?;
            }
            Err(ffmpeg::Error::Other { errno }) if errno == EAGAIN => return Ok(()),
            Err(ffmpeg::Error::Eof) => return Ok(()),
            Err(error) => return Err(format!("could not receive visual cache packet: {error}")),
        }
    }
}
