use super::{MAX_CODE_BYTES, MAX_CODE_LINES};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PatchLine {
    Context(Arc<str>),
    Added(Arc<str>),
    Removed(Arc<str>),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hunk {
    pub old_start: usize,
    pub old_count: usize,
    pub new_start: usize,
    pub new_count: usize,
    pub lines: Vec<PatchLine>,
    pub old_no_newline: bool,
    pub new_no_newline: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilePatch {
    pub old_path: Arc<str>,
    pub new_path: Arc<str>,
    pub hunks: Vec<Hunk>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Patch {
    pub files: Vec<FilePatch>,
}

impl Patch {
    pub fn parse(text: &str) -> Result<Self, String> {
        let lines: Vec<_> = text.lines().collect();
        let mut at = 0;
        let mut files = Vec::new();
        while at < lines.len() {
            if !lines[at].starts_with("--- ") {
                if lines[at].starts_with("GIT binary patch")
                    || lines[at].starts_with("Binary files ")
                {
                    return Err(format!(
                        "patch line {}: binary patches cannot be shown as editable code",
                        at + 1
                    ));
                }
                at += 1;
                continue;
            }
            let old_path = path(&lines[at][4..]);
            at += 1;
            let new_path = lines
                .get(at)
                .and_then(|line| line.strip_prefix("+++ "))
                .ok_or_else(|| {
                    format!(
                        "patch line {}: expected +++ new-file header after --- old-file",
                        at + 1
                    )
                })?;
            let new_path = path(new_path);
            at += 1;
            let mut hunks = Vec::new();
            while at < lines.len()
                && !lines[at].starts_with("--- ")
                && !lines[at].starts_with("diff --git ")
            {
                if !lines[at].starts_with("@@ ") {
                    if lines[at].starts_with(['+', '-', ' ', '\\']) {
                        return Err(format!(
                            "patch line {}: source line outside a declared hunk",
                            at + 1
                        ));
                    }
                    at += 1;
                    continue;
                }
                let header = lines[at];
                let mut ranges = header
                    .strip_prefix("@@ ")
                    .expect("hunk prefix")
                    .split_whitespace();
                let (old_start, old_count) = range(ranges.next().unwrap_or(""), '-')?;
                let (new_start, new_count) = range(ranges.next().unwrap_or(""), '+')?;
                if ranges.next() != Some("@@") {
                    return Err(format!(
                        "patch line {}: invalid unified hunk header '{header}'",
                        at + 1
                    ));
                }
                at += 1;
                let mut hunk = Hunk {
                    old_start,
                    old_count,
                    new_start,
                    new_count,
                    lines: Vec::new(),
                    old_no_newline: false,
                    new_no_newline: false,
                };
                let (mut old, mut new) = (0, 0);
                while at < lines.len() {
                    if lines[at] == "\\ No newline at end of file" {
                        match hunk.lines.last() {
                            Some(PatchLine::Context(_)) if old == old_count && new == new_count => {
                                hunk.old_no_newline = true;
                                hunk.new_no_newline = true;
                            }
                            Some(PatchLine::Removed(_)) if old == old_count => {
                                hunk.old_no_newline = true
                            }
                            Some(PatchLine::Added(_)) if new == new_count => {
                                hunk.new_no_newline = true
                            }
                            Some(_) => {
                                return Err(format!(
                                    "patch line {}: no-newline marker must follow the file's final source line",
                                    at + 1
                                ));
                            }
                            None => {
                                return Err(format!(
                                    "patch line {}: no-newline marker has no preceding source line",
                                    at + 1
                                ));
                            }
                        }
                        at += 1;
                        continue;
                    }
                    if old == old_count && new == new_count {
                        break;
                    }
                    let line = lines[at];
                    let value: Arc<str> = line.get(1..).unwrap_or("").into();
                    match line.as_bytes().first() {
                        Some(b' ') => {
                            old += 1;
                            new += 1;
                            hunk.lines.push(PatchLine::Context(value));
                        }
                        Some(b'-') => {
                            old += 1;
                            hunk.lines.push(PatchLine::Removed(value));
                        }
                        Some(b'+') => {
                            new += 1;
                            hunk.lines.push(PatchLine::Added(value));
                        }
                        _ => break,
                    }
                    if old > old_count || new > new_count {
                        return Err(format!(
                            "patch line {}: hunk body exceeds declared line counts",
                            at + 1
                        ));
                    }
                    at += 1;
                }
                if old != old_count || new != new_count {
                    return Err(format!(
                        "patch hunk -{old_start},{old_count} +{new_start},{new_count}: expected {old_count} old and {new_count} new lines, found {old} and {new}"
                    ));
                }
                hunks.push(hunk);
            }
            if hunks.is_empty() {
                return Err(format!(
                    "patch for '{new_path}' has no text hunks; binary/metadata-only patches are not source edits"
                ));
            }
            files.push(FilePatch {
                old_path: old_path.into(),
                new_path: new_path.into(),
                hunks,
            });
        }
        if files.is_empty() {
            return Err(
                "expected unified patch with ---/+++ file headers and @@ -old +new @@ hunks".into(),
            );
        }
        Ok(Self { files })
    }
    pub fn select(&self, file: Option<&str>) -> Result<&FilePatch, String> {
        if self.files.len() == 1 && file.is_none() {
            return Ok(&self.files[0]);
        }
        let Some(file) = file else {
            return Err(format!(
                "patch contains {} files; choose one with file=\"path\"",
                self.files.len()
            ));
        };
        let file = file.strip_prefix("./").unwrap_or(file);
        let matches: Vec<_> = self
            .files
            .iter()
            .filter(|entry| entry.new_path.as_ref() == file || entry.old_path.as_ref() == file)
            .collect();
        if matches.len() == 1 {
            Ok(matches[0])
        } else {
            Err(format!(
                "patch file '{file}' does not select exactly one file; available files: {}",
                self.files
                    .iter()
                    .map(|entry| entry.new_path.as_ref())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        }
    }
}
impl FilePatch {
    pub fn apply(&self, source: &str) -> Result<String, String> {
        let input: Vec<_> = source.lines().collect();
        let mut result = Vec::new();
        let mut cursor = 0;
        let mut output_no_newline = !source.ends_with('\n') && !source.is_empty();
        for hunk in &self.hunks {
            let start = if hunk.old_count == 0 {
                hunk.old_start
            } else {
                hunk.old_start
                    .checked_sub(1)
                    .ok_or("patch old source line must start at 1 unless count is zero")?
            };
            if start < cursor || start > input.len() {
                return Err(format!(
                    "patch hunk for '{}': old line {} is out of order or beyond {} source lines",
                    self.old_path,
                    hunk.old_start,
                    input.len()
                ));
            }
            result.extend_from_slice(&input[cursor..start]);
            cursor = start;
            let expected_new = if hunk.new_count == 0 {
                hunk.new_start
            } else {
                hunk.new_start
                    .checked_sub(1)
                    .ok_or("patch new source line must start at 1 unless count is zero")?
            };
            if result.len() != expected_new {
                return Err(format!(
                    "patch hunk for '{}': new line {} disagrees with previous hunk counts",
                    self.new_path, hunk.new_start
                ));
            }
            for line in &hunk.lines {
                match line {
                    PatchLine::Context(expected) | PatchLine::Removed(expected) => {
                        if input.get(cursor).copied() != Some(expected.as_ref()) {
                            return Err(format!(
                                "patch context mismatch in '{}' at line {}: expected {:?}, found {:?}; reload the matching base file",
                                self.old_path,
                                cursor + 1,
                                expected,
                                input.get(cursor)
                            ));
                        }
                        if matches!(line, PatchLine::Context(_)) {
                            result.push(expected.as_ref());
                        }
                        cursor += 1;
                    }
                    PatchLine::Added(value) => result.push(value.as_ref()),
                }
            }
            if hunk.old_no_newline && cursor != input.len() {
                return Err(
                    "patch no-newline marker does not refer to the end of the base file".into(),
                );
            }
            if cursor == input.len() {
                let actual_no_newline = !source.is_empty() && !source.ends_with('\n');
                if hunk.old_count > 0 && hunk.old_no_newline != actual_no_newline {
                    return Err(format!(
                        "patch context mismatch in '{}': base file's final newline differs from the patch; reload the matching base file",
                        self.old_path
                    ));
                }
                output_no_newline = hunk.new_no_newline;
            }
            if result.len() > MAX_CODE_LINES {
                return Err("patched source exceeds code line limit".into());
            }
        }
        result.extend_from_slice(&input[cursor..]);
        let newline = if source.contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        let bytes = result
            .iter()
            .try_fold(0usize, |total, line| {
                total.checked_add(line.len() + newline.len())
            })
            .ok_or("patched code length overflow")?;
        if bytes > MAX_CODE_BYTES {
            return Err("patched source exceeds code byte limit".into());
        }
        let mut text = result.join(newline);
        if !result.is_empty() && !output_no_newline {
            text.push_str(newline);
        }
        Ok(text)
    }
}
fn path(value: &str) -> &str {
    let value = value.split('\t').next().unwrap_or(value);
    value
        .strip_prefix("a/")
        .or_else(|| value.strip_prefix("b/"))
        .unwrap_or(value)
}
fn range(value: &str, prefix: char) -> Result<(usize, usize), String> {
    let value = value
        .strip_prefix(prefix)
        .ok_or("invalid unified patch hunk range")?;
    let (start, count) = value.split_once(',').unwrap_or((value, "1"));
    Ok((
        start.parse().map_err(|_| "invalid patch start line")?,
        count.parse().map_err(|_| "invalid patch line count")?,
    ))
}
