use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use shrimply_project_document::project::Project;

pub(crate) fn temporary_sibling(path: &Path) -> Result<tempfile::TempPath, std::io::Error> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    tempfile::Builder::new()
        .prefix(".shrimply-export-")
        .tempfile_in(parent)
        .map(tempfile::NamedTempFile::into_temp_path)
}

pub(crate) fn needs_global_header(container: &str) -> Result<bool, String> {
    use ffmpeg_next::sys;
    let name = CString::new(container).map_err(|error| error.to_string())?;
    let format = unsafe { sys::av_guess_format(name.as_ptr(), std::ptr::null(), std::ptr::null()) };
    if format.is_null() {
        return Err(format!("FFmpeg output format {container} is unavailable"));
    }
    Ok(unsafe { (*format).flags & sys::AVFMT_GLOBALHEADER != 0 })
}

pub fn default_filename(project: &Project, extension: &str) -> String {
    let mut base = strip_human_timestamp_suffix(project.name.trim());
    if base.is_empty() {
        base = fallback_project_name();
    }
    format!("{base}.{extension}")
}

pub fn ensure_extension(mut path: PathBuf, extension: &str) -> PathBuf {
    if !path
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case(extension))
    {
        path.set_extension(extension);
    }
    path
}

fn fallback_project_name() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs())
        .unwrap_or(0);
    format!("project_{timestamp}")
}

fn strip_human_timestamp_suffix(name: &str) -> String {
    let mut candidate = name.trim().to_string();
    loop {
        if let Some((prefix, suffix)) = candidate.rsplit_once('_')
            && (is_time_hyphen(suffix)
                || is_date_hyphen(suffix)
                || is_compact_time(suffix)
                || is_compact_date(suffix))
        {
            candidate = prefix.to_string();
            continue;
        }

        let dashed = strip_dashed_timestamp_suffix(&candidate);
        if dashed != candidate && !dashed.is_empty() {
            candidate = dashed;
            continue;
        }
        break;
    }

    if candidate.is_empty() {
        "project".to_string()
    } else {
        candidate
    }
}

fn strip_dashed_timestamp_suffix(name: &str) -> String {
    let parts: Vec<&str> = name.split('-').collect();
    if parts.len() >= 6 && is_datetime_hyphen_parts(&parts[parts.len() - 6..]) {
        return parts[..parts.len() - 6].join("-");
    }
    if parts.len() >= 3 && is_date_hyphen_parts(&parts[parts.len() - 3..]) {
        return parts[..parts.len() - 3].join("-");
    }
    name.to_string()
}

fn is_time_hyphen(part: &str) -> bool {
    let segments: Vec<_> = part.split('-').collect();
    segments.len() == 3 && is_part_lengths(&segments, [2, 2, 2]) && is_all_digits(&segments)
}

fn is_date_hyphen(part: &str) -> bool {
    let segments: Vec<_> = part.split('-').collect();
    segments.len() == 3 && is_part_lengths(&segments, [4, 2, 2]) && is_all_digits(&segments)
}

fn is_datetime_hyphen_parts(parts: &[&str]) -> bool {
    parts.len() == 6
        && is_part_lengths(&parts[0..3], [4, 2, 2])
        && is_part_lengths(&parts[3..6], [2, 2, 2])
        && is_all_digits(&parts[0..3])
        && is_all_digits(&parts[3..6])
}

fn is_date_hyphen_parts(parts: &[&str]) -> bool {
    is_part_lengths(parts, [4, 2, 2]) && is_all_digits(parts)
}

fn is_compact_time(part: &str) -> bool {
    part.len() == 6 && part.bytes().all(|byte| byte.is_ascii_digit())
}

fn is_compact_date(part: &str) -> bool {
    part.len() == 8 && part.bytes().all(|byte| byte.is_ascii_digit())
}

fn is_part_lengths(parts: &[&str], lens: [usize; 3]) -> bool {
    parts.len() == lens.len() && parts.iter().zip(lens).all(|(part, len)| part.len() == len)
}

fn is_all_digits(segments: &[&str]) -> bool {
    segments
        .iter()
        .all(|segment| !segment.is_empty() && segment.bytes().all(|byte| byte.is_ascii_digit()))
}
