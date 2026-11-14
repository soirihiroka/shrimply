//! Owned decoded media frames and backend-specific GPU imports.
use std::sync::Arc;

use ffmpeg_next::{frame::Video, sys};

#[cfg(target_os = "android")]
pub mod android;
pub mod math;

#[cfg(feature = "cuda")]
pub mod cuda;

/// An immutable FFmpeg frame reference. Cloning its Arc never copies pixels.
/// Native frame metadata and hardware-pool ownership survive until the last reader.
pub struct VideoFrame {
    native: Video,
    bytes: u64,
    #[cfg(target_os = "android")]
    android: Option<android::DecoderOutput>,
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
            #[cfg(target_os = "android")]
            android: None,
        })
    }

    #[cfg(target_os = "android")]
    pub fn from_android(
        native: Video,
        pool: Arc<android::AndroidFramePool>,
    ) -> Result<Self, String> {
        if native.format() != ffmpeg_next::format::Pixel::MEDIACODEC
            || native.width() == 0
            || native.height() == 0
            || unsafe { (*native.as_ptr()).data[3].is_null() }
        {
            return Err("Android hardware image requires a valid MediaCodec frame".into());
        }
        Ok(Self {
            native,
            bytes: 0,
            android: Some(android::DecoderOutput::new(pool)),
        })
    }

    #[cfg(target_os = "android")]
    pub fn android(&self) -> Option<&android::AndroidImage> {
        self.android.as_ref()?.image.get()?.as_ref().ok()
    }

    pub fn native(&self) -> &Video {
        &self.native
    }

    pub fn bytes(&self) -> u64 {
        #[cfg(target_os = "android")]
        if self.android.is_some() {
            return self
                .android()
                .expect("acquire selected decoder image before accounting its allocation")
                .bytes();
        }
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
