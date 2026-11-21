use shrimply_math_core::time_ticks;
use shrimply_project_document::{
    caption::{CaptionItem, markup, ytt},
    project::Project,
};
use std::path::{Path, PathBuf};

mod ass;
mod math;
mod text;

pub(crate) const MILLIS_PER_SECOND: u32 = 1_000;
pub(crate) const CENTIS_PER_SECOND: u32 = 100;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CaptionFormat {
    Ytt,
    Ass,
    Srt,
    Vtt,
    Txt,
}

impl CaptionFormat {
    pub const ALL: [Self; 5] = [Self::Ytt, Self::Ass, Self::Srt, Self::Vtt, Self::Txt];

    pub fn label(self) -> &'static str {
        match self {
            Self::Ytt => "YouTube captions (YTT)",
            Self::Ass => "Advanced SubStation Alpha (ASS)",
            Self::Srt => "SubRip (SRT)",
            Self::Vtt => "WebVTT (VTT)",
            Self::Txt => "Plain text (TXT)",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Ytt => "ytt",
            Self::Ass => "ass",
            Self::Srt => "srt",
            Self::Vtt => "vtt",
            Self::Txt => "txt",
        }
    }

    pub fn mime_type(self) -> &'static str {
        match self {
            Self::Ytt => "application/xml",
            Self::Ass => "text/x-ssa",
            Self::Srt => "application/x-subrip",
            Self::Vtt => "text/vtt",
            Self::Txt => "text/plain",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExportMode {
    Merge,
    Separate,
}

#[derive(Clone, Copy, Debug)]
pub struct ExportSettings {
    pub format: CaptionFormat,
    pub mode: ExportMode,
}

pub struct CaptionOutput {
    pub path: PathBuf,
    pub content: String,
}

/// Serializes all files without touching the filesystem. Confirm replacement of
/// these exact paths before passing them to `write_export`.
pub fn prepare_export(
    project: &Project,
    path: &Path,
    settings: ExportSettings,
) -> Result<Vec<CaptionOutput>, String> {
    let mut path = path.to_path_buf();
    if !path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case(settings.format.extension()))
    {
        path.set_extension(settings.format.extension());
    }
    let tracks = project.caption_tracks.iter().filter(|track| track.enabled);
    let groups: Vec<(PathBuf, Vec<&CaptionItem>)> = match settings.mode {
        ExportMode::Merge => vec![(
            path.clone(),
            tracks.flat_map(|track| &track.items).collect(),
        )],
        ExportMode::Separate => tracks
            .enumerate()
            .map(|(index, track)| {
                let mut stem = path.file_stem().unwrap_or_default().to_os_string();
                stem.push(format!(
                    "-track-{}.{}",
                    index + 1,
                    settings.format.extension()
                ));
                (path.with_file_name(stem), track.items.iter().collect())
            })
            .collect(),
    };
    let mut outputs = Vec::new();
    for (path, mut items) in groups {
        items.retain(|item| !markup::plain_text(&item.text).trim().is_empty());
        if items.is_empty() {
            continue;
        }
        for item in &items {
            if time_ticks(item.start, MILLIS_PER_SECOND).is_none()
                || time_ticks(item.end, MILLIS_PER_SECOND).is_none()
                || item.end <= item.start
            {
                return Err(format!("Caption {} has an invalid time range.", item.id));
            }
            if item.text.contains('\0') {
                return Err(format!("Caption {} contains a null character.", item.id));
            }
        }
        items.sort_by_key(|item| (item.start, item.end));
        let content = match settings.format {
            CaptionFormat::Ytt => ytt::document(&items),
            CaptionFormat::Ass => ass::document(&items, project.canvas_size)?,
            CaptionFormat::Srt => text::document(&items, false)?,
            CaptionFormat::Vtt => text::document(&items, true)?,
            CaptionFormat::Txt => {
                let paragraphs = items
                    .iter()
                    .map(|item| {
                        markup::plain_text(&item.text)
                            .replace("\r\n", "\n")
                            .replace('\r', "\n")
                    })
                    .collect::<Vec<_>>();
                format!("{}\n", paragraphs.join("\n\n"))
            }
        };
        outputs.push(CaptionOutput { path, content });
    }
    if outputs.is_empty() {
        return Err("There are no nonempty captions on enabled tracks to export.".into());
    }
    Ok(outputs)
}

/// Writes prepared files directly. Callers must confirm replacement first.
pub fn write_export(outputs: Vec<CaptionOutput>) -> Result<Vec<PathBuf>, String> {
    let mut written: Vec<PathBuf> = Vec::with_capacity(outputs.len());
    for CaptionOutput { path, content } in outputs {
        if let Err(error) = std::fs::write(&path, content) {
            let completed = written
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join("\n");
            return Err(format!(
                "Could not save {}: {error}.\nFiles already exported: {}",
                path.display(),
                if completed.is_empty() {
                    "none"
                } else {
                    &completed
                }
            ));
        }
        written.push(path);
    }
    Ok(written)
}
