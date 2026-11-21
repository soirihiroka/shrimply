mod ass;
mod comments;
mod effects;
mod math;
mod streams;
mod xml;

use std::collections::{BTreeSet, HashMap, HashSet};
use std::error::Error;
use std::io;
use std::path::{Path, PathBuf};

use glam::Vec2;
use serde_json::Value;
use shrimply_math_core::Fraction;
use shrimply_project_document::{
    AudioGenerator, AudioItem, AudioSource, AudioSpeedMethod, AudioTrack, AudioWaveform,
    Background, BackgroundGenerator, CanvasSize, CaptionTrack, Color, FoldedSequence, Project,
    SequenceReference, SolidColor, Time, VideoItem, VideoItemContent, VisualTrack, WhiteNoise,
};
use shrimply_property_model::timeline_value::{TimelineExpression, TimelineValue};
use uuid::Uuid;

use xml::Element;

const COUNTER_BACKGROUND_CHANNEL: u8 = 0xd0;
const COUNTER_BEEP_FREQUENCY_HZ: f32 = 1_000.0;
const COUNTER_FONT_HEIGHT_PERCENT: u32 = 70;
const PERCENT: u32 = 100;

pub struct ImportResult {
    pub project: Value,
    pub warnings: Vec<String>,
}

pub fn from_file(path: impl AsRef<Path>) -> Result<ImportResult, Box<dyn Error + Send + Sync>> {
    let path = path.as_ref();
    let root = xml::parse(path)?;
    if root.name != "mlt" {
        return Err(invalid("not an MLT/Kdenlive document"));
    }

    let profile = root
        .children_named("profile")
        .next()
        .ok_or_else(|| invalid("missing MLT profile"))?;
    let fps = Fraction::new(
        positive_u64(profile.attribute("frame_rate_num"), "frame_rate_num")?,
        positive_u64(profile.attribute("frame_rate_den"), "frame_rate_den")?,
    );
    let canvas_size = CanvasSize {
        width: positive_u32(profile.attribute("width"), "profile width")?,
        height: positive_u32(profile.attribute("height"), "profile height")?,
    };
    let root_dir = root
        .attribute("root")
        .map(PathBuf::from)
        .or_else(|| path.parent().map(Path::to_path_buf))
        .unwrap_or_default();

    let index = root
        .children
        .iter()
        .filter_map(|node| node.attribute("id").map(|id| (id, node)))
        .collect::<HashMap<_, _>>();
    let main_bin = root
        .children_named("playlist")
        .find(|node| node.attribute("id") == Some("main_bin"))
        .ok_or_else(|| invalid("missing Kdenlive main bin"))?;
    let active_uuid = parse_uuid(
        main_bin
            .property("kdenlive:docproperties.activetimeline")
            .ok_or_else(|| invalid("missing active Kdenlive sequence"))?,
    )?;
    let active = sequence_by_uuid(&root, active_uuid)
        .ok_or_else(|| invalid("active Kdenlive sequence does not exist"))?;

    let mut converter = Converter {
        root_dir,
        fps,
        canvas_size,
        index,
        warnings: BTreeSet::new(),
        stream_catalogs: HashMap::new(),
    };
    let mut main = converter.convert_sequence(active)?;

    let mut reachable = HashSet::new();
    converter.collect_reachable(active, &mut reachable);
    reachable.remove(&active_uuid);
    let mut folded_sequences = Vec::with_capacity(reachable.len());
    for sequence_id in reachable {
        let tractor = sequence_by_uuid(&root, sequence_id)
            .ok_or_else(|| invalid("referenced Kdenlive sequence does not exist"))?;
        let sequence = converter.convert_sequence(tractor)?;
        folded_sequences.push(FoldedSequence {
            id: sequence_id,
            video_tracks: sequence.video_tracks,
            audio_tracks: sequence.audio_tracks,
        });
    }

    main.caption_tracks = converter.caption_tracks(active)?;
    let comment_tracks = converter.comment_tracks(active, main_bin)?;
    let project = Project {
        name: path
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("Imported Kdenlive project")
            .to_owned(),
        fps,
        canvas_size,
        caption_tracks: main.caption_tracks,
        comment_tracks,
        video_tracks: main.video_tracks,
        audio_tracks: main.audio_tracks,
        folded_sequences,
        ..Default::default()
    };

    Ok(ImportResult {
        project: serde_json::to_value(project)?,
        warnings: converter.warnings.into_iter().collect(),
    })
}

struct ConvertedSequence {
    video_tracks: Vec<VisualTrack>,
    audio_tracks: Vec<AudioTrack>,
    caption_tracks: Vec<CaptionTrack>,
}

struct Converter<'a> {
    stream_catalogs: HashMap<PathBuf, Option<Vec<shrimply_media_info::demux::StreamKind>>>,
    root_dir: PathBuf,
    fps: Fraction,
    canvas_size: CanvasSize,
    index: HashMap<&'a str, &'a Element>,
    warnings: BTreeSet<String>,
}

impl<'a> Converter<'a> {
    fn convert_sequence(
        &mut self,
        tractor: &Element,
    ) -> Result<ConvertedSequence, Box<dyn Error + Send + Sync>> {
        let mut video_tracks = Vec::new();
        let mut audio_tracks = Vec::new();

        for (position, outer_track) in tractor.children_named("track").enumerate() {
            let Some(target) = outer_track
                .attribute("producer")
                .and_then(|id| self.index.get(id).copied())
            else {
                continue;
            };
            if position == 0 || target.property("kdenlive:playlistid") == Some("black_track") {
                continue;
            }
            if target.property("kdenlive:playlistid") == Some("timeline_preview") {
                continue;
            }

            let hidden = outer_track.attribute("hide").unwrap_or_default();
            let is_audio = target.property("kdenlive:audio_track") == Some("1");
            let lanes = target
                .children_named("track")
                .filter_map(|track| track.attribute("producer"))
                .filter_map(|id| self.index.get(id).copied())
                .collect::<Vec<_>>();
            if lanes.is_empty() {
                continue;
            }

            let nonempty = lanes
                .iter()
                .filter(|lane| lane.children.iter().any(|node| node.name == "entry"))
                .count();
            if nonempty > 1 {
                self.warnings.insert(
                    "A Kdenlive track used both playlist lanes; its second lane was imported as an adjacent track."
                        .to_owned(),
                );
            }

            let lanes_to_import = if nonempty == 0 { 1 } else { lanes.len() };
            for lane in lanes.into_iter().take(lanes_to_import) {
                if nonempty != 0 && !lane.children.iter().any(|node| node.name == "entry") {
                    continue;
                }
                if is_audio {
                    audio_tracks.push(AudioTrack {
                        enabled: hidden != "audio" && hidden != "both",
                        items: self.audio_items(lane)?,
                        ..AudioTrack::default()
                    });
                } else {
                    video_tracks.push(VisualTrack {
                        enabled: hidden != "video" && hidden != "both",
                        items: self.video_items(lane)?,
                        ..VisualTrack::default()
                    });
                }
            }
        }

        Ok(ConvertedSequence {
            video_tracks,
            audio_tracks,
            caption_tracks: Vec::new(),
        })
    }

    fn video_items(
        &mut self,
        playlist: &Element,
    ) -> Result<Vec<VideoItem>, Box<dyn Error + Send + Sync>> {
        let mut cursor = 0_i64;
        let mut items = Vec::new();
        for node in &playlist.children {
            match node.name.as_str() {
                "blank" => cursor += element_duration(node, self.fps)?,
                "entry" => {
                    let duration = element_duration(node, self.fps)?;
                    let producer = self.entry_producer(node)?;
                    if !streams::disabled(producer, "vstream", "video_index") {
                        items.push(self.video_item(node, producer, cursor, duration)?);
                    }
                    cursor += duration;
                }
                _ => {}
            }
        }
        Ok(items)
    }

    fn audio_items(
        &mut self,
        playlist: &Element,
    ) -> Result<Vec<AudioItem>, Box<dyn Error + Send + Sync>> {
        let mut cursor = 0_i64;
        let mut items = Vec::new();
        for node in &playlist.children {
            match node.name.as_str() {
                "blank" => cursor += element_duration(node, self.fps)?,
                "entry" => {
                    let duration = element_duration(node, self.fps)?;
                    let producer = self.entry_producer(node)?;
                    if !streams::disabled(producer, "astream", "audio_index") {
                        items.extend(self.audio_item(node, producer, cursor, duration)?);
                    }
                    cursor += duration;
                }
                _ => {}
            }
        }
        Ok(items)
    }

    fn video_item(
        &mut self,
        entry: &Element,
        producer: &Element,
        start_frame: i64,
        duration_frames: i64,
    ) -> Result<VideoItem, Box<dyn Error + Send + Sync>> {
        let source = self.source(producer)?;
        let start = frame_time(start_frame, self.fps);
        let end = frame_time(start_frame + duration_frames, self.fps);
        let entry_in = entry_in(entry, self.fps)?;
        let source_frame = match source.reverse_origin_frame {
            Some(origin) => origin
                .checked_sub(entry_in)
                .ok_or_else(|| invalid("reverse clip source offset overflowed"))?,
            None => entry_in,
        };
        let mut item = VideoItem::background_item(self.canvas_size, start, end);
        if source.counter.is_some() {
            item.animation_time_offset = frame_time(entry_in, self.fps);
        }
        item.time_offset = source_time(source_frame, source.speed.abs(), self.fps);
        item.source_duration = source
            .source_duration
            .unwrap_or_else(|| source_time(duration_frames, source.speed.abs(), self.fps));
        item.playback_speed = source.speed;
        item.track_id = source.video_track_id;
        item.file = source.path.into();
        item.source_width = source.width;
        item.source_height = source.height;
        if matches!(&source.visual, VideoItemContent::LayeredImage(_)) {
            item.sample_method =
                TimelineValue::new_const(shrimply_property_model::VideoSampleMethod::Nearest);
        }
        item.content = source.visual;
        item.transform = shrimply_project_document::Transform::natural_size(
            self.canvas_size,
            source.width,
            source.height,
        );
        item.transform.rotation_degrees = TimelineValue::new_const(source.rotation_degrees);
        let source_size = Vec2::new(source.width.max(1) as f32, source.height.max(1) as f32);
        let oriented_size = if ((source.rotation_degrees / 90.0).round() as i32).rem_euclid(2) != 0
        {
            Vec2::new(source_size.y, source_size.x)
        } else {
            source_size
        };
        item.transform.scale = TimelineValue::new_const(Vec2::splat(math::fit_scale(
            Vec2::new(
                self.canvas_size.width.max(1) as f32,
                self.canvas_size.height.max(1) as f32,
            ),
            oriented_size,
        )));
        item.default_transform = Some(item.transform.clone());
        self.apply_visual_effects(entry, &mut item)?;
        Ok(item)
    }

    fn audio_item(
        &mut self,
        entry: &Element,
        producer: &Element,
        start_frame: i64,
        duration_frames: i64,
    ) -> Result<Vec<AudioItem>, Box<dyn Error + Send + Sync>> {
        let source = self.source(producer)?;
        let entry_in = entry_in(entry, self.fps)?;
        if let Some(counter) = source.counter {
            return self.counter_audio_items(
                entry,
                counter,
                start_frame,
                entry_in,
                duration_frames,
            );
        }
        let source_frame = match source.reverse_origin_frame {
            Some(origin) => origin
                .checked_sub(entry_in)
                .ok_or_else(|| invalid("reverse clip source offset overflowed"))?,
            None => entry_in,
        };
        let item = AudioItem::builder(
            frame_time(start_frame, self.fps),
            frame_time(start_frame + duration_frames, self.fps),
        )
        .time_offset(source_time(source_frame, source.speed.abs(), self.fps))
        .source_duration(
            source
                .source_duration
                .unwrap_or_else(|| source_time(duration_frames, source.speed.abs(), self.fps)),
        )
        .playback_speed(source.speed)
        .speed_method(if source.pitch_preserved {
            AudioSpeedMethod::PreservePitch
        } else {
            AudioSpeedMethod::Naive
        })
        .track_id(source.audio_track_id)
        .file(source.path)
        .source(source.audio)
        .build();
        Ok(vec![self.apply_audio_effects(entry, item)?])
    }

    fn counter_audio_items(
        &mut self,
        entry: &Element,
        counter: CounterGenerator,
        start_frame: i64,
        entry_in: i64,
        duration_frames: i64,
    ) -> Result<Vec<AudioItem>, Box<dyn Error + Send + Sync>> {
        let mut source_frames = Vec::new();
        match counter.sound {
            CounterSound::Silent => {}
            CounterSound::TwoPop => {
                let nominal_fps = math::ceil_positive_fraction(self.fps)
                    .ok_or_else(|| invalid("counter frame rate is not positive"))?;
                let source_frame = counter
                    .length
                    .checked_sub(1)
                    .and_then(|out| out.checked_sub(nominal_fps.checked_mul(2)?))
                    .ok_or_else(|| invalid("counter 2-pop position overflowed"))?;
                let entry_out = entry_in
                    .checked_add(duration_frames)
                    .ok_or_else(|| invalid("counter clip duration overflowed"))?;
                if source_frame >= entry_in && source_frame < entry_out {
                    source_frames.push(source_frame);
                }
            }
            CounterSound::FrameZero => {
                let entry_out = entry_in
                    .checked_add(duration_frames)
                    .ok_or_else(|| invalid("counter clip duration overflowed"))?;
                for source_frame in entry_in..entry_out {
                    let Some(position) = counter_position(counter, source_frame) else {
                        continue;
                    };
                    if shrimply_math_core::smpte_timecode(position, self.fps, counter.drop_frame)
                        .is_some_and(|timecode| timecode.frames == 0)
                    {
                        source_frames.push(source_frame);
                    }
                }
            }
        }

        let mut items = Vec::with_capacity(source_frames.len());
        for source_frame in source_frames {
            let relative_frame = source_frame
                .checked_sub(entry_in)
                .ok_or_else(|| invalid("counter beep offset overflowed"))?;
            let item_start = start_frame
                .checked_add(relative_frame)
                .ok_or_else(|| invalid("counter beep position overflowed"))?;
            let item_end = item_start
                .checked_add(1)
                .ok_or_else(|| invalid("counter beep duration overflowed"))?;
            let item = AudioItem::builder(
                frame_time(item_start, self.fps),
                frame_time(item_end, self.fps),
            )
            .source_duration(frame_time(1, self.fps))
            .speed_method(AudioSpeedMethod::Naive)
            .source(AudioSource::Generator(Box::new(AudioGenerator {
                frequency_hz: TimelineValue::new_const(COUNTER_BEEP_FREQUENCY_HZ),
                ..AudioGenerator::default()
            })))
            .build();
            items.push(self.apply_audio_effects(entry, item)?);
        }
        Ok(items)
    }

    fn source(&mut self, producer: &Element) -> Result<Source, Box<dyn Error + Send + Sync>> {
        if producer.name == "tractor" {
            let sequence_id = parse_uuid(
                producer
                    .property("kdenlive:uuid")
                    .ok_or_else(|| invalid("nested sequence has no UUID"))?,
            )?;
            let reference = SequenceReference {
                sequence_id,
                instance_id: Uuid::new_v4(),
            };
            return Ok(Source {
                path: PathBuf::new(),
                visual: VideoItemContent::FoldedSequence(reference),
                audio: AudioSource::FoldedSequence(reference),
                width: self.canvas_size.width,
                height: self.canvas_size.height,
                speed: Fraction::from(1_u64),
                pitch_preserved: true,
                video_track_id: 0,
                audio_track_id: 0,
                source_duration: Some(frame_time(sequence_duration(producer, self.fps)?, self.fps)),
                reverse_origin_frame: None,
                rotation_degrees: 0.0,
                counter: None,
            });
        }

        let service = producer.property("mlt_service").unwrap_or_default();
        if matches!(service, "color" | "colour") {
            let color = parse_mlt_color(
                producer
                    .property("resource")
                    .ok_or_else(|| invalid("color producer has no color"))?,
            )?;
            return Ok(Source {
                path: PathBuf::new(),
                visual: VideoItemContent::Background(Box::new(Background {
                    generator: BackgroundGenerator::SolidColor(Box::new(SolidColor {
                        color: TimelineValue::new_const(color),
                    })),
                })),
                audio: AudioSource::Media,
                width: self.canvas_size.width,
                height: self.canvas_size.height,
                speed: Fraction::from(1_u64),
                pitch_preserved: true,
                video_track_id: 0,
                audio_track_id: 0,
                source_duration: None,
                reverse_origin_frame: None,
                rotation_degrees: 0.0,
                counter: None,
            });
        }

        let resource = producer
            .property("kdenlive:originalurl")
            .or_else(|| producer.property("warp_resource"))
            .or_else(|| producer.property("resource"));
        let generator_path = resource.map_or_else(PathBuf::new, |resource| {
            resolve_path(
                &self.root_dir,
                resource
                    .strip_prefix("xml:")
                    .or_else(|| resource.strip_prefix("consumer:"))
                    .unwrap_or(resource),
            )
        });
        if let Some(generator) = generator_source(producer, &generator_path, self.fps)? {
            let (visual, audio, warning, counter) = match generator {
                GeneratorSource::ColorBars => (
                    VideoItemContent::Background(Box::new(Background {
                        generator: BackgroundGenerator::TestPattern,
                    })),
                    AudioSource::Media,
                    "Kdenlive Color Bars were approximated with the Shrimply test pattern.",
                    None,
                ),
                GeneratorSource::WhiteNoise => (
                    VideoItemContent::Background(Box::new(Background {
                        generator: BackgroundGenerator::WhiteNoise(Box::<WhiteNoise>::default()),
                    })),
                    AudioSource::Generator(Box::new(AudioGenerator {
                        waveform: AudioWaveform::WhiteNoise,
                        ..AudioGenerator::default()
                    })),
                    "Kdenlive White Noise was approximated with Shrimply video and audio generators.",
                    None,
                ),
                GeneratorSource::Counter(counter) => (
                    counter_visual(counter, self.canvas_size, self.fps)?,
                    AudioSource::Generator(Box::new(AudioGenerator {
                        frequency_hz: TimelineValue::new_const(COUNTER_BEEP_FREQUENCY_HZ),
                        ..AudioGenerator::default()
                    })),
                    if counter.clock_background {
                        "Kdenlive Counter was converted to animated Shrimply text; its typeface was approximated and film-leader graphics were omitted."
                    } else {
                        "Kdenlive Counter was converted to animated Shrimply text; its typeface was approximated."
                    },
                    Some(counter),
                ),
            };
            self.warnings.insert(warning.to_owned());
            return Ok(Source {
                path: PathBuf::new(),
                visual,
                audio,
                width: self.canvas_size.width,
                height: self.canvas_size.height,
                speed: Fraction::from(1_u64),
                pitch_preserved: true,
                video_track_id: 0,
                audio_track_id: 0,
                source_duration: counter.map(|counter| frame_time(counter.length, self.fps)),
                reverse_origin_frame: None,
                rotation_degrees: 0.0,
                counter,
            });
        }
        let resource = resource.ok_or_else(|| invalid("media producer has no resource"))?;
        let path = resolve_path(&self.root_dir, resource);
        let speed = if service == "timewarp" {
            parse_fraction(producer.property("warp_speed").unwrap_or("1"))?
        } else {
            Fraction::from(1_u64)
        };
        let pitch_preserved = producer.property("warp_pitch") == Some("1");
        let length = producer
            .property("length")
            .map(|value| math::parse_frame(value, self.fps).map_err(invalid))
            .transpose()?;
        let producer_out = producer
            .attribute("out")
            .map(|value| math::parse_frame(value, self.fps).map_err(invalid))
            .transpose()?;
        let source_duration_frames = if let Some(length) = length {
            Some(length)
        } else if let Some(out) = producer_out {
            Some(
                out.checked_add(1)
                    .ok_or_else(|| invalid("producer duration overflowed"))?,
            )
        } else {
            None
        };
        let source_duration =
            source_duration_frames.map(|frames| source_time(frames, speed.abs(), self.fps));
        let reverse_origin_frame = if speed < Fraction::from(0_u64) {
            let producer_in = producer
                .attribute("in")
                .map(|value| math::parse_frame(value, self.fps).map_err(invalid))
                .transpose()?
                .unwrap_or(0);
            let producer_out = producer_out
                .or_else(|| length.and_then(|length| length.checked_sub(1)))
                .ok_or_else(|| invalid("reverse timewarp producer has no out or length"))?;
            Some(
                producer_in
                    .checked_add(producer_out)
                    .ok_or_else(|| invalid("reverse clip origin overflowed"))?,
            )
        } else {
            None
        };
        let (video_track_id, video_index, audio_track_id) =
            if shrimply_media_import::image_kind(&path).is_some() {
                (0, 0, 0)
            } else {
                streams::select(producer, &path, &mut self.stream_catalogs).map_err(invalid)?
            };
        let rotation_degrees = if producer.property("autorotate") == Some("0")
            || producer.property("disable_exif") == Some("1")
        {
            0.0
        } else {
            producer
                .property(&format!("meta.media.{video_index}.codec.rotate"))
                .and_then(|value| value.parse().ok())
                .unwrap_or(0.0)
        };
        let producer_width = property_u32(
            producer,
            &[
                "meta.media.width",
                &format!("meta.media.{video_index}.codec.width"),
            ],
        )
        .filter(|&width| width > 0);
        let producer_height = property_u32(
            producer,
            &[
                "meta.media.height",
                &format!("meta.media.{video_index}.codec.height"),
            ],
        )
        .filter(|&height| height > 0);
        let (visual, size) = match shrimply_media_import::image_reference(
            &path,
            self.canvas_size,
            source_duration.unwrap_or(Time::ZERO),
        ) {
            Some((
                content @ (VideoItemContent::Pdf(_) | VideoItemContent::LayeredImage(_)),
                size,
            )) => (content, Some(size.map_err(invalid)?)),
            Some((content, size)) => (content, size.ok()),
            None if producer_width.is_none() || producer_height.is_none() => (
                VideoItemContent::Media,
                shrimply_media_import::video_dimensions(&path, video_track_id),
            ),
            None => (VideoItemContent::Media, None),
        };
        // PDF producers use page geometry; other producers may override source dimensions.
        let (width, height) = if let (VideoItemContent::Pdf(_), Some(size)) = (&visual, size) {
            (size.x, size.y)
        } else {
            (
                producer_width
                    .or_else(|| size.map(|s| s.x))
                    .unwrap_or(self.canvas_size.width),
                producer_height
                    .or_else(|| size.map(|s| s.y))
                    .unwrap_or(self.canvas_size.height),
            )
        };
        Ok(Source {
            path,
            visual,
            audio: AudioSource::Media,
            width,
            height,
            speed,
            pitch_preserved,
            video_track_id,
            audio_track_id,
            source_duration,
            reverse_origin_frame,
            rotation_degrees,
            counter: None,
        })
    }

    fn entry_producer(&self, entry: &Element) -> Result<&'a Element, Box<dyn Error + Send + Sync>> {
        entry
            .attribute("producer")
            .and_then(|id| self.index.get(id).copied())
            .ok_or_else(|| invalid("timeline entry references a missing producer"))
    }

    fn collect_reachable(&self, tractor: &Element, sequences: &mut HashSet<Uuid>) {
        let Some(uuid) = tractor
            .property("kdenlive:uuid")
            .and_then(|value| parse_uuid(value).ok())
        else {
            return;
        };
        if !sequences.insert(uuid) {
            return;
        }
        for lane in tractor
            .children_named("track")
            .filter_map(|track| track.attribute("producer"))
            .filter_map(|id| self.index.get(id).copied())
            .flat_map(|track| track.children_named("track"))
            .filter_map(|track| track.attribute("producer"))
            .filter_map(|id| self.index.get(id).copied())
        {
            for nested in lane
                .children_named("entry")
                .filter_map(|entry| entry.attribute("producer"))
                .filter_map(|id| self.index.get(id).copied())
                .filter(|producer| producer.name == "tractor")
            {
                self.collect_reachable(nested, sequences);
            }
        }
    }

    fn caption_tracks(
        &mut self,
        tractor: &Element,
    ) -> Result<Vec<CaptionTrack>, Box<dyn Error + Send + Sync>> {
        let Some(list) = tractor
            .property("kdenlive:sequenceproperties.subtitlesList")
            .or_else(|| tractor.property("kdenlive:subtitlesList"))
        else {
            return Ok(Vec::new());
        };
        let disabled = tractor.property("kdenlive:sequenceproperties.hidesubtitle") == Some("1")
            || tractor.children_named("filter").any(|filter| {
                matches!(
                    filter.property("mlt_service"),
                    Some("avfilter.ass" | "avfilter.subtitles")
                ) && filter.property("disable") == Some("1")
            });
        let value: Value = serde_json::from_str(list)?;
        let files = value
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.get("file").and_then(Value::as_str));
        let mut tracks = Vec::new();
        for file in files {
            tracks.push(CaptionTrack {
                enabled: !disabled,
                items: ass::read(&resolve_path(&self.root_dir, file), self.fps).map_err(invalid)?,
                ..CaptionTrack::default()
            });
            self.warnings.insert(
                "ASS subtitle fonts, outlines, and positioning were approximated with Shrimply caption styling."
                    .to_owned(),
            );
        }
        Ok(tracks)
    }
}

struct Source {
    path: PathBuf,
    visual: VideoItemContent,
    audio: AudioSource,
    width: u32,
    height: u32,
    speed: Fraction,
    pitch_preserved: bool,
    video_track_id: u32,
    audio_track_id: u32,
    source_duration: Option<Time>,
    reverse_origin_frame: Option<i64>,
    rotation_degrees: f32,
    counter: Option<CounterGenerator>,
}

#[derive(Clone, Copy)]
enum GeneratorSource {
    ColorBars,
    WhiteNoise,
    Counter(CounterGenerator),
}

#[derive(Clone, Copy)]
struct CounterGenerator {
    length: i64,
    direction: CounterDirection,
    style: CounterStyle,
    sound: CounterSound,
    clock_background: bool,
    drop_frame: bool,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum CounterDirection {
    Down,
    Up,
}

#[derive(Clone, Copy)]
enum CounterStyle {
    Seconds,
    SecondsPlusOne,
    Frames,
    Timecode,
    Clock,
}

#[derive(Clone, Copy)]
enum CounterSound {
    Silent,
    TwoPop,
    FrameZero,
}

fn generator_source(
    producer: &Element,
    path: &Path,
    fps: Fraction,
) -> Result<Option<GeneratorSource>, Box<dyn Error + Send + Sync>> {
    let service = producer.property("mlt_service").unwrap_or_default();
    if matches!(service, "xml" | "consumer")
        && path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.eq_ignore_ascii_case("mlt"))
    {
        let root = xml::parse(path).map_err(invalid)?;
        let mut generators = root
            .children
            .iter()
            .filter(|node| matches!(node.name.as_str(), "producer" | "chain"))
            .filter(|node| {
                node.property("mlt_service")
                    .or_else(|| node.attribute("mlt_service"))
                    .is_some()
            });
        let Some(generator) = generators.next() else {
            return Ok(None);
        };
        if generators.next().is_some() {
            return Ok(None);
        }
        return classify_generator(generator, fps);
    }
    classify_generator(producer, fps)
}

fn classify_generator(
    producer: &Element,
    fps: Fraction,
) -> Result<Option<GeneratorSource>, Box<dyn Error + Send + Sync>> {
    let service = producer
        .property("mlt_service")
        .or_else(|| producer.attribute("mlt_service"))
        .unwrap_or_default();
    Ok(match service {
        "frei0r.test_pat_B" => Some(GeneratorSource::ColorBars),
        "noise" => Some(GeneratorSource::WhiteNoise),
        "count" => {
            let length = producer
                .property("length")
                .map(|value| math::parse_frame(value, fps).map_err(invalid))
                .transpose()?
                .or_else(|| {
                    producer
                        .attribute("out")
                        .and_then(|value| math::parse_frame(value, fps).ok())
                        .and_then(|out| out.checked_add(1))
                })
                .ok_or_else(|| invalid("counter generator has no duration"))?;
            if length <= 0 {
                return Err(invalid("counter generator duration is not positive"));
            }
            Some(GeneratorSource::Counter(CounterGenerator {
                length,
                direction: if producer.property("direction").unwrap_or("down") == "down" {
                    CounterDirection::Down
                } else {
                    CounterDirection::Up
                },
                style: match producer.property("style").unwrap_or("seconds+1") {
                    "frames" => CounterStyle::Frames,
                    "timecode" => CounterStyle::Timecode,
                    "clock" => CounterStyle::Clock,
                    "seconds+1" => CounterStyle::SecondsPlusOne,
                    _ => CounterStyle::Seconds,
                },
                sound: match producer.property("sound").unwrap_or("silent") {
                    "2pop" => CounterSound::TwoPop,
                    "frame0" => CounterSound::FrameZero,
                    _ => CounterSound::Silent,
                },
                clock_background: producer.property("background").unwrap_or("clock") == "clock",
                drop_frame: producer
                    .property("drop")
                    .and_then(|value| value.parse::<i64>().ok())
                    .is_some_and(|value| value != 0),
            }))
        }
        _ => None,
    })
}

fn counter_visual(
    counter: CounterGenerator,
    canvas_size: CanvasSize,
    fps: Fraction,
) -> Result<VideoItemContent, Box<dyn Error + Send + Sync>> {
    let initial_position = counter_position(counter, 0)
        .ok_or_else(|| invalid("counter initial position overflowed"))?;
    let mut item = VideoItem::text_item(canvas_size, Time::ZERO, Time::ZERO);
    let VideoItemContent::Text(text) = &mut item.content else {
        unreachable!("text item constructor must create text content");
    };
    text.text = TimelineValue::new_const(counter_text(counter, initial_position, fps)?);
    text.text.expression = Some(TimelineExpression {
        id: Uuid::new_v4(),
        enabled: true,
        source: counter_expression(counter),
    });
    text.font_size = TimelineValue::new_const(
        canvas_size
            .height
            .saturating_mul(COUNTER_FONT_HEIGHT_PERCENT)
            .checked_div(PERCENT)
            .unwrap_or(0) as f32,
    );
    text.font_weight = TimelineValue::new_const(400.0);
    text.color = TimelineValue::new_const(Color::<u8>::BLACK);
    text.background_color = TimelineValue::new_const(Color::<u8>::from_rgba(
        COUNTER_BACKGROUND_CHANNEL,
        COUNTER_BACKGROUND_CHANNEL,
        COUNTER_BACKGROUND_CHANNEL,
        u8::MAX,
    ));
    text.background_padding = TimelineValue::new_const(Vec2::new(
        canvas_size.width as f32,
        canvas_size.height as f32,
    ));
    Ok(item.content)
}

fn counter_position(counter: CounterGenerator, source_frame: i64) -> Option<i64> {
    if counter.direction == CounterDirection::Down {
        counter.length.checked_sub(1)?.checked_sub(source_frame)
    } else {
        Some(source_frame)
    }
}

fn counter_text(
    counter: CounterGenerator,
    position: i64,
    fps: Fraction,
) -> Result<String, Box<dyn Error + Send + Sync>> {
    if matches!(counter.style, CounterStyle::Frames) {
        return Ok(position.to_string());
    }
    let timecode = shrimply_math_core::smpte_timecode(position, fps, counter.drop_frame)
        .ok_or_else(|| invalid("could not format counter timecode"))?;
    Ok(match counter.style {
        CounterStyle::Frames => unreachable!(),
        CounterStyle::Timecode => shrimply_math_core::format_smpte_timecode(timecode),
        CounterStyle::Clock => format!(
            "{:02}:{:02}:{:02}",
            timecode.hours, timecode.minutes, timecode.seconds
        ),
        CounterStyle::Seconds => timecode.seconds.to_string(),
        CounterStyle::SecondsPlusOne => (timecode.seconds + 1).to_string(),
    })
}

fn counter_expression(counter: CounterGenerator) -> String {
    let position = if counter.direction == CounterDirection::Down {
        format!("{} - int(time * fps)", counter.length - 1)
    } else {
        "int(time * fps)".to_owned()
    };
    let drop_frame = counter.drop_frame;
    let value = match counter.style {
        CounterStyle::Frames => "`${frame}`".to_owned(),
        CounterStyle::Timecode => format!("timecode(frame, fps, {drop_frame})"),
        CounterStyle::Clock => format!(
            "let parts = timecode(frame, fps, {drop_frame}).split(\":\");\n`${{parts[0]}}:${{parts[1]}}:${{parts[2].sub_string(0, 2)}}`"
        ),
        CounterStyle::Seconds => format!(
            "let parts = timecode(frame, fps, {drop_frame}).split(\":\");\nparts[2].sub_string(0, 2)"
        ),
        CounterStyle::SecondsPlusOne => format!(
            "let parts = timecode(frame, fps, {drop_frame}).split(\":\");\n`${{parse_int(parts[2].sub_string(0, 2)) + 1}}`"
        ),
    };
    format!("let frame = {position};\n{value}")
}

fn sequence_by_uuid(root: &Element, uuid: Uuid) -> Option<&Element> {
    root.children_named("tractor").find(|tractor| {
        tractor
            .property("kdenlive:uuid")
            .and_then(|value| parse_uuid(value).ok())
            == Some(uuid)
    })
}

fn sequence_duration(
    tractor: &Element,
    fps: Fraction,
) -> Result<i64, Box<dyn Error + Send + Sync>> {
    if let Some(duration) = tractor.property("kdenlive:maxduration") {
        return math::parse_frame(duration, fps).map_err(invalid);
    }
    element_duration(tractor, fps)
}

fn element_duration(node: &Element, fps: Fraction) -> Result<i64, Box<dyn Error + Send + Sync>> {
    if let Some(length) = node.attribute("length") {
        return math::parse_frame(length, fps).map_err(invalid);
    }
    let start = math::parse_frame(node.attribute("in").unwrap_or("0"), fps).map_err(invalid)?;
    let end_value = node
        .attribute("out")
        .ok_or_else(|| invalid("timeline element has no out point"))?;
    let end = math::parse_frame(end_value, fps).map_err(invalid)?;
    Ok(end - start + 1)
}

fn entry_in(entry: &Element, fps: Fraction) -> Result<i64, Box<dyn Error + Send + Sync>> {
    math::parse_frame(entry.attribute("in").unwrap_or("0"), fps).map_err(invalid)
}

fn frame_time(frame: i64, fps: Fraction) -> Time {
    source_time(frame, Fraction::from(1_u64), fps)
}

fn source_time(frame: i64, speed: Fraction, fps: Fraction) -> Time {
    let magnitude = Fraction::from(frame.unsigned_abs()) * speed / fps;
    Time {
        seconds: if frame < 0 { -magnitude } else { magnitude },
    }
}

fn parse_fraction(value: &str) -> Result<Fraction, Box<dyn Error + Send + Sync>> {
    value
        .trim()
        .parse::<Fraction>()
        .map_err(|error| invalid(error.to_string()))
}

fn parse_mlt_color(value: &str) -> Result<Color<u8>, Box<dyn Error + Send + Sync>> {
    let hex = value
        .trim()
        .trim_start_matches("0x")
        .trim_start_matches('#');
    if hex.len() != 8 {
        return Err(invalid("MLT color must contain RGBA bytes"));
    }
    Ok(Color::<u8>::from_rgba(
        u8::from_str_radix(&hex[0..2], 16)?,
        u8::from_str_radix(&hex[2..4], 16)?,
        u8::from_str_radix(&hex[4..6], 16)?,
        u8::from_str_radix(&hex[6..8], 16)?,
    ))
}

fn parse_uuid(value: &str) -> Result<Uuid, Box<dyn Error + Send + Sync>> {
    Ok(Uuid::parse_str(value.trim_matches(['{', '}']))?)
}

fn resolve_path(root: &Path, value: &str) -> PathBuf {
    let path = Path::new(value);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

fn property_u32(element: &Element, names: &[&str]) -> Option<u32> {
    names
        .iter()
        .find_map(|name| element.property(name).and_then(|value| value.parse().ok()))
}

fn positive_u64(value: Option<&str>, name: &str) -> Result<u64, Box<dyn Error + Send + Sync>> {
    let value = value
        .ok_or_else(|| invalid(format!("missing {name}")))?
        .parse::<u64>()?;
    if value == 0 {
        return Err(invalid(format!("{name} must be positive")));
    }
    Ok(value)
}

fn positive_u32(value: Option<&str>, name: &str) -> Result<u32, Box<dyn Error + Send + Sync>> {
    let value = positive_u64(value, name)?;
    Ok(value.try_into()?)
}

fn invalid(message: impl Into<String>) -> Box<dyn Error + Send + Sync> {
    Box::new(io::Error::new(io::ErrorKind::InvalidData, message.into()))
}
