use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::thread;

use ffmpeg_next as ffmpeg;
use libc::EAGAIN;
use serde::{Deserialize, Serialize};
use shrimply_audio_modifiers::{AudioModifierEffect, CacheFormat, CacheModifier, OpusCacheQuality};
use shrimply_path_core::project_cache_directory;
use shrimply_project_document::project::{
    AudioItem, AudioSource, AudioTrack, ItemAddress, Project, RepeatStrategy, Time,
    default_playback_speed,
};
use shrimply_resource_pipeline::{
    Event, JobContext, Pipeline, Processor, RequestDisposition, Subscription, TryNext,
};
use uuid::Uuid;

const MANIFEST_NAME: &str = "manifest.json";
const CACHE_VERSION: u32 = 1;
const SAMPLE_RATE: u32 = 48_000;
const CHANNELS: usize = 2;

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
    modifier_id: Uuid,
    settings: CacheModifier,
    duration: Time,
}

struct BakeProcessor {
    inputs: Arc<Mutex<HashMap<Uuid, BakeInput>>>,
}

struct Runtime {
    pipeline: Pipeline<Uuid, BakeProcessor>,
    inputs: Arc<Mutex<HashMap<Uuid, BakeInput>>>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StoredFormat {
    Opus,
    Flac,
}

#[derive(Serialize, Deserialize)]
struct Manifest {
    version: u32,
    kind: String,
    format: StoredFormat,
    duration: Time,
    sample_rate: u32,
    channels: u32,
}

struct ReadyEntry {
    path: PathBuf,
    duration: Time,
}

static JOBS: LazyLock<Mutex<HashMap<Uuid, Job>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
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
                        "audio cache job closed without a terminal event".to_string(),
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
        let mut inputs = self.inputs.lock().expect("audio cache input lock poisoned");
        assert!(
            !inputs.contains_key(&modifier_id),
            "audio cache input already exists"
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
            .expect("audio cache input lock poisoned")
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
            .expect("audio cache input lock poisoned")
            .remove(&modifier_id)
            .ok_or_else(|| "audio cache bake input disappeared".to_string())?;
        bake_inner(
            input.project,
            modifier_id,
            &input.settings,
            input.duration,
            context,
        )
    }
}

pub fn status(modifier_id: Uuid) -> Status {
    let job = {
        let mut jobs = JOBS.lock().expect("audio modifier cache job lock poisoned");
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
        .expect("audio cache operation lock poisoned");
    if matches!(status(modifier_id), Status::Baking { .. }) {
        return Err("this cache is already baking".to_string());
    }
    let (project, settings, duration) = bake_project(project, &address, modifier_id)?;
    invalidate_inner(modifier_id)?;
    let total = duration.as_sample_frame(SAMPLE_RATE);
    let (disposition, subscription) = RUNTIME.request(BakeInput {
        project,
        modifier_id,
        settings,
        duration,
    });
    if disposition == RequestDisposition::Joined {
        subscription.cancel();
        return Err("this cache is already baking".to_string());
    }
    JOBS.lock()
        .expect("audio modifier cache job lock poisoned")
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
        .expect("audio cache operation lock poisoned");
    invalidate_inner(modifier_id)
}

fn invalidate_inner(modifier_id: Uuid) -> Result<(), String> {
    RUNTIME.cancel(modifier_id);
    JOBS.lock()
        .expect("audio modifier cache job lock poisoned")
        .remove(&modifier_id);
    match fs::remove_dir_all(cache_directory(modifier_id)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("could not invalidate audio cache: {error}")),
    }
}

pub(crate) fn effective_item(item: &AudioItem) -> Result<Option<AudioItem>, String> {
    let mut ready = None;
    for (index, modifier) in item.modifiers.iter().enumerate().rev() {
        if !modifier.enabled
            || !matches!(modifier.effect, AudioModifierEffect::Cache(_))
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
    effective.source = AudioSource::Media;
    effective.track_id = 0;
    effective.time_offset = Time::ZERO;
    effective.source_duration = entry.duration;
    effective.playback_speed = default_playback_speed();
    effective.repeat_strategy = RepeatStrategy::Empty;
    effective.modifiers = item.modifiers[index + 1..].to_vec();
    Ok(Some(effective))
}

fn bake_project(
    mut project: Project,
    address: &ItemAddress,
    modifier_id: Uuid,
) -> Result<(Project, CacheModifier, Time), String> {
    let item = project
        .audio_item(address)
        .ok_or_else(|| "audio cache item no longer exists".to_string())?;
    let index = item
        .modifiers
        .iter()
        .position(|modifier| modifier.id == modifier_id)
        .ok_or_else(|| "audio cache modifier no longer exists".to_string())?;
    let AudioModifierEffect::Cache(settings) = &item.modifiers[index].effect else {
        return Err("selected audio modifier is not a cache".to_string());
    };
    let settings = settings.clone();
    let duration = item.end.saturating_sub(item.start);
    if duration == Time::ZERO {
        return Err("cannot cache an empty audio item".to_string());
    }
    let mut item = item.clone();
    item.start = Time::ZERO;
    item.end = duration;
    item.modifiers.truncate(index);
    item.gain = Default::default();
    item.transitions = Default::default();
    project.caption_tracks.clear();
    project.video_tracks.clear();
    project.audio_tracks = vec![AudioTrack {
        items: vec![item],
        ..Default::default()
    }];
    Ok((project, settings, duration))
}

fn bake_inner(
    project: Project,
    modifier_id: Uuid,
    settings: &CacheModifier,
    duration: Time,
    context: &JobContext<Progress>,
) -> Result<(), String> {
    if context.is_cancelled() {
        return Err("audio cache bake cancelled".to_string());
    }
    let root = project_cache_directory().join("modifiers");
    fs::create_dir_all(&root).map_err(|error| format!("could not create cache folder: {error}"))?;
    let temporary = root.join(format!(
        ".{}-{}",
        modifier_id.simple(),
        Uuid::new_v4().simple()
    ));
    fs::create_dir(&temporary)
        .map_err(|error| format!("could not create temporary cache folder: {error}"))?;
    let result = (|| {
        let total = duration.as_sample_frame(SAMPLE_RATE);
        let samples = super::streaming::mix_project_offline(&project, SAMPLE_RATE, |done, _| {
            context.report(Progress {
                completed: done,
                total,
            })
        })?;
        if context.is_cancelled() {
            return Err("audio cache bake cancelled".to_string());
        }
        let stored_format = match settings.format {
            CacheFormat::Opus => StoredFormat::Opus,
            CacheFormat::Flac => StoredFormat::Flac,
        };
        let path = temporary.join(media_name(stored_format));
        encode(&samples, &path, stored_format, settings.opus_quality)?;
        let manifest = Manifest {
            version: CACHE_VERSION,
            kind: "audio".to_string(),
            format: stored_format,
            duration,
            sample_rate: SAMPLE_RATE,
            channels: CHANNELS as u32,
        };
        fs::write(
            temporary.join(MANIFEST_NAME),
            serde_json::to_vec(&manifest)
                .map_err(|error| format!("could not encode audio cache manifest: {error}"))?,
        )
        .map_err(|error| format!("could not write audio cache manifest: {error}"))?;
        let destination = cache_directory(modifier_id);
        let _operation = CACHE_OPERATIONS
            .lock()
            .expect("audio cache operation lock poisoned");
        if context.is_cancelled() {
            return Err("audio cache bake cancelled".to_string());
        }
        fs::rename(&temporary, &destination)
            .map_err(|error| format!("could not finish audio cache: {error}"))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&temporary);
    }
    result
}

fn ready_entry(modifier_id: Uuid) -> Result<ReadyEntry, String> {
    let directory = cache_directory(modifier_id);
    let bytes = fs::read(directory.join(MANIFEST_NAME))
        .map_err(|error| format!("audio cache is missing its manifest: {error}"))?;
    let manifest: Manifest = serde_json::from_slice(&bytes)
        .map_err(|error| format!("audio cache manifest is invalid: {error}"))?;
    if manifest.version != CACHE_VERSION || manifest.kind != "audio" {
        return Err("audio cache version is unsupported; invalidate and rebake it".to_string());
    }
    if manifest.sample_rate != SAMPLE_RATE || manifest.channels != CHANNELS as u32 {
        return Err("audio cache layout is unsupported; invalidate and rebake it".to_string());
    }
    let path = directory.join(media_name(manifest.format));
    if !path.is_file() {
        return Err("audio cache media is missing".to_string());
    }
    Ok(ReadyEntry {
        path,
        duration: manifest.duration,
    })
}

fn cache_directory(modifier_id: Uuid) -> PathBuf {
    project_cache_directory()
        .join("modifiers")
        .join(modifier_id.simple().to_string())
}

fn media_name(format: StoredFormat) -> &'static str {
    match format {
        StoredFormat::Opus => "audio.opus",
        StoredFormat::Flac => "audio.flac",
    }
}

fn encode(
    samples: &[f32],
    path: &Path,
    format: StoredFormat,
    quality: OpusCacheQuality,
) -> Result<(), String> {
    ffmpeg::init().map_err(|error| format!("could not initialize FFmpeg: {error}"))?;
    let (encoder_name, sample_format) = match format {
        StoredFormat::Opus => (
            "libopus",
            ffmpeg::format::Sample::F32(ffmpeg::format::sample::Type::Packed),
        ),
        StoredFormat::Flac => (
            "flac",
            ffmpeg::format::Sample::I32(ffmpeg::format::sample::Type::Packed),
        ),
    };
    let codec = ffmpeg::codec::encoder::find_by_name(encoder_name)
        .ok_or_else(|| format!("FFmpeg encoder {encoder_name} was not found"))?;
    let mut encoder = ffmpeg::codec::Context::new_with_codec(codec)
        .encoder()
        .audio()
        .map_err(|error| format!("could not configure {encoder_name}: {error}"))?;
    encoder.set_rate(SAMPLE_RATE as i32);
    encoder.set_channel_layout(ffmpeg::channel_layout::ChannelLayout::STEREO);
    encoder.set_format(sample_format);
    encoder.set_time_base(ffmpeg::Rational(1, SAMPLE_RATE as i32));
    if matches!(format, StoredFormat::Opus) {
        encoder.set_bit_rate(quality.bitrate());
    }
    let mut output = ffmpeg::format::output(path)
        .map_err(|error| format!("could not create audio cache media: {error}"))?;
    if output
        .format()
        .flags()
        .contains(ffmpeg::format::Flags::GLOBAL_HEADER)
    {
        unsafe {
            (*encoder.as_mut_ptr()).flags |= ffmpeg::sys::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
        }
    }
    let mut encoder = encoder
        .open_as(codec)
        .map_err(|error| format!("could not open {encoder_name}: {error}"))?;
    let stream_index = {
        let mut stream = output
            .add_stream_with(encoder.as_ref())
            .map_err(|error| format!("could not add audio cache stream: {error}"))?;
        stream.set_time_base(ffmpeg::Rational(1, SAMPLE_RATE as i32));
        stream.index()
    };
    output
        .write_header()
        .map_err(|error| format!("could not write audio cache header: {error}"))?;
    let stream_time_base = output
        .stream(stream_index)
        .expect("audio cache stream disappeared")
        .time_base();
    let frame_size = usize::try_from(encoder.frame_size())
        .ok()
        .filter(|size| *size > 0)
        .unwrap_or(1024);
    let mut offset = 0;
    let mut pts = 0;
    while offset < samples.len() / CHANNELS {
        let frames = frame_size.min(samples.len() / CHANNELS - offset);
        let mut frame = ffmpeg::frame::Audio::new(
            sample_format,
            frame_size,
            ffmpeg::channel_layout::ChannelLayout::STEREO,
        );
        frame.set_rate(SAMPLE_RATE);
        frame.set_pts(Some(pts));
        fill_frame(&mut frame, format, &samples[offset * CHANNELS..], frames);
        encoder
            .send_frame(&frame)
            .map_err(|error| format!("could not encode audio cache: {error}"))?;
        write_packets(&mut encoder, &mut output, stream_index, stream_time_base)?;
        offset += frames;
        pts += frame_size as i64;
    }
    encoder
        .send_eof()
        .map_err(|error| format!("could not finalize audio cache encoder: {error}"))?;
    write_packets(&mut encoder, &mut output, stream_index, stream_time_base)?;
    output
        .write_trailer()
        .map_err(|error| format!("could not finalize audio cache media: {error}"))
}

fn fill_frame(
    frame: &mut ffmpeg::frame::Audio,
    format: StoredFormat,
    samples: &[f32],
    frames: usize,
) {
    match format {
        StoredFormat::Opus => {
            for (index, sample) in frame.plane_mut::<(f32, f32)>(0).iter_mut().enumerate() {
                *sample = if index < frames {
                    (
                        samples[index * CHANNELS].clamp(-1.0, 1.0),
                        samples[index * CHANNELS + 1].clamp(-1.0, 1.0),
                    )
                } else {
                    (0.0, 0.0)
                };
            }
        }
        StoredFormat::Flac => {
            for (index, sample) in frame.plane_mut::<(i32, i32)>(0).iter_mut().enumerate() {
                *sample = if index < frames {
                    (
                        to_i32(samples[index * CHANNELS]),
                        to_i32(samples[index * CHANNELS + 1]),
                    )
                } else {
                    (0, 0)
                };
            }
        }
    }
}

fn to_i32(sample: f32) -> i32 {
    (sample.clamp(-1.0, 1.0) * i32::MAX as f32).round() as i32
}

fn write_packets(
    encoder: &mut ffmpeg::codec::encoder::audio::Encoder,
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
                    .map_err(|error| format!("could not write audio cache packet: {error}"))?;
            }
            Err(ffmpeg::Error::Other { errno }) if errno == EAGAIN => return Ok(()),
            Err(ffmpeg::Error::Eof) => return Ok(()),
            Err(error) => return Err(format!("could not receive audio cache packet: {error}")),
        }
    }
}
