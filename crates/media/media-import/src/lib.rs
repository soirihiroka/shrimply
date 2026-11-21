use hashbrown::HashMap;
mod av;
mod staging;
pub use staging::{StagedSource, stage_source, stage_source_in};
#[cfg(test)]
mod portable_tests;
#[cfg(test)]
mod sanim_tests;
pub use av::{remux_to_mp4, video_dimensions};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use glam::UVec2;
use shrimply_math_core::Fraction;
use shrimply_resource_pipeline::{JobContext, Pipeline, Processor, Subscription};

use shrimply_project_document::project::{
    Asset, AssetSnapshot, AudioItem, CanvasSize, CaptionItem, LayerVisibility, LayeredImageItem,
    Project, RepeatStrategy, ResolvedTransform, Time, Transform, VideoItem, VideoItemContent,
    VisualModifier, default_playback_speed,
};
use shrimply_project_document::timeline_search;
use shrimply_property_model::timeline_value::*;
use shrimply_visual_modifiers::{ModifierEffect, scene_3d::Scene3dModifierEffect};

use shrimply_timeline_edit::{ItemKey, TrackKind, insert_sorted, next_group_id};

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
        if matches!(
            file_kind(&key.path),
            Some(FileKind::Visual(
                VisualMediaKind::Manim | VisualMediaKind::Blender
            ))
        ) {
            shrimply_trust_core::approve_import(vec![key.path.clone()])?;
        }
        if let Some(info) = INSPECTION_CACHE
            .get_or_init(Mutex::default)
            .lock()
            .expect("media inspection cache lock poisoned")
            .get(&key)
            .filter(|info| {
                file_kind(&key.path) != Some(FileKind::Visual(VisualMediaKind::Sanim))
                    && info.snapshot.ensure_current().is_ok()
            })
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
    Visual(VisualMediaKind),
    Audio,
    Vtt,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VisualMediaKind {
    Video,
    Image,
    AnimatedImage,
    Gif,
    Svg,
    Pdf,
    Manim,
    Sanim,
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
    pub video_sizes: Vec<UVec2>,
    pub video_fps: Option<Fraction>,
    pub layered_image_layers: Vec<LayerVisibility>,
    pub caption_cues: Vec<VttCue>,
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

fn media_extension(path: &Path) -> Option<&str> {
    let extension = path.extension()?.to_str()?;
    if extension.eq_ignore_ascii_case("part") {
        Path::new(path.file_stem()?).extension()?.to_str()
    } else {
        Some(extension)
    }
}

pub fn requires_mp4_remux(path: &Path) -> bool {
    media_extension(path)
        .is_some_and(|ext| ext.eq_ignore_ascii_case("mkv") || ext.eq_ignore_ascii_case("webm"))
}

pub fn file_kind(path: &Path) -> Option<FileKind> {
    use VisualMediaKind::*;
    Some(match media_extension(path)?.to_ascii_lowercase().as_str() {
        "mp4" | "mov" | "mkv" | "webm" => FileKind::Visual(Video),
        "gif" => FileKind::Visual(Gif),
        "svg" => FileKind::Visual(Svg),
        "pdf" => FileKind::Visual(Pdf),
        "py" => FileKind::Visual(Manim),
        "sanim" => FileKind::Visual(Sanim),
        "blend" if shrimply_blender_core::SUPPORTED => FileKind::Visual(Blender),
        "psd" | "kra" => FileKind::Visual(LayeredImage),
        "obj" | "glb" => FileKind::Visual(Obj),
        "ply" => FileKind::Visual(Gaussian),
        "vtt" => FileKind::Vtt,
        "jpg" | "jpeg" | "png" | "webp" | "avif" | "heif" | "heic" | "jxl" | "tif" | "tiff"
        | "bmp" => FileKind::Visual(Image),
        "aac" | "aiff" | "aif" | "alac" | "flac" | "m4a" | "mp3" | "ogg" | "opus" | "wav" => {
            FileKind::Audio
        }
        _ => return None,
    })
}

/// File-backed pictures that an interchange document can reference without running a generator.
pub fn image_kind(path: &Path) -> Option<VisualMediaKind> {
    use VisualMediaKind::*;
    match file_kind(path)? {
        FileKind::Visual(kind @ (Image | Gif | Svg | Pdf | LayeredImage)) => Some(kind),
        _ => None,
    }
}

/// Keep the source type when an interchange document references an offline image.
pub fn image_reference(
    path: &Path,
    canvas_size: CanvasSize,
    duration: Time,
) -> Option<(VideoItemContent, Result<UVec2, String>)> {
    let kind = image_kind(path)?;
    Some(match inspect(path.to_owned(), canvas_size, duration) {
        Ok(info) => (video_content_for_import(&info), Ok(info.video_sizes[0])),
        Err(error) => (kind.content(&[]), Err(error)),
    })
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
    if file_kind == FileKind::Visual(VisualMediaKind::Sanim) {
        let parent = path.parent().unwrap_or(Path::new("."));
        let entry = path
            .file_name()
            .and_then(|v| v.to_str())
            .ok_or("Sanim filename must be UTF-8")?;
        let sources = sanim_lang::FileSources::new(parent)?;
        let mut engine = sanim_lang::Engine::default();
        engine.viewport.width = canvas_size.width.max(1);
        engine.viewport.height = canvas_size.height.max(1);
        let report = engine
            .duration(entry, &sources, &std::collections::BTreeMap::new())
            .or_else(|error| {
                if error.code != "S138" {
                    return Err(error);
                }
                engine.available_duration = Some(default_visual_duration.seconds);
                engine.duration(entry, &sources, &std::collections::BTreeMap::new())
            })
            .map_err(|e| e.to_string())?;
        snapshot.verify_current()?;
        let duration = Time {
            seconds: if report.indefinite {
                report.duration.max(default_visual_duration.seconds)
            } else if report.duration > Fraction::from(0) {
                report.duration
            } else {
                default_visual_duration.seconds
            },
        };
        shrimply_project_document::source_events::publish_import(
            path,
            canvas_size,
            shrimply_project_document::source_events::Events {
                duration,
                index: report.event_index.clone(),
            },
        );
        return Ok(MediaInfo {
            source,
            snapshot,
            duration,
            visual_kind: Some(VisualMediaKind::Sanim),
            video_streams: 1,
            video_sizes: vec![UVec2::new(
                canvas_size.width.max(1),
                canvas_size.height.max(1),
            )],
            video_fps: Some(report.viewport.rate),
            audio_streams: 0,
            layered_image_layers: Vec::new(),
            caption_cues: Vec::new(),
        });
    }
    if file_kind == FileKind::Vtt {
        let caption_cues = parse_vtt_contents(&snapshot.read_to_string()?);
        let duration = caption_cues
            .iter()
            .map(|cue| cue.end)
            .max()
            .ok_or_else(|| format!("{} has no VTT cues", path.display()))?;
        snapshot.verify_current()?;
        return Ok(MediaInfo {
            source,
            snapshot,
            duration,
            video_streams: 0,
            audio_streams: 0,
            visual_kind: None,
            video_sizes: Vec::new(),
            video_fps: None,
            layered_image_layers: Vec::new(),
            caption_cues,
        });
    }
    if let FileKind::Visual(kind @ (VisualMediaKind::Manim | VisualMediaKind::Blender)) = file_kind
    {
        shrimply_trust_core::require(path)?;
        let duration = if kind == VisualMediaKind::Blender {
            Time {
                seconds: shrimply_blender_core::file_duration(path)?,
            }
        } else {
            default_visual_duration
        };
        snapshot.verify_current()?;
        return Ok(MediaInfo {
            source,
            snapshot,
            duration,
            visual_kind: Some(kind),
            video_streams: 1,
            video_sizes: vec![UVec2::new(
                canvas_size.width.max(1),
                canvas_size.height.max(1),
            )],
            video_fps: None,
            audio_streams: 0,
            layered_image_layers: Vec::new(),
            caption_cues: Vec::new(),
        });
    }
    if let FileKind::Visual(
        kind @ (VisualMediaKind::Image | VisualMediaKind::Svg | VisualMediaKind::Gif),
    ) = file_kind
    {
        use shrimply_drawing_skia::media::{self, animation::Animation};
        let bytes = snapshot.read_bounded(media::MAX_SOURCE_BYTES)?;
        let (visual_kind, [width, height], duration, video_fps) =
            if let Some(mut animation) = Animation::probe(&bytes)? {
                let actual = if animation.is_gif() {
                    VisualMediaKind::Gif
                } else {
                    VisualMediaKind::AnimatedImage
                };
                if !matches!(
                    (kind, actual),
                    (VisualMediaKind::Gif, VisualMediaKind::Gif)
                        | (VisualMediaKind::Image, VisualMediaKind::AnimatedImage)
                ) {
                    return Err(format!(
                        "{} contents do not match its image type",
                        path.display()
                    ));
                }
                animation.frame_at(Time::ZERO)?;
                (
                    actual,
                    animation.dimensions(),
                    animation.duration,
                    Some(animation.average_frame_rate()),
                )
            } else {
                let prepared = media::prepare(Arc::new(bytes))?;
                if !matches!(
                    (kind, prepared.kind()),
                    (VisualMediaKind::Image, media::Kind::Bitmap)
                        | (VisualMediaKind::Svg, media::Kind::Svg)
                ) {
                    return Err(format!(
                        "{} contents do not match its image type",
                        path.display()
                    ));
                }
                (kind, prepared.dimensions(), default_visual_duration, None)
            };
        snapshot.verify_current()?;
        return Ok(MediaInfo {
            source,
            snapshot,
            duration,
            visual_kind: Some(visual_kind),
            video_streams: 1,
            video_sizes: vec![UVec2::new(width, height)],
            video_fps,
            audio_streams: 0,
            layered_image_layers: Vec::new(),
            caption_cues: Vec::new(),
        });
    }
    if file_kind == FileKind::Visual(VisualMediaKind::Pdf) {
        let pages = shrimply_pdf::page_sizes(snapshot.read()?)?;
        let first = pages
            .first()
            .expect("PDF inspection requires at least one page");
        snapshot.verify_current()?;
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
            caption_cues: Vec::new(),
        });
    }
    if file_kind == FileKind::Visual(VisualMediaKind::LayeredImage) {
        let image = shrimply_layered_image_loader::load(source.clone())?;
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
            caption_cues: Vec::new(),
        });
    }
    if matches!(
        file_kind,
        FileKind::Visual(VisualMediaKind::Obj | VisualMediaKind::Gaussian)
    ) {
        let visual_kind = if file_kind == FileKind::Visual(VisualMediaKind::Gaussian) {
            shrimply_3dgs_core::RenderSession::load(&source)
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
            caption_cues: Vec::new(),
        });
    }

    av::inspect(source, snapshot, file_kind)
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
        TrackKind::Caption if !info.caption_cues.is_empty() => {
            apply_vtt_cues_to_tracks(project, &info.caption_cues, track_indices, start)
        }
        TrackKind::Video => apply_video_to_tracks(project, info, track_indices, start, end),
        TrackKind::Audio => apply_audio_to_tracks(project, info, track_indices, start, end),
        TrackKind::Comment => {
            Err("visual and media files cannot be imported to comment tracks".to_string())
        }
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
    let cues = parse_vtt(path)?;
    if cues.is_empty() {
        return Err(format!("{} has no VTT cues", path.display()));
    }
    apply_vtt_cues_to_tracks(project, &cues, track_indices, start)
}

pub fn apply_vtt_cues_to_tracks(
    project: &mut Project,
    cues: &[VttCue],
    track_indices: &[usize],
    start: Time,
) -> Result<ImportResult, String> {
    if cues.is_empty() {
        return Err("file has no VTT cues".into());
    }
    let step = project.frame_step();
    let start = start.max(Time::ZERO).snapped(step);
    let mut ranges = cues
        .iter()
        .map(|cue| {
            let cue_start = start.saturating_add(cue.start).snapped(step);
            let cue_end = start
                .saturating_add(cue.end)
                .snapped(step)
                .max(cue_start.saturating_add(step));
            (cue_start, cue_end, &cue.text)
        })
        .collect::<Vec<_>>();
    ranges.sort_by_key(|(start, end, _)| (*start, *end));
    if ranges.windows(2).any(|pair| pair[0].1 > pair[1].0) {
        return Err("VTT cues overlap on the project frame grid; a single caption track cannot contain overlapping cues".into());
    }

    for &track_index in track_indices {
        let Some(track) = project.caption_tracks.get(track_index) else {
            return Err(format!("caption track {} does not exist", track_index + 1));
        };
        for &(item_start, item_end, _) in &ranges {
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
        for &(item_start, item_end, text) in &ranges {
            let mut item = CaptionItem::new(item_start, item_end, text.clone());
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
            playback_fps: shrimply_project_document::project::native_playback_fps(),
            repeat_strategy: RepeatStrategy::Hold,
            stabilize_video: false,
            stabilization_method: Default::default(),
            stabilization_crop_ratio:
                shrimply_project_document::project::default_video_stabilization_crop_ratio(),
            stabilization_first_derivative_weight:
                shrimply_project_document::project::default_video_stabilization_first_derivative_weight(),
            stabilization_second_derivative_weight:
                shrimply_project_document::project::default_video_stabilization_second_derivative_weight(),
            stabilization_third_derivative_weight:
                shrimply_project_document::project::default_video_stabilization_third_derivative_weight(),
            mesh_flow_rows: shrimply_project_document::project::default_mesh_flow_rows(),
            mesh_flow_columns: shrimply_project_document::project::default_mesh_flow_columns(),
            mesh_flow_smoothing_radius:
                shrimply_project_document::project::default_mesh_flow_smoothing_radius(),
            mesh_flow_iterations: shrimply_project_document::project::default_mesh_flow_iterations(),
            mesh_flow_adaptive_weights: Default::default(),
            animation_time_offset: Time::ZERO,
            motion_blur: Default::default(),
            transform: transform.clone(),
            modifiers: modifiers_for_import(info),
            sample_method: TimelineValue::new_const(
                if matches!(info.visual_kind, Some(VisualMediaKind::LayeredImage)) {
                    shrimply_property_model::VideoSampleMethod::Nearest
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
    info.visual_kind
        .unwrap_or(VisualMediaKind::Video)
        .content(&info.layered_image_layers)
}

impl VisualMediaKind {
    fn content(self, layers: &[LayerVisibility]) -> VideoItemContent {
        match self {
            VisualMediaKind::Video => VideoItemContent::Media,
            VisualMediaKind::Image => VideoItemContent::Image,
            VisualMediaKind::Gif => VideoItemContent::Gif,
            VisualMediaKind::AnimatedImage => VideoItemContent::AnimatedImage,
            VisualMediaKind::Svg => VideoItemContent::Svg,
            VisualMediaKind::Pdf => VideoItemContent::Pdf(Box::default()),
            VisualMediaKind::Manim => VideoItemContent::Manim(Box::default()),
            VisualMediaKind::Sanim => VideoItemContent::Sanim(Box::default()),
            VisualMediaKind::Blender => VideoItemContent::Blender(Box::default()),
            VisualMediaKind::LayeredImage => {
                VideoItemContent::LayeredImage(Box::new(LayeredImageItem {
                    layers: layers.to_vec(),
                }))
            }
            VisualMediaKind::Obj => VideoItemContent::Obj(Box::default()),
            VisualMediaKind::Gaussian => VideoItemContent::Gaussian(Box::default()),
        }
    }
}

pub fn modifiers_for_import(info: &MediaInfo) -> Vec<VisualModifier> {
    if info.visual_kind != Some(VisualMediaKind::Obj) {
        return Vec::new();
    }
    [
        Scene3dModifierEffect::Object(Box::new(
            shrimply_visual_modifiers::scene_3d::Object3dModifier::with_file(info.source.clone()),
        )),
        Scene3dModifierEffect::SunLight(Default::default()),
    ]
    .map(|effect| VisualModifier::new(ModifierEffect::scene_3d(effect)))
    .into()
}

pub fn repeat_strategy_for_import(info: &MediaInfo) -> RepeatStrategy {
    if matches!(
        info.visual_kind,
        Some(VisualMediaKind::Gif | VisualMediaKind::AnimatedImage)
    ) {
        RepeatStrategy::Repeat
    } else {
        RepeatStrategy::Hold
    }
}

#[derive(Clone)]
pub struct VttCue {
    start: Time,
    end: Time,
    text: String,
}

fn parse_vtt(path: &Path) -> Result<Vec<VttCue>, String> {
    let contents = fs::read_to_string(path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    Ok(parse_vtt_contents(&contents))
}

fn parse_vtt_contents(contents: &str) -> Vec<VttCue> {
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
    cues
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

pub struct OwnedFile {
    path: PathBuf,
    keep: bool,
}

impl OwnedFile {
    pub fn new(path: PathBuf) -> Self {
        Self { path, keep: false }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn keep(mut self) {
        self.keep = true;
    }
}

impl Drop for OwnedFile {
    fn drop(&mut self) {
        if !self.keep
            && let Err(error) = fs::remove_file(&self.path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(path = %self.path.display(), %error, "Could not remove abandoned external import file");
        }
    }
}
