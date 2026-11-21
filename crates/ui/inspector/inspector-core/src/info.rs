use std::path::PathBuf;

use shrimply_media_info::{ExactRatio, ExactTime, FileInfo, StreamInfo};
use shrimply_project_document::project::{AudioSource, ItemRef, Project, VideoItemContent};

use crate::InspectorTarget;

pub mod metadata;

pub const ARTWORK_PREVIEW_HEIGHT: u32 = 220;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MediaInfoPresentation {
    pub artwork: Option<MediaInfoArtwork>,
    pub groups: Vec<MediaInfoGroup>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MediaInfoArtwork {
    pub data: Vec<u8>,
    pub mime_type: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MediaInfoGroup {
    pub title: String,
    pub rows: Vec<MediaInfoRow>,
    source: Option<SourceMetadata>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MediaInfoRow {
    pub label: String,
    pub value: String,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SourceMetadata {
    None,
    Video(u32),
    Audio(u32),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InspectorMedia {
    pub path: PathBuf,
    pub revision: Option<u64>,
    pub selected: SourceMetadata,
}

pub fn media_info_presentation(
    info: &FileInfo,
    format_date: impl Fn(i64) -> Option<String>,
) -> MediaInfoPresentation {
    let mut groups = Vec::new();

    let mut file = MediaInfoGroup::new("File");
    if let Some(name) = &info.file.file_name {
        file.add("Name", name);
    }
    if let Some(extension) = &info.file.extension {
        file.add("Extension", extension);
    }
    file.add("Size", format_bytes(info.file.byte_size));
    if let Some(path) = &info.file.canonical_path
        && path != &info.file.path
    {
        file.add("Canonical Path", path);
    }
    if let Some(created) = info.file.created_unix_seconds.and_then(&format_date) {
        file.add("Created", created);
    }
    if let Some(modified) = info.file.modified_unix_seconds.and_then(&format_date) {
        file.add("Modified", modified);
    }
    groups.push(file);

    if let Some(format) = &info.container {
        let mut group = MediaInfoGroup::new("Media");
        group.add("Format", &format.description);
        group.add("Format IDs", format.format_names.join(", "));
        if !format.extensions.is_empty() {
            group.add("Known Extensions", format.extensions.join(", "));
        }
        if !format.mime_types.is_empty() {
            group.add("MIME Type", format.mime_types.join(", "));
        }
        if let Some(start_time) = &format.start_time {
            group.add("File Start Time", format_time(start_time));
        }
        if let Some(duration) = &format.duration {
            group.add("File Duration", format_time(duration));
        }
        if let Some(bit_rate) = format.bit_rate {
            group.add("Bit Rate", format_rate(bit_rate));
        }
        group.add("Streams", info.streams.len().to_string());
        groups.push(group);
    }

    let tags = &info.common_tags;
    if tags.title.is_some()
        || tags.artist.is_some()
        || tags.album.is_some()
        || tags.album_artist.is_some()
        || tags.genre.is_some()
        || tags.year.is_some()
        || tags.track.is_some()
        || tags.disc.is_some()
    {
        let mut group = MediaInfoGroup::new("Tags");
        for (label, value) in [
            ("Title", tags.title.as_deref()),
            ("Artist", tags.artist.as_deref()),
            ("Album", tags.album.as_deref()),
            ("Album Artist", tags.album_artist.as_deref()),
            ("Genre", tags.genre.as_deref()),
        ] {
            if let Some(value) = value {
                group.add(label, value);
            }
        }
        if let Some(year) = tags.year {
            group.add("Year", year.to_string());
        }
        if let Some(track) = tags.track {
            group.add("Track", number_of(track, tags.track_total));
        }
        if let Some(disc) = tags.disc {
            group.add("Disc", number_of(disc, tags.disc_total));
        }
        groups.push(group);
    }

    if !info.artwork.is_empty() {
        let mut group = MediaInfoGroup::new("Embedded Artwork");
        for artwork in &info.artwork {
            let dimensions = artwork
                .width
                .zip(artwork.height)
                .map(|(width, height)| format!("{width} × {height}, "))
                .unwrap_or_default();
            let color_depth = artwork
                .color_depth
                .map(|depth| format!("{depth}-bit, "))
                .unwrap_or_default();
            let description = artwork
                .description
                .as_deref()
                .filter(|description| !description.is_empty())
                .map(|description| format!(" · {description}"))
                .unwrap_or_default();
            let mime = artwork.mime_type.as_deref().unwrap_or("unknown type");
            group.add(
                format!("Artwork {}", artwork.index + 1),
                format!(
                    "{}{description} · {mime} · {dimensions}{color_depth}{}",
                    artwork.picture_type,
                    format_bytes(artwork.byte_size as u64)
                ),
            );
        }
        groups.push(group);
    }

    let mut video_ordinal = 0_u32;
    let mut audio_ordinal = 0_u32;
    for stream in &info.streams {
        let source = match stream.kind.as_str() {
            "video" => {
                let source = SourceMetadata::Video(video_ordinal);
                video_ordinal = video_ordinal.saturating_add(1);
                source
            }
            "audio" => {
                let source = SourceMetadata::Audio(audio_ordinal);
                audio_ordinal = audio_ordinal.saturating_add(1);
                source
            }
            _ => SourceMetadata::None,
        };
        groups.push(stream_group(stream, source));
    }

    if !info.chapters.is_empty() {
        let mut group = MediaInfoGroup::new("Chapters");
        for (index, chapter) in info.chapters.iter().enumerate() {
            group.add(
                format!("Chapter {}", index + 1),
                format!(
                    "{} – {}",
                    format_time(&chapter.start),
                    format_time(&chapter.end)
                ),
            );
            group.add_tags(&chapter.tags);
        }
        groups.push(group);
    }

    if let Some(image) = &info.image {
        let mut group = MediaInfoGroup::new("Image");
        group.add("Image Format", &image.image_type);
        group.add(
            "Pixel Dimensions",
            format!("{} × {}", image.width, image.height),
        );
        for field in &image.exif {
            group.add(format!("{} / {}", field.ifd, field.tag), &field.value);
        }
        groups.push(group);
    }

    if let Some(format) = &info.container
        && !format.tags.is_empty()
    {
        let mut group = MediaInfoGroup::new("Container Tags");
        group.add_tags(&format.tags);
        groups.push(group);
    }
    if !info.tags.is_empty() {
        let mut group = MediaInfoGroup::new("Raw Audio Tags");
        for tag in &info.tags {
            group.add(format!("{} · {}", tag.key, tag.source), &tag.value);
        }
        groups.push(group);
    }
    if !info.diagnostics.is_empty() {
        let mut group = MediaInfoGroup::new("Diagnostics");
        for diagnostic in &info.diagnostics {
            group.add("Metadata", diagnostic);
        }
        groups.push(group);
    }

    MediaInfoPresentation {
        artwork: primary_artwork(info).map(|artwork| MediaInfoArtwork {
            data: artwork.data.clone(),
            mime_type: artwork.mime_type.clone(),
        }),
        groups,
    }
}

impl MediaInfoGroup {
    fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            rows: Vec::new(),
            source: None,
        }
    }

    fn add(&mut self, label: impl Into<String>, value: impl Into<String>) {
        self.rows.push(MediaInfoRow {
            label: label.into(),
            value: value.into(),
        });
    }

    fn add_tags(&mut self, tags: &[shrimply_media_info::TagValue]) {
        for tag in tags {
            self.add(&tag.key, &tag.value);
        }
    }

    pub fn display_title(&self, selected: SourceMetadata) -> String {
        if self.source == Some(selected) && selected != SourceMetadata::None {
            format!("{} · Selected", self.title)
        } else {
            self.title.clone()
        }
    }
}

fn stream_group(stream: &StreamInfo, source: SourceMetadata) -> MediaInfoGroup {
    let mut group = MediaInfoGroup::new(format!("Stream {} ({})", stream.index + 1, stream.kind));
    group.source = Some(source);
    group.add("Stream Index", stream.index.to_string());
    group.add("Stream ID", stream.id.to_string());
    group.add("Codec", &stream.codec);
    if let Some(description) = &stream.codec_description {
        group.add("Codec Description", description);
    }
    if let Some(profile) = &stream.profile {
        group.add("Profile", profile);
    }
    if let Some(bit_rate) = stream.bit_rate {
        group.add("Bit Rate", format_rate(bit_rate));
    }
    if let Some(time_base) = &stream.time_base {
        group.add("Time Base", format_ratio(time_base));
    }
    if let Some(start_time) = &stream.start_time {
        group.add("Start Time", format_time(start_time));
    }
    if let Some(duration) = &stream.duration {
        group.add("Duration", format_time(duration));
    }
    if let Some(frames) = stream.frame_count {
        group.add("Frames", frames.to_string());
    }
    group.add("Disposition", &stream.disposition);
    if let Some(video) = &stream.video {
        group.add("Dimensions", format!("{} × {}", video.width, video.height));
        group.add("Pixel Format", &video.pixel_format);
        if let Some(rate) = &video.average_frame_rate {
            group.add("Frame Rate", format!("{} FPS", format_ratio(rate)));
        }
        if let Some(rate) = &video.nominal_frame_rate {
            group.add("Nominal Frame Rate", format!("{} FPS", format_ratio(rate)));
        }
        if let Some(aspect) = &video.sample_aspect_ratio {
            group.add("Pixel Aspect Ratio", format_ratio(aspect));
        }
        group.add("Color Range", &video.color_range);
        group.add("Color Space", &video.color_space);
        group.add("Color Primaries", &video.color_primaries);
        group.add("Color Transfer", &video.color_transfer);
        group.add("Chroma Location", &video.chroma_location);
        group.add("B-Frames", if video.has_b_frames { "Yes" } else { "No" });
    }
    if let Some(audio) = &stream.audio {
        group.add("Sample Rate", format!("{} Hz", audio.sample_rate));
        group.add("Channels", audio.channels.to_string());
        group.add("Channel Layout", &audio.channel_layout);
        group.add("Sample Format", &audio.sample_format);
        group.add("Frame Size", audio.frame_size.to_string());
    }
    group.add_tags(&stream.tags);
    group
}

fn primary_artwork(info: &FileInfo) -> Option<&shrimply_media_info::Artwork> {
    info.artwork
        .iter()
        .find(|artwork| artwork.picture_type == "CoverFront")
        .or_else(|| info.artwork.first())
}

pub(crate) fn target_media(project: &Project, target: &InspectorTarget) -> Option<InspectorMedia> {
    let InspectorTarget::Item(address) = target else {
        return None;
    };
    let (file, selected) = match project.item(address)? {
        ItemRef::Audio(item) if matches!(item.source, AudioSource::Media | AudioSource::Tts(_)) => {
            (&item.file, SourceMetadata::Audio(item.track_id))
        }
        ItemRef::Video(item) if !matches!(item.content, VideoItemContent::FoldedSequence(_)) => {
            let selected = if matches!(
                item.content,
                VideoItemContent::Media | VideoItemContent::Gif | VideoItemContent::AnimatedImage
            ) {
                SourceMetadata::Video(item.track_id)
            } else {
                SourceMetadata::None
            };
            (&item.file, selected)
        }
        ItemRef::Comment(_) | ItemRef::Caption(_) | ItemRef::Audio(_) | ItemRef::Video(_) => {
            return None;
        }
    };
    (!file.path().as_os_str().is_empty()).then(|| InspectorMedia {
        path: file.path().to_path_buf(),
        revision: file.snapshot().ok().map(|snapshot| snapshot.revision()),
        selected,
    })
}

fn number_of(value: u32, total: Option<u32>) -> String {
    total.map_or_else(|| value.to_string(), |total| format!("{value} of {total}"))
}

fn format_ratio(value: &ExactRatio) -> String {
    if value.denominator == 1 {
        value.numerator.to_string()
    } else {
        format!("{}/{}", value.numerator, value.denominator)
    }
}

fn format_time(value: &ExactTime) -> String {
    format!("{} × {} s", value.value, format_ratio(&value.time_base))
}

fn format_rate(bits_per_second: i64) -> String {
    const KILOBIT: u64 = 1_000;
    const MEGABIT: u64 = KILOBIT * 1_000;
    const GIGABIT: u64 = MEGABIT * 1_000;
    let bits = bits_per_second.max(0) as u64;
    let (value, unit) = if bits >= GIGABIT {
        (GIGABIT, "Gb/s")
    } else if bits >= MEGABIT {
        (MEGABIT, "Mb/s")
    } else if bits >= KILOBIT {
        (KILOBIT, "kb/s")
    } else {
        return format!("{bits} b/s");
    };
    format!("{}.{} {unit}", bits / value, bits % value * 10 / value)
}

fn format_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = KIB * 1024;
    const GIB: u64 = MIB * 1024;
    let (value, unit) = if bytes >= GIB {
        (GIB, "GiB")
    } else if bytes >= MIB {
        (MIB, "MiB")
    } else if bytes >= KIB {
        (KIB, "KiB")
    } else {
        return format!("{bytes} B");
    };
    format!("{}.{} {unit}", bytes / value, bytes % value * 10 / value)
}
