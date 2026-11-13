//! LSP Content-Length framing. Stdout contains protocol messages only.
use serde_json::Value;
use std::io::{BufRead, Read, Write};
const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;
const MAX_HEADER_BYTES: usize = 8192;

pub(crate) fn read(input: &mut impl BufRead) -> Result<Option<Value>, String> {
    let mut length = None;
    let mut header_bytes = 0;
    loop {
        let mut line = String::new();
        let read = (&mut *input)
            .take((MAX_HEADER_BYTES - header_bytes + 1) as u64)
            .read_line(&mut line)
            .map_err(|error| error.to_string())?;
        if read == 0 {
            return if header_bytes == 0 {
                Ok(None)
            } else {
                Err("incomplete LSP header".into())
            };
        }
        header_bytes += read;
        if header_bytes > MAX_HEADER_BYTES {
            return Err("LSP header exceeds limit".into());
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
        let (name, value) = line
            .trim_end()
            .split_once(':')
            .ok_or("malformed LSP header")?;
        if name.eq_ignore_ascii_case("content-length") {
            if length.is_some() {
                return Err("duplicate Content-Length header".into());
            }
            length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| "invalid Content-Length")?,
            );
        }
    }
    let length = length.ok_or("missing Content-Length header")?;
    if length > MAX_MESSAGE_BYTES {
        return Err("LSP message exceeds limit".into());
    }
    let mut bytes = vec![0; length];
    input
        .read_exact(&mut bytes)
        .map_err(|error| format!("incomplete LSP body: {error}"))?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| format!("invalid LSP JSON: {error}"))
}

pub(crate) fn write(output: &mut impl Write, message: &Value) -> Result<(), String> {
    let bytes = serde_json::to_vec(message).map_err(|error| error.to_string())?;
    write!(output, "Content-Length: {}\r\n\r\n", bytes.len()).map_err(|error| error.to_string())?;
    output
        .write_all(&bytes)
        .map_err(|error| error.to_string())?;
    output.flush().map_err(|error| error.to_string())
}
