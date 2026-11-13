//! LSP UTF-16 positions and file URIs; the language AST continues to use byte spans.
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub(crate) struct Document {
    pub text: String,
    pub version: i64,
    starts: Vec<(usize, usize)>,
    unicode: Vec<(usize, usize)>,
}
impl Document {
    pub fn new(text: String, version: i64) -> Self {
        let mut starts = vec![(0, 0)];
        let mut unicode = Vec::new();
        let mut adjustment = 0;
        let mut characters = text.char_indices().peekable();
        while let Some((offset, ch)) = characters.next() {
            if ch == '\r' || ch == '\n' {
                let end = if ch == '\r' && characters.peek().is_some_and(|(_, ch)| *ch == '\n') {
                    characters.next().expect("peeked LF").0 + 1
                } else {
                    offset + 1
                };
                starts.push((end, adjustment));
            } else if ch.len_utf8() != ch.len_utf16() {
                adjustment += ch.len_utf8() - ch.len_utf16();
                unicode.push((offset + ch.len_utf8(), adjustment));
            }
        }
        Self {
            text,
            version,
            starts,
            unicode,
        }
    }
    pub fn offset(&self, position: &Value) -> Result<usize, String> {
        let line = position["line"]
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .ok_or("position.line must be a nonnegative integer")?;
        let character = position["character"]
            .as_u64()
            .ok_or("position.character must be a nonnegative UTF-16 offset")?;
        let start = self
            .starts
            .get(line)
            .ok_or("position.line exceeds document")?
            .0;
        let end = self
            .starts
            .get(line + 1)
            .map(|(offset, _)| *offset)
            .unwrap_or(self.text.len());
        let text = self.text[start..end].trim_end_matches(['\r', '\n']);
        let mut units = 0;
        for (offset, ch) in text.char_indices() {
            if units == character {
                return Ok(start + offset);
            }
            units += ch.len_utf16() as u64;
            if units > character {
                return Err("position splits a UTF-16 surrogate pair".into());
            }
        }
        if units == character {
            Ok(start + text.len())
        } else {
            Err("position.character exceeds line".into())
        }
    }
    pub fn position(&self, offset: usize) -> Value {
        let mut offset = offset.min(self.text.len());
        while !self.text.is_char_boundary(offset) {
            offset -= 1;
        }
        // CRLF is one line break. Offsets inside the pair map to the
        // preceding line's end, matching LSP's UTF-16 line model.
        if offset > 0
            && self.text.as_bytes().get(offset - 1) == Some(&b'\r')
            && self.text.as_bytes().get(offset) == Some(&b'\n')
        {
            offset -= 1;
        }
        let line = self
            .starts
            .partition_point(|(start, _)| *start <= offset)
            .saturating_sub(1);
        let adjustment = self
            .unicode
            .partition_point(|(end, _)| *end <= offset)
            .checked_sub(1)
            .map_or(0, |index| self.unicode[index].1);
        let character = offset - self.starts[line].0 - (adjustment - self.starts[line].1);
        json!({"line":line,"character":character})
    }
    /// Split a source span using the canonical document line index, including
    /// bare CR and CRLF. Semantic tokens cannot cross LSP line boundaries.
    pub fn span_lines(&self, start: usize, end: usize) -> impl Iterator<Item = (usize, &str)> {
        let first = self
            .starts
            .partition_point(|(offset, _)| *offset <= start)
            .saturating_sub(1);
        self.starts
            .iter()
            .enumerate()
            .skip(first)
            .take_while(move |(_, (offset, _))| *offset < end)
            .filter_map(move |(line, (offset, _))| {
                let begin = start.max(*offset);
                let finish = end.min(
                    self.starts
                        .get(line + 1)
                        .map_or(self.text.len(), |(offset, _)| *offset),
                );
                let text = self.text[begin..finish].trim_end_matches(['\r', '\n']);
                (!text.is_empty()).then_some((begin, text))
            })
    }
    pub fn range(&self, span: sanim_lang::core::Span) -> Value {
        json!({"start":self.position(span.start),"end":self.position(span.end.max(span.start))})
    }
    /// All changes in a notification apply in order, atomically, to the previous text.
    pub fn changed(&self, version: i64, changes: &[Value]) -> Result<Self, String> {
        if version <= self.version {
            return Err("ignored out-of-order document version".into());
        }
        let mut document = self.clone();
        for change in changes {
            let replacement = change["text"]
                .as_str()
                .ok_or("content change requires text")?;
            if let Some(range) = change.get("range") {
                let start = document.offset(&range["start"])?;
                let end = document.offset(&range["end"])?;
                if end < start {
                    return Err("change range ends before it starts".into());
                }
                if let Some(length) = change.get("rangeLength").and_then(Value::as_u64)
                    && document.text[start..end].encode_utf16().count() as u64 != length
                {
                    return Err("change rangeLength does not match its UTF-16 range".into());
                }
                document.text.replace_range(start..end, replacement);
            } else {
                document.text = replacement.into();
            }
            document = Self::new(document.text, version);
        }
        document.version = version;
        Ok(document)
    }
}

pub(crate) fn path(uri: &str) -> Result<PathBuf, String> {
    let encoded = uri
        .strip_prefix("file://")
        .ok_or("only file:// document URIs are supported")?;
    let encoded = if let Some(path) = encoded.strip_prefix("localhost/") {
        format!("/{path}")
    } else if encoded.starts_with('/') {
        encoded.to_owned()
    } else {
        return Err("remote file URI hosts are unsupported".into());
    };
    let mut bytes = Vec::new();
    let input = encoded.as_bytes();
    let mut i = 0;
    while i < input.len() {
        if input[i] == b'%' {
            let pair = input
                .get(i + 1..i + 3)
                .ok_or("incomplete file URI percent escape")?;
            let pair = std::str::from_utf8(pair).map_err(|_| "invalid file URI percent escape")?;
            bytes
                .push(u8::from_str_radix(pair, 16).map_err(|_| "invalid file URI percent escape")?);
            i += 3;
        } else {
            bytes.push(input[i]);
            i += 1;
        }
    }
    let decoded = String::from_utf8(bytes).map_err(|_| "file URI must contain UTF-8")?;
    if decoded.contains('\0') {
        return Err("file URI cannot contain NUL".into());
    }
    #[cfg(windows)]
    let decoded = decoded
        .strip_prefix('/')
        .filter(|path| path.as_bytes().get(1) == Some(&b':'))
        .unwrap_or(&decoded)
        .to_owned();
    Ok(PathBuf::from(decoded))
}

pub(crate) fn uri(path: &Path) -> Result<String, String> {
    let absolute = std::path::absolute(path).map_err(|error| error.to_string())?;
    let path = absolute
        .to_str()
        .ok_or("file path must be UTF-8")?
        .replace('\\', "/");
    let mut uri = if path.starts_with('/') {
        "file://".to_owned()
    } else {
        "file:///".to_owned()
    };
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b':' | b'-' | b'_' | b'.' | b'~') {
            uri.push(byte as char);
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    Ok(uri)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn positions_round_trip_utf16_and_crlf_without_splitting_surrogates() {
        let document = Document::new("a😊b\r\néx\n".into(), 1);
        assert_eq!(
            document.offset(&json!({"line":0,"character":3})).unwrap(),
            5
        );
        assert!(document.offset(&json!({"line":0,"character":2})).is_err());
        assert_eq!(document.position(5), json!({"line":0,"character":3}));
        assert_eq!(document.position(7), json!({"line":0,"character":4}));
        assert_eq!(
            document.offset(&json!({"line":1,"character":1})).unwrap(),
            10
        );
        let edited=document.changed(2,&[json!({"range":{"start":{"line":0,"character":1},"end":{"line":0,"character":3}},"rangeLength":2,"text":"x"})]).unwrap();
        assert_eq!(edited.text, "axb\r\néx\n");
        assert!(document.changed(2,&[json!({"text":"changed"}),json!({"range":{"start":{"line":9,"character":0},"end":{"line":9,"character":0}},"text":"bad"})]).is_err());
        assert_eq!(document.text, "a😊b\r\néx\n");
    }
    #[test]
    fn bare_cr_lines_use_the_same_position_model_as_the_parser() {
        let document = Document::new("a\r😊\nb".into(), 1);
        assert_eq!(document.position(6), json!({"line":1,"character":2}));
        assert_eq!(document.position(7), json!({"line":2,"character":0}));
        assert_eq!(
            document.offset(&json!({"line":1,"character":2})).unwrap(),
            6
        );
    }
    #[test]
    fn local_file_uri_round_trips_spaces_unicode_and_escapes() {
        let path = std::env::temp_dir().join("Sanim # 你好.sanim");
        assert_eq!(super::path(&uri(&path).unwrap()).unwrap(), path);
        assert!(super::path("https://example.com/a.sanim").is_err());
        assert!(super::path("file:///bad%Q0").is_err());
        assert!(super::path("file:///bad%00name").is_err());
    }
}
