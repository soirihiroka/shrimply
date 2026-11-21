pub mod demux;

use std::{
    collections::HashMap,
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

use ffmpeg::{codec, format, media};
use ffmpeg_next as ffmpeg;
use lofty::{
    file::TaggedFileExt, picture::PictureInformation, prelude::Accessor, probe::Probe,
    tag::ItemValue,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

type Cache = Mutex<HashMap<(PathBuf, Option<u64>), Arc<FileInfo>>>;

static CACHE: OnceLock<Cache> = OnceLock::new();
static FFMPEG: OnceLock<Result<(), String>> = OnceLock::new();

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ExactRatio {
    pub numerator: i64,
    pub denominator: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ExactTime {
    pub value: i64,
    pub time_base: ExactRatio,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TagValue {
    pub key: String,
    pub value: String,
    pub source: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Artwork {
    pub index: usize,
    pub picture_type: String,
    pub mime_type: Option<String>,
    pub description: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub color_depth: Option<u32>,
    pub byte_size: usize,
    #[serde(skip)]
    #[schemars(skip)]
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct CommonTags {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub genre: Option<String>,
    pub year: Option<u32>,
    pub track: Option<u32>,
    pub track_total: Option<u32>,
    pub disc: Option<u32>,
    pub disc_total: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct FileSystemInfo {
    pub path: String,
    pub canonical_path: Option<String>,
    pub file_name: Option<String>,
    pub extension: Option<String>,
    pub byte_size: u64,
    pub created_unix_seconds: Option<i64>,
    pub modified_unix_seconds: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ContainerInfo {
    pub format_names: Vec<String>,
    pub description: String,
    pub extensions: Vec<String>,
    pub mime_types: Vec<String>,
    pub duration: Option<ExactTime>,
    pub start_time: Option<ExactTime>,
    pub bit_rate: Option<i64>,
    pub tags: Vec<TagValue>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct VideoStreamInfo {
    pub width: u32,
    pub height: u32,
    pub pixel_format: String,
    pub sample_aspect_ratio: Option<ExactRatio>,
    pub average_frame_rate: Option<ExactRatio>,
    pub nominal_frame_rate: Option<ExactRatio>,
    pub color_range: String,
    pub color_space: String,
    pub color_primaries: String,
    pub color_transfer: String,
    pub chroma_location: String,
    pub has_b_frames: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct AudioStreamInfo {
    pub sample_rate: u32,
    pub channels: u16,
    pub channel_layout: String,
    pub sample_format: String,
    pub frame_size: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct StreamInfo {
    pub index: usize,
    pub id: i32,
    pub kind: String,
    pub codec: String,
    pub codec_description: Option<String>,
    pub profile: Option<String>,
    pub bit_rate: Option<i64>,
    pub time_base: Option<ExactRatio>,
    pub start_time: Option<ExactTime>,
    pub duration: Option<ExactTime>,
    pub frame_count: Option<i64>,
    pub disposition: String,
    pub tags: Vec<TagValue>,
    pub video: Option<VideoStreamInfo>,
    pub audio: Option<AudioStreamInfo>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ChapterInfo {
    pub id: i64,
    pub start: ExactTime,
    pub end: ExactTime,
    pub tags: Vec<TagValue>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ExifField {
    pub ifd: String,
    pub tag: String,
    pub value: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ImageInfo {
    pub width: usize,
    pub height: usize,
    pub image_type: String,
    pub exif: Vec<ExifField>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct FileInfo {
    pub file: FileSystemInfo,
    pub container: Option<ContainerInfo>,
    pub streams: Vec<StreamInfo>,
    pub chapters: Vec<ChapterInfo>,
    pub common_tags: CommonTags,
    pub tags: Vec<TagValue>,
    pub artwork: Vec<Artwork>,
    pub image: Option<ImageInfo>,
    pub diagnostics: Vec<String>,
}

pub fn inspect(path: &Path, revision: Option<u64>) -> Result<Arc<FileInfo>, String> {
    let key = (path.to_path_buf(), revision);
    if let Some(info) = CACHE
        .get_or_init(Mutex::default)
        .lock()
        .expect("media info cache lock poisoned")
        .get(&key)
        .cloned()
    {
        return Ok(info);
    }
    let info = Arc::new(inspect_uncached(path)?);
    CACHE
        .get_or_init(Mutex::default)
        .lock()
        .expect("media info cache lock poisoned")
        .insert(key, info.clone());
    Ok(info)
}

fn inspect_uncached(path: &Path) -> Result<FileInfo, String> {
    let metadata = path
        .metadata()
        .map_err(|error| format!("could not inspect {}: {error}", path.display()))?;
    if !metadata.is_file() {
        return Err(format!("media source is not a file: {}", path.display()));
    }
    let mut result = FileInfo {
        file: FileSystemInfo {
            path: path.to_string_lossy().into_owned(),
            canonical_path: path
                .canonicalize()
                .ok()
                .map(|path| path.to_string_lossy().into_owned()),
            file_name: path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned()),
            extension: path
                .extension()
                .map(|extension| extension.to_string_lossy().into_owned()),
            byte_size: metadata.len(),
            created_unix_seconds: metadata.created().ok().and_then(unix_seconds),
            modified_unix_seconds: metadata.modified().ok().and_then(unix_seconds),
        },
        container: None,
        streams: Vec::new(),
        chapters: Vec::new(),
        common_tags: CommonTags::default(),
        tags: Vec::new(),
        artwork: Vec::new(),
        image: None,
        diagnostics: Vec::new(),
    };
    if let Err(error) = inspect_ffmpeg(path, &mut result) {
        result.diagnostics.push(error);
    }
    if result.streams.iter().any(|stream| stream.kind == "audio")
        && let Err(error) = inspect_audio_tags(path, &mut result)
    {
        result.diagnostics.push(error);
    }
    fill_common_tags_from_container(&mut result);
    if imagesize::size(path).is_ok()
        && let Err(error) = inspect_image(path, &mut result)
    {
        result.diagnostics.push(error);
    }
    Ok(result)
}

fn unix_seconds(time: SystemTime) -> Option<i64> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_secs()).ok())
}

fn inspect_ffmpeg(path: &Path, result: &mut FileInfo) -> Result<(), String> {
    FFMPEG
        .get_or_init(|| ffmpeg::init().map_err(|error| error.to_string()))
        .clone()
        .map_err(|error| format!("could not initialize FFmpeg: {error}"))?;
    let input = demux::open(path)
        .map_err(|error| format!("FFmpeg could not inspect {}: {error}", path.display()))?;
    let detected = input.format();
    result.container = Some(ContainerInfo {
        format_names: detected.name().split(',').map(str::to_string).collect(),
        description: detected.description().to_string(),
        extensions: detected
            .extensions()
            .into_iter()
            .map(str::to_string)
            .collect(),
        mime_types: detected
            .mime_types()
            .into_iter()
            .map(str::to_string)
            .collect(),
        duration: positive_time(
            input.duration(),
            ExactRatio {
                numerator: 1,
                denominator: ffmpeg::ffi::AV_TIME_BASE as i64,
            },
        ),
        start_time: valid_time(
            // SAFETY: the input context owns a live AVFormatContext for this scope.
            unsafe { (*input.as_ptr()).start_time },
            ExactRatio {
                numerator: 1,
                denominator: ffmpeg::ffi::AV_TIME_BASE as i64,
            },
        ),
        bit_rate: (input.bit_rate() > 0).then(|| input.bit_rate()),
        tags: dictionary(input.metadata(), "container"),
    });
    for stream in input.streams() {
        result.streams.push(stream_info(&stream));
    }
    for chapter in input.chapters() {
        let time_base = ratio(chapter.time_base()).unwrap_or(ExactRatio {
            numerator: 1,
            denominator: 1,
        });
        result.chapters.push(ChapterInfo {
            id: chapter.id(),
            start: ExactTime {
                value: chapter.start(),
                time_base: time_base.clone(),
            },
            end: ExactTime {
                value: chapter.end(),
                time_base,
            },
            tags: dictionary(chapter.metadata(), "chapter"),
        });
    }
    Ok(())
}

fn stream_info(stream: &format::stream::Stream<'_>) -> StreamInfo {
    let parameters = stream.parameters();
    let kind = parameters.medium();
    let time_base = ratio(stream.time_base());
    let decoder = codec::decoder::find(parameters.id());
    let context = codec::context::Context::from_parameters(parameters.clone()).ok();
    let mut profile = None;
    let (video, audio) = match kind {
        media::Type::Video => {
            // avcodec_parameters_to_context provides typed pixel/color fields without
            // opening a decoder just to describe a stream.
            let video = context.map(|context| {
                let video = unsafe { &*context.as_ptr() };
                profile = Some(format!(
                    "{:?}",
                    codec::Profile::from((parameters.id(), video.profile))
                ));
                VideoStreamInfo {
                    width: video.width.max(0) as u32,
                    height: video.height.max(0) as u32,
                    pixel_format: match format::Pixel::from(video.pix_fmt) {
                        format::Pixel::None => "Unknown".into(),
                        format => format!("{format:?}"),
                    },
                    sample_aspect_ratio: ratio(video.sample_aspect_ratio.into()),
                    average_frame_rate: ratio(stream.avg_frame_rate()),
                    nominal_frame_rate: ratio(stream.rate()),
                    color_range: format!("{:?}", ffmpeg::color::Range::from(video.color_range)),
                    color_space: format!("{:?}", ffmpeg::color::Space::from(video.colorspace)),
                    color_primaries: format!(
                        "{:?}",
                        ffmpeg::color::Primaries::from(video.color_primaries)
                    ),
                    color_transfer: format!(
                        "{:?}",
                        ffmpeg::color::TransferCharacteristic::from(video.color_trc)
                    ),
                    chroma_location: format!(
                        "{:?}",
                        ffmpeg::chroma::Location::from(video.chroma_sample_location)
                    ),
                    has_b_frames: video.has_b_frames != 0,
                }
            });
            (video, None)
        }
        media::Type::Audio => {
            let audio = context.and_then(|context| context.decoder().audio().ok());
            profile = audio
                .as_ref()
                .map(|decoder| format!("{:?}", decoder.profile()));
            (
                None,
                audio.map(|decoder| AudioStreamInfo {
                    sample_rate: decoder.rate(),
                    channels: decoder.channels(),
                    channel_layout: format!("{:?}", decoder.channel_layout()),
                    sample_format: format!("{:?}", decoder.format()),
                    frame_size: decoder.frame_size(),
                }),
            )
        }
        _ => (None, None),
    };
    StreamInfo {
        index: stream.index(),
        id: stream.id(),
        kind: format!("{kind:?}").to_lowercase(),
        codec: parameters.id().name().to_string(),
        codec_description: decoder.map(|decoder| decoder.description().to_string()),
        profile,
        bit_rate: (parameters.bit_rate() > 0).then(|| parameters.bit_rate()),
        time_base: time_base.clone(),
        start_time: time_base
            .clone()
            .and_then(|time_base| valid_time(stream.start_time(), time_base)),
        duration: time_base.and_then(|time_base| positive_time(stream.duration(), time_base)),
        frame_count: (stream.frames() > 0).then(|| stream.frames()),
        disposition: format!("{:?}", stream.disposition()),
        tags: dictionary(stream.metadata(), "stream"),
        video,
        audio,
    }
}

fn ratio(value: ffmpeg::Rational) -> Option<ExactRatio> {
    (value.numerator() > 0 && value.denominator() > 0).then(|| ExactRatio {
        numerator: i64::from(value.numerator()),
        denominator: i64::from(value.denominator()),
    })
}

fn valid_time(value: i64, time_base: ExactRatio) -> Option<ExactTime> {
    (value != ffmpeg::ffi::AV_NOPTS_VALUE).then_some(ExactTime { value, time_base })
}

fn positive_time(value: i64, time_base: ExactRatio) -> Option<ExactTime> {
    (value > 0).then_some(ExactTime { value, time_base })
}

fn dictionary(dictionary: ffmpeg::DictionaryRef<'_>, source: &str) -> Vec<TagValue> {
    dictionary
        .iter()
        .map(|(key, value)| TagValue {
            key: key.to_string(),
            value: value.to_string(),
            source: source.to_string(),
        })
        .collect()
}

fn inspect_audio_tags(path: &Path, result: &mut FileInfo) -> Result<(), String> {
    let probe = Probe::open(path)
        .map_err(|error| format!("audio tags were unavailable: {error}"))?
        .guess_file_type()
        .map_err(|error| format!("audio tags were unavailable: {error}"))?;
    let tagged = probe
        .options(lofty::config::ParseOptions::new().read_properties(false))
        .read()
        .map_err(|error| format!("audio tags were unavailable: {error}"))?;
    if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
        result.common_tags = CommonTags {
            title: tag.title().map(|value| value.into_owned()),
            artist: tag.artist().map(|value| value.into_owned()),
            album: tag.album().map(|value| value.into_owned()),
            album_artist: tag
                .get_string(lofty::tag::ItemKey::AlbumArtist)
                .map(str::to_string),
            genre: tag.genre().map(|value| value.into_owned()),
            year: tag.date().map(|date| u32::from(date.year)),
            track: tag.track(),
            track_total: tag.track_total(),
            disc: tag.disk(),
            disc_total: tag.disk_total(),
        };
    }
    for tag in tagged.tags() {
        let source = format!("audio:{:?}", tag.tag_type());
        for item in tag.items() {
            let value = match item.value() {
                ItemValue::Text(value) | ItemValue::Locator(value) => value.clone(),
                ItemValue::Binary(value) => format!("<{} bytes>", value.len()),
            };
            result.tags.push(TagValue {
                key: format!("{:?}", item.key()),
                value,
                source: source.clone(),
            });
        }
        for picture in tag.pictures() {
            if result
                .artwork
                .iter()
                .any(|existing| existing.data == picture.data())
            {
                continue;
            }
            let details = PictureInformation::from_picture(picture).ok();
            let dimensions = imagesize::blob_size(picture.data()).ok();
            result.artwork.push(Artwork {
                index: result.artwork.len(),
                picture_type: format!("{:?}", picture.pic_type()),
                mime_type: picture.mime_type().map(ToString::to_string),
                description: picture.description().map(str::to_string),
                width: dimensions
                    .as_ref()
                    .and_then(|size| u32::try_from(size.width).ok())
                    .or_else(|| {
                        details
                            .as_ref()
                            .map(|details| details.width)
                            .filter(|width| *width > 0)
                    }),
                height: dimensions
                    .as_ref()
                    .and_then(|size| u32::try_from(size.height).ok())
                    .or_else(|| {
                        details
                            .as_ref()
                            .map(|details| details.height)
                            .filter(|height| *height > 0)
                    }),
                color_depth: details
                    .map(|details| details.color_depth)
                    .filter(|depth| *depth > 0),
                byte_size: picture.data().len(),
                data: picture.data().to_vec(),
            });
        }
    }
    Ok(())
}

fn fill_common_tags_from_container(result: &mut FileInfo) {
    let Some(container) = &result.container else {
        return;
    };
    for tag in &container.tags {
        let key = tag.key.to_ascii_lowercase().replace([' ', '-'], "_");
        match key.as_str() {
            "title" if result.common_tags.title.is_none() => {
                result.common_tags.title = Some(tag.value.clone());
            }
            "artist" if result.common_tags.artist.is_none() => {
                result.common_tags.artist = Some(tag.value.clone());
            }
            "album" if result.common_tags.album.is_none() => {
                result.common_tags.album = Some(tag.value.clone());
            }
            "album_artist" | "albumartist" if result.common_tags.album_artist.is_none() => {
                result.common_tags.album_artist = Some(tag.value.clone());
            }
            "genre" if result.common_tags.genre.is_none() => {
                result.common_tags.genre = Some(tag.value.clone());
            }
            "date" | "year" if result.common_tags.year.is_none() => {
                result.common_tags.year = tag.value.get(..4).and_then(|year| year.parse().ok());
            }
            "track" | "tracknumber" if result.common_tags.track.is_none() => {
                let mut values = tag.value.split('/').filter_map(|value| value.parse().ok());
                result.common_tags.track = values.next();
                result.common_tags.track_total = values.next();
            }
            "disc" | "discnumber" if result.common_tags.disc.is_none() => {
                let mut values = tag.value.split('/').filter_map(|value| value.parse().ok());
                result.common_tags.disc = values.next();
                result.common_tags.disc_total = values.next();
            }
            _ => {}
        }
    }
}

fn inspect_image(path: &Path, result: &mut FileInfo) -> Result<(), String> {
    let size = imagesize::size(path)
        .map_err(|error| format!("image metadata was unavailable: {error}"))?;
    let image_type = File::open(path)
        .ok()
        .and_then(|file| imagesize::reader_type(BufReader::new(file)).ok())
        .map(|kind| format!("{kind:?}"))
        .unwrap_or_else(|| "Unknown".to_string());
    let mut image = ImageInfo {
        width: size.width,
        height: size.height,
        image_type,
        exif: Vec::new(),
    };
    if let Ok(file) = File::open(path)
        && let Ok(exif) = exif::Reader::new().read_from_container(&mut BufReader::new(file))
    {
        image.exif = exif
            .fields()
            .map(|field| ExifField {
                ifd: format!("{:?}", field.ifd_num),
                tag: field.tag.to_string(),
                value: field.display_value().with_unit(&exif).to_string(),
            })
            .collect();
    }
    result.image = Some(image);
    Ok(())
}
