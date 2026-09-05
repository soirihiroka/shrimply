use hashbrown::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use ffmpeg::{Rational, codec, encoder, format, media};
use ffmpeg_next as ffmpeg;
use glam::{UVec2, Vec2};
use shrimply_math_core::Fraction;
use shrimply_resource_pipeline::{JobContext, Pipeline, Processor, Subscription};

use shrimply_core::timeline_value::*;
use shrimply_project::project::{
    Asset, AssetSnapshot, AudioItem, CanvasSize, CaptionItem, LayerVisibility, LayeredImageItem,
    Project, RepeatStrategy, ResolvedTransform, Time, Transform, VideoItem, VideoItemContent,
    VisualModifier, default_playback_speed,
};
use shrimply_project::timeline_search;
use shrimply_state::player_state::{self, ProjectChange, SharedPlayerState};
use shrimply_timeline::selection_state::{self, SharedSelectionState};
use shrimply_video_modifiers::{ModifierEffect, scene_3d::Scene3dModifierEffect};

use shrimply_timeline::{ItemKey, TrackKind, insert_sorted, next_group_id};

type VideoSizes = Vec<UVec2>;
type VideoSizeCache = Mutex<HashMap<AssetSnapshot, VideoSizes>>;

static VIDEO_SIZE_CACHE: OnceLock<VideoSizeCache> = OnceLock::new();
static INSPECTION_CACHE: OnceLock<Mutex<HashMap<InspectionKey, MediaInfo>>> = OnceLock::new();
static INSPECTIONS: OnceLock<Pipeline<InspectionKey, MediaInspector>> = OnceLock::new();
static INSPECTION_WORKERS: OnceLock<std::sync::mpsc::SyncSender<shrimply_resource_pipeline::Job>> =
    OnceLock::new();

const MEDIA_INSPECTION_THREADS: usize = 2;
const MEDIA_INSPECTION_QUEUE: usize = 64;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct InspectionKey {
    path: PathBuf,
    canvas_width: u32,
    canvas_height: u32,
    default_visual_duration_nanos: i128,
}

struct MediaInspector;

impl Processor<InspectionKey> for MediaInspector {
    type Progress = ();
    type Output = MediaInfo;

    fn process(
        &self,
        key: InspectionKey,
        _context: &JobContext<Self::Progress>,
    ) -> Result<Self::Output, String> {
        if let Some(info) = INSPECTION_CACHE
            .get_or_init(Mutex::default)
            .lock()
            .expect("media inspection cache lock poisoned")
            .get(&key)
            .filter(|info| info.snapshot.ensure_current().is_ok())
            .cloned()
        {
            return Ok(info);
        }
        let info = inspect(
            key.path.clone(),
            CanvasSize {
                width: key.canvas_width,
                height: key.canvas_height,
            },
            Time::from_nanos_i128(key.default_visual_duration_nanos),
        )?;
        INSPECTION_CACHE
            .get_or_init(Mutex::default)
            .lock()
            .expect("media inspection cache lock poisoned")
            .insert(key, info.clone());
        Ok(info)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileKind {
    Mp4,
    Mov,
    Mkv,
    WebM,
    Image,
    Gif,
    Svg,
    Pdf,
    Python,
    Blender,
    LayeredImage,
    Obj,
    Ply,
    Audio,
    Vtt,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VisualMediaKind {
    Video,
    Image,
    Gif,
    Svg,
    Pdf,
    Manim,
    Blender,
    LayeredImage,
    Obj,
    Gaussian,
}

#[derive(Clone)]
pub struct MediaInfo {
    pub source: Asset,
    pub snapshot: AssetSnapshot,
    pub duration: Time,
    pub video_streams: usize,
    pub audio_streams: usize,
    pub visual_kind: Option<VisualMediaKind>,
    pub video_sizes: VideoSizes,
    pub video_fps: Option<Fraction>,
    pub layered_image_layers: Vec<LayerVisibility>,
}

pub struct ImportResult {
    pub selection: Vec<ItemKey>,
    pub video: bool,
    pub audio: bool,
    pub captions: bool,
}

pub fn request_inspection(
    path: PathBuf,
    canvas_size: CanvasSize,
    default_visual_duration: Time,
) -> Subscription<InspectionKey, (), MediaInfo> {
    let key = InspectionKey {
        path,
        canvas_width: canvas_size.width,
        canvas_height: canvas_size.height,
        default_visual_duration_nanos: default_visual_duration.as_nanos_i128(),
    };
    INSPECTIONS
        .get_or_init(|| {
            Pipeline::new(MediaInspector, |job| {
                let queued = INSPECTION_WORKERS
                    .get_or_init(|| {
                        let (sender, receiver) = std::sync::mpsc::sync_channel::<
                            shrimply_resource_pipeline::Job,
                        >(MEDIA_INSPECTION_QUEUE);
                        let receiver = std::sync::Arc::new(Mutex::new(receiver));
                        for index in 0..MEDIA_INSPECTION_THREADS {
                            let receiver = receiver.clone();
                            std::thread::Builder::new()
                                .name(format!("media-inspection-{index}"))
                                .spawn(move || {
                                    loop {
                                        let job = receiver
                                            .lock()
                                            .expect("media inspection queue lock poisoned")
                                            .recv();
                                        let Ok(job) = job else { break };
                                        job();
                                    }
                                })
                                .expect("could not start media inspection worker");
                        }
                        sender
                    })
                    .try_send(job);
                assert!(queued.is_ok(), "media inspection queue is full");
            })
        })
        .request(key)
        .1
}

pub fn file_kind(path: &Path) -> Option<FileKind> {
    let extension = path.extension()?.to_str()?;
    let extension = if extension.eq_ignore_ascii_case("part") {
        Path::new(path.file_stem()?)
            .extension()
            .and_then(|extension| extension.to_str())?
    } else {
        extension
    };
    if extension.eq_ignore_ascii_case("mp4") {
        Some(FileKind::Mp4)
    } else if extension.eq_ignore_ascii_case("mov") {
        Some(FileKind::Mov)
    } else if extension.eq_ignore_ascii_case("mkv") {
        Some(FileKind::Mkv)
    } else if extension.eq_ignore_ascii_case("webm") {
        Some(FileKind::WebM)
    } else if image_extension(extension) {
        Some(FileKind::Image)
    } else if extension.eq_ignore_ascii_case("gif") {
        Some(FileKind::Gif)
    } else if extension.eq_ignore_ascii_case("svg") {
        Some(FileKind::Svg)
    } else if extension.eq_ignore_ascii_case("pdf") {
        Some(FileKind::Pdf)
    } else if extension.eq_ignore_ascii_case("py") {
        Some(FileKind::Python)
    } else if extension.eq_ignore_ascii_case("blend") {
        Some(FileKind::Blender)
    } else if extension.eq_ignore_ascii_case("psd") || extension.eq_ignore_ascii_case("kra") {
        Some(FileKind::LayeredImage)
    } else if extension.eq_ignore_ascii_case("obj") || extension.eq_ignore_ascii_case("glb") {
        Some(FileKind::Obj)
    } else if extension.eq_ignore_ascii_case("ply") {
        Some(FileKind::Ply)
    } else if extension.eq_ignore_ascii_case("vtt") {
        Some(FileKind::Vtt)
    } else if audio_extension(extension) {
        Some(FileKind::Audio)
    } else {
        None
    }
}

pub fn direct_media_kind(kind: FileKind) -> bool {
    matches!(
        kind,
        FileKind::Mp4
            | FileKind::Mov
            | FileKind::Image
            | FileKind::Gif
            | FileKind::Svg
            | FileKind::Pdf
            | FileKind::Python
            | FileKind::Blender
            | FileKind::LayeredImage
            | FileKind::Obj
            | FileKind::Ply
            | FileKind::Audio
    )
}

pub enum TrackImportStart {
    Inspect(TrackImportInspection),
    Complete((ImportResult, Time)),
}

pub struct TrackImportInspection {
    pub subscription: Subscription<InspectionKey, (), MediaInfo>,
    pub context: TrackImportContext,
}

pub struct TrackImportContext {
    kind: TrackKind,
    track_indices: Vec<usize>,
    start: Time,
}

pub fn start_track_import(
    project: &mut Project,
    path: PathBuf,
    kind: TrackKind,
    track_indices: Vec<usize>,
    start: Time,
    default_visual_duration: Time,
) -> Result<TrackImportStart, String> {
    if track_indices.is_empty() {
        return Err("no import tracks were selected".to_string());
    }
    let file_kind = file_kind(&path).ok_or_else(|| "unsupported file type".to_string())?;
    if direct_media_kind(file_kind) && kind != TrackKind::Caption {
        return Ok(TrackImportStart::Inspect(TrackImportInspection {
            subscription: request_inspection(path, project.canvas_size, default_visual_duration),
            context: TrackImportContext {
                kind,
                track_indices,
                start,
            },
        }));
    }
    if file_kind != FileKind::Vtt {
        return Err(if kind == TrackKind::Caption {
            "only VTT files can be imported to caption tracks"
        } else {
            "MKV and WebM need to be remuxed before track import"
        }
        .to_string());
    }
    if kind != TrackKind::Caption {
        return Err("VTT files can only be imported to caption tracks".to_string());
    }
    let result = apply_vtt_to_tracks(project, &path, &track_indices, start)?;
    shrimply_project::project::commit_edit(project, "import-vtt");
    Ok(TrackImportStart::Complete((result, project.duration())))
}

pub fn finish_track_import_inspection(
    project: &mut Project,
    context: TrackImportContext,
    info: &MediaInfo,
) -> Result<(ImportResult, Time), String> {
    info.snapshot.ensure_current()?;
    let result = apply_media_to_tracks(
        project,
        info,
        context.kind,
        &context.track_indices,
        context.start,
    )?;
    shrimply_project::project::commit_edit(project, "import-media-to-tracks");
    Ok((result, project.duration()))
}

pub fn finish_track_import(
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    result: Result<(ImportResult, Time), String>,
) -> Result<(), String> {
    let (result, duration) = result?;
    let focused_item = result.selection.first().copied();
    selection_state::set_selected_items(selection_state, result.selection, focused_item);
    player_state::refresh_project(
        player_state,
        ProjectChange {
            duration: Some(duration),
            frame_rate: None,
            audio: result.audio,
            audio_beats: result.audio,
            audio_waveforms: result.audio,
            video: result.video,
            live_preview: false,
            captions: result.captions,
            inspector: true,
        },
    );
    Ok(())
}

pub fn inspect(
    path: PathBuf,
    canvas_size: CanvasSize,
    default_visual_duration: Time,
) -> Result<MediaInfo, String> {
    let source = Asset::new(path);
    let snapshot = source.snapshot()?;
    let path = snapshot.path();
    let file_kind = file_kind(path).ok_or_else(|| "unsupported file type".to_string())?;
    if matches!(file_kind, FileKind::Python | FileKind::Blender) {
        let duration = if file_kind == FileKind::Blender {
            Time {
                seconds: shrimply_blender::file_duration(path)?,
            }
        } else {
            default_visual_duration
        };
        return Ok(MediaInfo {
            source,
            snapshot,
            duration,
            visual_kind: Some(if file_kind == FileKind::Python {
                VisualMediaKind::Manim
            } else {
                VisualMediaKind::Blender
            }),
            video_streams: 1,
            video_sizes: vec![UVec2::new(
                canvas_size.width.max(1),
                canvas_size.height.max(1),
            )],
            video_fps: None,
            audio_streams: 0,
            layered_image_layers: Vec::new(),
        });
    }
    if file_kind == FileKind::Svg {
        let size = snapshot
            .read_to_string()
            .ok()
            .and_then(|svg| svg_native_size(&svg))
            .unwrap_or_else(|| UVec2::new(canvas_size.width.max(1), canvas_size.height.max(1)));
        return Ok(MediaInfo {
            source,
            snapshot,
            duration: default_visual_duration,
            visual_kind: Some(VisualMediaKind::Svg),
            video_streams: 1,
            video_sizes: vec![size],
            video_fps: None,
            audio_streams: 0,
            layered_image_layers: Vec::new(),
        });
    }
    if file_kind == FileKind::Pdf {
        let pages = shrimply_pdf::page_sizes(snapshot.read()?)?;
        let first = pages
            .first()
            .expect("PDF inspection requires at least one page");
        return Ok(MediaInfo {
            source,
            snapshot,
            duration: default_visual_duration,
            visual_kind: Some(VisualMediaKind::Pdf),
            video_streams: 1,
            video_sizes: vec![UVec2::new(first.width, first.height)],
            video_fps: None,
            audio_streams: 0,
            layered_image_layers: Vec::new(),
        });
    }
    if file_kind == FileKind::LayeredImage {
        let image = shrimply_video::load_layered_image(source.clone())?;
        snapshot.ensure_current()?;
        let layered_image_layers = image
            .entries
            .iter()
            .map(|entry| LayerVisibility {
                id: uuid::Uuid::new_v4(),
                path: entry.path.clone(),
                visibility: None,
            })
            .collect();
        return Ok(MediaInfo {
            source,
            snapshot,
            duration: default_visual_duration,
            visual_kind: Some(VisualMediaKind::LayeredImage),
            video_streams: 1,
            video_sizes: vec![UVec2::new(image.width, image.height)],
            video_fps: None,
            audio_streams: 0,
            layered_image_layers,
        });
    }
    if matches!(file_kind, FileKind::Obj | FileKind::Ply) {
        let visual_kind = if file_kind == FileKind::Ply {
            shrimply_3dgs::RenderSession::load(&source)
                .map_err(|error| format!("could not parse {}: {error}", path.display()))?;
            VisualMediaKind::Gaussian
        } else {
            if path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("glb"))
            {
                shrimply_scene_3d::load_glb(path)
                    .map_err(|error| format!("could not parse {}: {error}", path.display()))?;
            } else {
                shrimply_scene_3d::load_obj(path)
                    .map_err(|error| format!("could not parse {}: {error}", path.display()))?;
            }
            VisualMediaKind::Obj
        };
        snapshot.verify_current()?;
        return Ok(MediaInfo {
            source,
            snapshot,
            duration: default_visual_duration,
            visual_kind: Some(visual_kind),
            video_streams: 1,
            video_sizes: vec![UVec2::new(
                canvas_size.width.max(1),
                canvas_size.height.max(1),
            )],
            video_fps: None,
            audio_streams: 0,
            layered_image_layers: Vec::new(),
        });
    }

    let input = format::input(path)
        .map_err(|error| format!("could not open {}: {error}", path.display()))?;
    let mut video_streams = 0;
    let cached_video_sizes = VIDEO_SIZE_CACHE
        .get_or_init(Mutex::default)
        .lock()
        .ok()
        .and_then(|cache| cache.get(&snapshot).cloned());
    let mut video_sizes = Vec::new();
    let mut video_fps = None;
    let mut audio_streams = 0;
    let mut stream_duration_seconds = 0.0_f64;

    for stream in input.streams() {
        match stream.parameters().medium() {
            media::Type::Video if file_kind != FileKind::Audio => {
                video_streams += 1;
                if video_fps.is_none() {
                    let mut rate = stream.avg_frame_rate();
                    if rate.numerator() <= 0 || rate.denominator() <= 0 {
                        rate = stream.rate();
                    }
                    if rate.numerator() > 0 && rate.denominator() > 0 {
                        video_fps = Some(Fraction::new_raw(
                            u64::try_from(rate.numerator()).expect("positive video frame rate"),
                            u64::try_from(rate.denominator()).expect("positive video frame rate"),
                        ));
                    }
                }
                if let Some(size) = cached_video_sizes
                    .as_ref()
                    .and_then(|sizes| sizes.get(video_streams - 1))
                {
                    video_sizes.push(*size);
                } else if let Ok(decoder) =
                    codec::context::Context::from_parameters(stream.parameters())
                        .and_then(|context| context.decoder().video())
                {
                    video_sizes.push(UVec2::new(decoder.width(), decoder.height()));
                } else {
                    video_sizes.push(UVec2::ZERO);
                }
            }
            media::Type::Audio => audio_streams += 1,
            _ => {}
        }
        if stream.duration() > 0 {
            stream_duration_seconds = stream_duration_seconds
                .max(stream.duration() as f64 * rational_as_f64(stream.time_base()));
        }
    }

    if video_streams == 0 && audio_streams == 0 {
        return Err(format!("{} has no audio or video streams", path.display()));
    }

    let container_duration_seconds = if input.duration() > 0 {
        input.duration() as f64 / 1_000_000.0
    } else {
        0.0
    };
    if cached_video_sizes.is_none()
        && let Ok(mut cache) = VIDEO_SIZE_CACHE.get_or_init(Mutex::default).lock()
    {
        cache.insert(snapshot.clone(), video_sizes.clone());
    }

    let visual_kind = match file_kind {
        FileKind::Image => Some(VisualMediaKind::Image),
        FileKind::Gif => Some(VisualMediaKind::Gif),
        FileKind::Mp4 | FileKind::Mov | FileKind::Mkv | FileKind::WebM if video_streams > 0 => {
            Some(VisualMediaKind::Video)
        }
        _ => None,
    };
    if matches!(file_kind, FileKind::Image | FileKind::Gif) {
        audio_streams = 0;
        video_streams = video_streams.min(1);
        video_sizes.truncate(video_streams);
    }
    let duration = match file_kind {
        FileKind::Image => default_visual_duration,
        FileKind::Gif => {
            let source_duration =
                Time::from_seconds_f64(container_duration_seconds.max(stream_duration_seconds));
            if source_duration > Time::ZERO {
                source_duration
            } else {
                default_visual_duration
            }
        }
        _ => Time::from_seconds_f64(container_duration_seconds.max(stream_duration_seconds)),
    };

    snapshot.verify_current()?;
    Ok(MediaInfo {
        source,
        snapshot,
        duration,
        visual_kind,
        video_streams,
        video_sizes,
        video_fps,
        audio_streams,
        layered_image_layers: Vec::new(),
    })
}

fn svg_native_size(svg: &str) -> Option<UVec2> {
    let tag = svg_root_tag(svg)?;
    let width = svg_attr(tag, "width").and_then(svg_length_px);
    let height = svg_attr(tag, "height").and_then(svg_length_px);
    let view_box = svg_attr(tag, "viewBox").and_then(svg_view_box_size);

    match (width, height, view_box) {
        (Some(width), Some(height), _) => Some(UVec2::new(size_px(width)?, size_px(height)?)),
        (Some(width), None, Some(view_box)) if view_box.x > 0.0 => Some(UVec2::new(
            size_px(width)?,
            size_px(width * view_box.y / view_box.x)?,
        )),
        (None, Some(height), Some(view_box)) if view_box.y > 0.0 => Some(UVec2::new(
            size_px(height * view_box.x / view_box.y)?,
            size_px(height)?,
        )),
        (None, None, Some(view_box)) => {
            Some(UVec2::new(size_px(view_box.x)?, size_px(view_box.y)?))
        }
        _ => None,
    }
}

fn svg_root_tag(svg: &str) -> Option<&str> {
    let start = svg.find("<svg")?;
    let rest = &svg[start..];
    let end = rest.find('>')?;
    Some(&rest[..end])
}

fn svg_attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let mut rest = tag;
    loop {
        let index = rest.find(name)?;
        let before = rest[..index].chars().next_back();
        let after = rest[index + name.len()..].chars().next();
        if before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            || after.is_some_and(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            rest = &rest[index + name.len()..];
            continue;
        }

        let mut value = rest[index + name.len()..].trim_start();
        if !value.starts_with('=') {
            rest = &rest[index + name.len()..];
            continue;
        }
        value = value[1..].trim_start();
        let quote = value.chars().next()?;
        if quote != '"' && quote != '\'' {
            return None;
        }
        value = &value[quote.len_utf8()..];
        let end = value.find(quote)?;
        return Some(&value[..end]);
    }
}

fn svg_length_px(value: &str) -> Option<f32> {
    let value = value.trim();
    if value.ends_with('%') {
        return None;
    }
    let number_end = value
        .char_indices()
        .take_while(|(_, c)| c.is_ascii_digit() || matches!(c, '.' | '+' | '-' | 'e' | 'E'))
        .map(|(index, c)| index + c.len_utf8())
        .last()?;
    let number: f32 = value[..number_end].parse().ok()?;
    if !number.is_finite() || number <= 0.0 {
        return None;
    }
    let unit = value[number_end..].trim();
    let scale = match unit {
        "" | "px" => 1.0,
        "in" => 96.0,
        "cm" => 96.0 / 2.54,
        "mm" => 96.0 / 25.4,
        "pt" => 96.0 / 72.0,
        "pc" => 16.0,
        _ => return None,
    };
    Some(number * scale)
}

fn svg_view_box_size(value: &str) -> Option<Vec2> {
    let values: Vec<f32> = value
        .split(|c: char| c.is_ascii_whitespace() || c == ',')
        .filter(|part| !part.is_empty())
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    if values.len() != 4 || values.iter().any(|value| !value.is_finite()) {
        return None;
    }
    (values[2] > 0.0 && values[3] > 0.0).then(|| Vec2::new(values[2], values[3]))
}

fn size_px(value: f32) -> Option<u32> {
    (value.is_finite() && value > 0.0).then(|| value.ceil() as u32)
}

pub fn apply_media_to_tracks(
    project: &mut Project,
    info: &MediaInfo,
    kind: TrackKind,
    track_indices: &[usize],
    start: Time,
) -> Result<ImportResult, String> {
    let step = project.frame_step();
    let start = start.max(Time::ZERO).snapped(step);
    let end = start
        .saturating_add(info.duration)
        .snapped(step)
        .max(start.saturating_add(step));
    match kind {
        TrackKind::Video => apply_video_to_tracks(project, info, track_indices, start, end),
        TrackKind::Audio if info.visual_kind.is_some() => {
            Err("visual files cannot be imported to audio tracks".to_string())
        }
        TrackKind::Audio => apply_audio_to_tracks(project, info, track_indices, start, end),
        TrackKind::Caption => {
            Err("visual and media files cannot be imported to caption tracks".to_string())
        }
    }
}

pub fn apply_vtt_to_tracks(
    project: &mut Project,
    path: &Path,
    track_indices: &[usize],
    start: Time,
) -> Result<ImportResult, String> {
    let step = project.frame_step();
    let start = start.max(Time::ZERO).snapped(step);
    let cues = parse_vtt(path)?;
    if cues.is_empty() {
        return Err(format!("{} has no VTT cues", path.display()));
    }

    for &track_index in track_indices {
        let Some(track) = project.caption_tracks.get(track_index) else {
            return Err(format!("caption track {} does not exist", track_index + 1));
        };
        for cue in &cues {
            let item_start = start.saturating_add(cue.start).snapped(step);
            let item_end = start
                .saturating_add(cue.end)
                .snapped(step)
                .max(item_start.saturating_add(step));
            if timeline_search::collides(&track.items, item_start, item_end) {
                return Err(format!(
                    "VTT import collides with caption track {}",
                    track_index + 1
                ));
            }
        }
    }

    let mut selection = Vec::new();
    let group_id = (track_indices.len() > 1).then(|| next_group_id(project));
    for &track_index in track_indices {
        let Some(track) = project.caption_tracks.get_mut(track_index) else {
            continue;
        };
        for cue in &cues {
            let item_start = start.saturating_add(cue.start).snapped(step);
            let item_end = start
                .saturating_add(cue.end)
                .snapped(step)
                .max(item_start.saturating_add(step));
            let mut item = CaptionItem::new(item_start, item_end, cue.text.clone());
            item.group_id = group_id;
            let item_index = insert_sorted(&mut track.items, item);
            selection.push(ItemKey {
                kind: TrackKind::Caption,
                track_index,
                item_index,
            });
        }
    }

    Ok(ImportResult {
        selection,
        video: false,
        audio: false,
        captions: true,
    })
}

pub fn vtt_ranges(path: &Path) -> Result<Vec<(Time, Time)>, String> {
    Ok(parse_vtt(path)?
        .into_iter()
        .map(|cue| (cue.start, cue.end))
        .collect())
}

pub fn remux_mkv_to_mp4(input: &Path) -> Result<PathBuf, String> {
    let output = remux_output_path(input);
    remux(input, &output).inspect_err(|_| {
        let _ = fs::remove_file(&output);
    })?;
    Ok(output)
}

fn apply_video_to_tracks(
    project: &mut Project,
    info: &MediaInfo,
    track_indices: &[usize],
    start: Time,
    end: Time,
) -> Result<ImportResult, String> {
    if info.video_streams < track_indices.len() {
        return Err(format!(
            "{} has {} video stream(s), but {} video track(s) are selected",
            info.source.display(),
            info.video_streams,
            track_indices.len()
        ));
    }
    for &track_index in track_indices {
        let Some(track) = project.video_tracks.get(track_index) else {
            return Err(format!("video track {} does not exist", track_index + 1));
        };
        if timeline_search::collides(&track.items, start, end) {
            return Err(format!(
                "media import collides with video track {}",
                track_index + 1
            ));
        }
    }

    let mut selection = Vec::new();
    let group_id = (track_indices.len() > 1).then(|| next_group_id(project));
    for (stream_index, &track_index) in track_indices.iter().enumerate() {
        let source_size = info
            .video_sizes
            .get(stream_index)
            .copied()
            .filter(|size| size.x > 0 && size.y > 0);
        let transform = if matches!(
            info.visual_kind,
            Some(VisualMediaKind::Obj | VisualMediaKind::Gaussian)
        ) {
            Transform::from_resolved(ResolvedTransform::IDENTITY)
        } else {
            source_size
                .map(|size| Transform::natural_size(project.canvas_size, size.x, size.y))
                .unwrap_or_else(|| Transform::fill(project.canvas_size))
        };
        let source_size = source_size.unwrap_or_default();
        let item = VideoItem {
            id: uuid::Uuid::new_v4(),
            start,
            end,
            time_offset: Time::ZERO,
            source_duration: info.duration,
            playback_speed: default_playback_speed(),
            playback_fps: shrimply_project::project::native_playback_fps(),
            repeat_strategy: RepeatStrategy::Hold,
            stabilize_video: false,
            stabilization_method: Default::default(),
            stabilization_crop_ratio:
                shrimply_project::project::default_video_stabilization_crop_ratio(),
            stabilization_first_derivative_weight:
                shrimply_project::project::default_video_stabilization_first_derivative_weight(),
            stabilization_second_derivative_weight:
                shrimply_project::project::default_video_stabilization_second_derivative_weight(),
            stabilization_third_derivative_weight:
                shrimply_project::project::default_video_stabilization_third_derivative_weight(),
            mesh_flow_rows: shrimply_project::project::default_mesh_flow_rows(),
            mesh_flow_columns: shrimply_project::project::default_mesh_flow_columns(),
            mesh_flow_smoothing_radius:
                shrimply_project::project::default_mesh_flow_smoothing_radius(),
            mesh_flow_iterations: shrimply_project::project::default_mesh_flow_iterations(),
            mesh_flow_adaptive_weights: Default::default(),
            animation_time_offset: Time::ZERO,
            motion_blur: Default::default(),
            transform: transform.clone(),
            modifiers: modifiers_for_import(info),
            sample_method: TimelineValue::new_const(
                if matches!(info.visual_kind, Some(VisualMediaKind::LayeredImage)) {
                    shrimply_core::VideoSampleMethod::Nearest
                } else {
                    Default::default()
                },
            ),
            skia_drawing_strategy: Default::default(),
            compositing: Default::default(),
            visibility: TimelineValue::new_const(TimelineBool::True),
            alpha_mask_video: None,
            transitions: Default::default(),
            svg_color_overrides: Vec::new(),
            source_width: source_size.x,
            source_height: source_size.y,
            default_transform: Some(transform),
            content: video_content_for_import(info),
            video_generation: None,
            group_id,
            render_canvas_size: None,
            track_id: stream_index as u32,
            file: info.source.clone(),
        };
        let Some(track) = project.video_tracks.get_mut(track_index) else {
            continue;
        };
        let item_index = insert_sorted(&mut track.items, item);
        selection.push(ItemKey {
            kind: TrackKind::Video,
            track_index,
            item_index,
        });
    }

    Ok(ImportResult {
        selection,
        video: true,
        audio: false,
        captions: false,
    })
}

fn apply_audio_to_tracks(
    project: &mut Project,
    info: &MediaInfo,
    track_indices: &[usize],
    start: Time,
    end: Time,
) -> Result<ImportResult, String> {
    if info.audio_streams < track_indices.len() {
        return Err(format!(
            "{} has {} audio stream(s), but {} audio track(s) are selected",
            info.source.display(),
            info.audio_streams,
            track_indices.len()
        ));
    }
    for &track_index in track_indices {
        let Some(track) = project.audio_tracks.get(track_index) else {
            return Err(format!("audio track {} does not exist", track_index + 1));
        };
        if timeline_search::collides(&track.items, start, end) {
            return Err(format!(
                "media import collides with audio track {}",
                track_index + 1
            ));
        }
    }

    let mut selection = Vec::new();
    let group_id = (track_indices.len() > 1).then(|| next_group_id(project));
    for (stream_index, &track_index) in track_indices.iter().enumerate() {
        let item = AudioItem::builder(start, end)
            .source_duration(info.duration)
            .group_id(group_id)
            .track_id(stream_index as u32)
            .file(info.source.clone())
            .build();
        let Some(track) = project.audio_tracks.get_mut(track_index) else {
            continue;
        };
        let item_index = insert_sorted(&mut track.items, item);
        selection.push(ItemKey {
            kind: TrackKind::Audio,
            track_index,
            item_index,
        });
    }

    Ok(ImportResult {
        selection,
        video: false,
        audio: true,
        captions: false,
    })
}

pub fn video_content_for_import(info: &MediaInfo) -> VideoItemContent {
    match info.visual_kind.unwrap_or(VisualMediaKind::Video) {
        VisualMediaKind::Video => VideoItemContent::Media,
        VisualMediaKind::Image => VideoItemContent::Image,
        VisualMediaKind::Gif => VideoItemContent::Gif,
        VisualMediaKind::Svg => VideoItemContent::Svg,
        VisualMediaKind::Pdf => VideoItemContent::Pdf(Box::default()),
        VisualMediaKind::Manim => VideoItemContent::Manim(Box::default()),
        VisualMediaKind::Blender => VideoItemContent::Blender(Box::default()),
        VisualMediaKind::LayeredImage => {
            VideoItemContent::LayeredImage(Box::new(LayeredImageItem {
                layers: info.layered_image_layers.clone(),
            }))
        }
        VisualMediaKind::Obj => VideoItemContent::Obj(Box::default()),
        VisualMediaKind::Gaussian => VideoItemContent::Gaussian(Box::default()),
    }
}

pub fn modifiers_for_import(info: &MediaInfo) -> Vec<VisualModifier> {
    if info.visual_kind != Some(VisualMediaKind::Obj) {
        return Vec::new();
    }
    [
        Scene3dModifierEffect::Object(Box::new(
            shrimply_video_modifiers::scene_3d::Object3dModifier::with_file(info.source.clone()),
        )),
        Scene3dModifierEffect::SunLight(Default::default()),
    ]
    .map(|effect| VisualModifier::new(ModifierEffect::scene_3d(effect)))
    .into()
}

pub fn repeat_strategy_for_import(info: &MediaInfo) -> RepeatStrategy {
    if info.visual_kind == Some(VisualMediaKind::Gif) {
        RepeatStrategy::Repeat
    } else {
        RepeatStrategy::Hold
    }
}

#[derive(Clone)]
struct VttCue {
    start: Time,
    end: Time,
    text: String,
}

fn parse_vtt(path: &Path) -> Result<Vec<VttCue>, String> {
    let contents = fs::read_to_string(path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    let contents = contents
        .trim_start_matches('\u{feff}')
        .replace("\r\n", "\n");
    let mut lines = contents.lines().peekable();
    if lines
        .peek()
        .is_some_and(|line| line.trim_start().starts_with("WEBVTT"))
    {
        lines.next();
    }

    let mut cues = Vec::new();
    while let Some(line) = lines.next() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if matches!(line, "NOTE" | "STYLE" | "REGION") || line.starts_with("NOTE ") {
            skip_vtt_block(&mut lines);
            continue;
        }

        let timing = if line.contains("-->") {
            line
        } else {
            match lines.next().map(str::trim) {
                Some(next) if next.contains("-->") => next,
                _ => {
                    skip_vtt_block(&mut lines);
                    continue;
                }
            }
        };
        let Some((start, end)) = parse_vtt_timing(timing) else {
            skip_vtt_block(&mut lines);
            continue;
        };

        let mut text = Vec::new();
        while let Some(next) = lines.peek() {
            if next.trim().is_empty() {
                lines.next();
                break;
            }
            text.push(lines.next().unwrap_or_default());
        }
        let text = text.join("\n").trim().to_string();
        if start < end && !text.is_empty() {
            cues.push(VttCue { start, end, text });
        }
    }
    Ok(cues)
}

fn skip_vtt_block<'a>(lines: &mut std::iter::Peekable<std::str::Lines<'a>>) {
    while let Some(line) = lines.peek() {
        if line.trim().is_empty() {
            lines.next();
            break;
        }
        lines.next();
    }
}

fn parse_vtt_timing(line: &str) -> Option<(Time, Time)> {
    let (start, rest) = line.split_once("-->")?;
    let end = rest.split_whitespace().next()?;
    Some((
        parse_vtt_timestamp(start.trim())?,
        parse_vtt_timestamp(end)?,
    ))
}

fn parse_vtt_timestamp(value: &str) -> Option<Time> {
    let (whole, fraction) = value
        .split_once('.')
        .or_else(|| value.split_once(','))
        .unwrap_or((value, ""));
    let parts = whole.split(':').collect::<Vec<_>>();
    let (hours, minutes, seconds): (u64, u64, u64) = match parts.as_slice() {
        [minutes, seconds] => (0_u64, minutes.parse().ok()?, seconds.parse().ok()?),
        [hours, minutes, seconds] => (
            hours.parse().ok()?,
            minutes.parse().ok()?,
            seconds.parse().ok()?,
        ),
        _ => return None,
    };
    if minutes >= 60 || seconds >= 60 {
        return None;
    }
    let mut fraction_nanos = 0_u64;
    let mut scale = 100_000_000_u64;
    for digit in fraction.chars().take(9) {
        fraction_nanos = fraction_nanos.saturating_add(digit.to_digit(10)? as u64 * scale);
        scale /= 10;
    }
    let seconds_total = hours
        .saturating_mul(3600)
        .saturating_add(minutes.saturating_mul(60))
        .saturating_add(seconds);
    Some(Time::from_nanos(
        seconds_total
            .saturating_mul(1_000_000_000)
            .saturating_add(fraction_nanos),
    ))
}

fn remux(input: &Path, output: &Path) -> Result<(), String> {
    let mut input_context = format::input(input)
        .map_err(|error| format!("could not open {}: {error}", input.display()))?;
    let mut output_context = format::output(output)
        .map_err(|error| format!("could not create {}: {error}", output.display()))?;
    let stream_count = input_context.nb_streams() as usize;
    let mut stream_mapping = vec![-1; stream_count];
    let mut input_time_bases = vec![Rational(0, 1); stream_count];
    let mut output_index = 0;

    for (input_index, input_stream) in input_context.streams().enumerate() {
        let medium = input_stream.parameters().medium();
        if !matches!(
            medium,
            media::Type::Audio | media::Type::Video | media::Type::Subtitle
        ) {
            continue;
        }

        stream_mapping[input_index] = output_index;
        input_time_bases[input_index] = input_stream.time_base();
        output_index += 1;

        let mut output_stream = output_context
            .add_stream(encoder::find(codec::Id::None))
            .map_err(|error| format!("could not add output stream: {error}"))?;
        output_stream.set_parameters(input_stream.parameters());
        unsafe {
            (*output_stream.parameters().as_mut_ptr()).codec_tag = 0;
        }
    }

    output_context.set_metadata(input_context.metadata().to_owned());
    output_context
        .write_header()
        .map_err(|error| format!("could not write {} header: {error}", output.display()))?;

    for (stream, mut packet) in input_context.packets() {
        let input_index = stream.index();
        let output_index = stream_mapping[input_index];
        if output_index < 0 {
            continue;
        }
        let output_stream = output_context
            .stream(output_index as usize)
            .ok_or_else(|| "remux output stream disappeared".to_string())?;
        packet.rescale_ts(input_time_bases[input_index], output_stream.time_base());
        packet.set_position(-1);
        packet.set_stream(output_index as usize);
        packet
            .write_interleaved(&mut output_context)
            .map_err(|error| format!("could not write {} packet: {error}", output.display()))?;
    }

    output_context
        .write_trailer()
        .map_err(|error| format!("could not write {} trailer: {error}", output.display()))
}

fn remux_output_path(input: &Path) -> PathBuf {
    let parent = input.parent().unwrap_or_else(|| Path::new(""));
    let stem = input
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("remuxed");
    let direct = parent.join(format!("{stem}.mp4"));
    if !direct.exists() {
        return direct;
    }

    for index in 1.. {
        let output = parent.join(format!("{stem}-remuxed-{index}.mp4"));
        if !output.exists() {
            return output;
        }
    }
    unreachable!()
}

fn rational_as_f64(value: Rational) -> f64 {
    if value.denominator() == 0 {
        0.0
    } else {
        value.numerator() as f64 / value.denominator() as f64
    }
}

fn audio_extension(extension: &str) -> bool {
    [
        "aac", "aiff", "aif", "alac", "flac", "m4a", "mp3", "ogg", "opus", "wav",
    ]
    .iter()
    .any(|candidate| extension.eq_ignore_ascii_case(candidate))
}

fn image_extension(extension: &str) -> bool {
    matches!(
        extension.to_ascii_lowercase().as_str(),
        "jpg" | "jpeg" | "png" | "webp" | "avif"
    )
}
