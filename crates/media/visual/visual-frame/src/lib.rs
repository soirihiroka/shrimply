//! Owned decoded media frames and backend-specific GPU imports.
use std::sync::Arc;

use ffmpeg_next::{frame::Video, sys};

pub mod math;

#[cfg(feature = "cuda")]
pub mod cuda;

/// An immutable FFmpeg frame reference. Cloning its Arc never copies pixels.
/// Native frame metadata and hardware-pool ownership survive until the last reader.
pub struct VideoFrame {
    native: Video,
    bytes: u64,
}

impl VideoFrame {
    pub fn new(native: Video) -> Result<Self, String> {
        let raw = unsafe { &*native.as_ptr() };
        if raw.width <= 0 || raw.height <= 0 {
            return Err("decoded video frame dimensions must be positive".into());
        }
        let format = if let Some(context) = unsafe { raw.hw_frames_ctx.as_ref() } {
            unsafe { (*context.data.cast::<sys::AVHWFramesContext>()).sw_format }
        } else {
            native.format().into()
        };
        // Payload accounting, not a claim about driver allocation size or tiling.
        let bytes = unsafe { sys::av_image_get_buffer_size(format, raw.width, raw.height, 1) };
        if bytes < 0 {
            return Err(format!(
                "measure decoded video frame: {}",
                ffmpeg_next::Error::from(bytes)
            ));
        }
        Ok(Self {
            native,
            bytes: bytes as u64,
        })
    }

    pub fn native(&self) -> &Video {
        &self.native
    }

    pub fn bytes(&self) -> u64 {
        self.bytes
    }
}

/// Import into a backend's own GPU representation without a CPU pixel transfer.
/// Implementations retain the source through the final GPU read and own the
/// synchronization needed before access and before returning decoder surfaces.
/// Context and storage remain statically typed; GPU handles are not interchangeable.
pub trait GpuFrame: Sized {
    type Context;

    fn import(context: &mut Self::Context, source: Arc<VideoFrame>) -> Result<Self, String>;
    fn source(&self) -> &Arc<VideoFrame>;
}
