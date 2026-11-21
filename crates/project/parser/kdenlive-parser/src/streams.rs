//! MLT absolute stream numbers include every stream. Its relative video numbers
//! include attached pictures, which Shrimply excludes from playable video tracks.
use super::xml::Element;
use shrimply_media_info::demux::{StreamKind, stream_kinds};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

pub(super) fn disabled(producer: &Element, relative: &str, absolute: &str) -> bool {
    producer
        .property(relative)
        .or_else(|| producer.property(absolute))
        == Some("-1")
}

pub(super) fn select(
    producer: &Element,
    path: &Path,
    cache: &mut HashMap<PathBuf, Option<Vec<StreamKind>>>,
) -> Result<(u32, usize, u32), String> {
    let video = producer.property("vstream");
    let audio = producer.property("astream");
    let video_index = producer.property("video_index");
    let audio_index = producer.property("audio_index");
    // With no explicit selection, ordinary image and generated producers need no I/O.
    if [video, audio, video_index, audio_index]
        .iter()
        .all(Option::is_none)
    {
        return Ok((0, 0, 0));
    }
    let catalog = cache
        .entry(path.to_owned())
        .or_insert_with(|| stream_kinds(path).ok());
    // A missing file may still have its stream layout recorded in the project.
    let recorded = catalog.is_none().then(|| {
        producer
            .children_named("property")
            .filter_map(|property| {
                let index = property
                    .attribute("name")?
                    .strip_prefix("meta.media.")?
                    .strip_suffix(".stream.type")?
                    .parse::<usize>()
                    .ok()?;
                let kind = match property.text.as_str() {
                    "video" => StreamKind::Video,
                    "audio" => StreamKind::Audio,
                    _ => StreamKind::Other,
                };
                Some((index, kind))
            })
            .collect::<HashMap<_, _>>()
    });
    let from_catalog = |index| {
        catalog
            .as_ref()
            .and_then(|kinds| kinds.get(index))
            .or_else(|| recorded.as_ref()?.get(&index))
            .copied()
    };
    let resolve = |relative: Option<&str>,
                   absolute: Option<&str>,
                   medium: StreamKind|
     -> Result<(u32, usize), String> {
        if relative.or(absolute) == Some("-1") || relative.or(absolute).is_none() {
            return Ok((0, 0));
        }
        let rel = relative.and_then(|s| s.parse::<usize>().ok());
        let abs = absolute.and_then(|s| s.parse::<usize>().ok());
        let mut mlt_index = 0;
        let mut track_id = 0;
        for index in 0.. {
            let Some(kind) = from_catalog(index) else {
                break;
            };
            if kind == medium || (medium == StreamKind::Video && kind == StreamKind::Artwork) {
                if rel.map_or(Some(index) == abs, |r| mlt_index == r) {
                    if kind == StreamKind::Artwork {
                        return Err(format!(
                            "{} selects cover artwork as a video track",
                            path.display()
                        ));
                    }
                    return Ok((track_id, index));
                }
                mlt_index += 1;
                if kind == medium {
                    track_id += 1;
                }
            }
        }
        // Offline projects can lack stream metadata. Retain the relative selection
        // if available, and the absolute index for dimensions stored by MLT.
        Ok((
            rel.and_then(|i| i.try_into().ok()).unwrap_or(0),
            abs.unwrap_or(0),
        ))
    };
    let (video_id, video_index) = resolve(video, video_index, StreamKind::Video)?;
    Ok((
        video_id,
        video_index,
        resolve(audio, audio_index, StreamKind::Audio)?.0,
    ))
}
