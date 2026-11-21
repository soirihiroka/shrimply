//! File references shared by the Kdenlive and OpenTimelineIO importers.
use super::av::{check_cancellation, remux_to};
use super::*;
use shrimply_resource_pipeline::CancelToken;
use std::io::{ErrorKind, Read};

/// File-backed pictures that an interchange document can reference without running a generator.
pub fn image_kind(path: &Path) -> Option<VisualMediaKind> {
    use VisualMediaKind::*;
    match file_kind(path)? {
        FileKind::Visual(kind @ (Image | Gif | Svg | Pdf | LayeredImage)) => Some(kind),
        _ => None,
    }
}

/// Interchange projects can refer to local media by path or by an escaped file URL.
pub fn interchange_path(base: &Path, reference: &str) -> Result<PathBuf, String> {
    if reference
        .get(..5)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("file:"))
    {
        return url::Url::parse(reference)
            .ok()
            .and_then(|url| url.to_file_path().ok())
            .ok_or_else(|| format!("Invalid local media URL: {reference}"));
    }
    Ok(base.join(reference))
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

fn requires_remux(path: &Path) -> Result<bool, String> {
    let kind = file_kind(path);
    if !path.is_file() {
        return if kind == Some(FileKind::RemuxToMp4) {
            Err(format!(
                "{} must be remuxed to MP4. Relink the missing media before importing the project.",
                path.display()
            ))
        } else {
            Ok(false) // Preserve ordinary offline media for relinking.
        };
    }
    if kind == Some(FileKind::RemuxToMp4) {
        return Ok(true);
    }

    // FFmpeg's Matroska demuxer requires the EBML header. Sniff only the
    // signature so unrelated images and audio never need a second probe.
    const EBML_HEADER: [u8; 4] = [0x1a, 0x45, 0xdf, 0xa3];
    let mut header = [0; EBML_HEADER.len()];
    match fs::File::open(path).and_then(|mut file| file.read_exact(&mut header)) {
        Ok(()) if header == EBML_HEADER => {}
        Ok(()) => return Ok(false),
        Err(error) if error.kind() == ErrorKind::UnexpectedEof => return Ok(false),
        Err(error) => return Err(format!("Could not read {}: {error}", path.display())),
    }
    match kind {
        None | Some(FileKind::Visual(VisualMediaKind::Video) | FileKind::Audio) => Ok(true),
        _ => Err(format!(
            "{} is an EBML container. Relink it as MKV/WebM before importing the project.",
            path.display()
        )),
    }
}

/// Interchange parsers preserve source paths, including offline assets. Normalize
/// those references together after the user chooses where to save the project.
pub fn save_interchange_project(
    destination: &Path,
    mut project: Project,
    cancellation: CancelToken,
) -> Result<(), String> {
    let mut outputs = HashMap::new();
    let sources: std::collections::BTreeSet<_> =
        project.assets().into_iter().map(Asset::path).collect();
    for path in sources {
        check_cancellation(&cancellation)?;
        if !requires_remux(path)? {
            continue;
        }
        let snapshot = Asset::new(path).snapshot()?;
        let name = path.file_name().ok_or("Missing media filename")?;
        let target = std::path::absolute(destination.with_file_name(name).with_extension("mp4"))
            .map_err(|error| error.to_string())?;
        let output = remux_to(path, target, cancellation.clone())?;
        snapshot.verify_current()?;
        outputs.insert(path.to_owned(), output);
    }
    project.visit_assets_mut(|asset| {
        if let Some(output) = outputs.get(asset.path()) {
            *asset = Asset::new(output.path());
        }
    });
    check_cancellation(&cancellation)?;
    shrimply_project_document::project::create_project_file(destination, &project)?;
    for output in outputs.into_values() {
        output.keep();
    }
    Ok(())
}
