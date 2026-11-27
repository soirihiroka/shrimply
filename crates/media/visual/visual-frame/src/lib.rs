//! Owned decoded media frames and backend-specific GPU imports.
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use ffmpeg_next::{frame::Video, sys};

#[cfg(target_os = "android")]
pub mod android;
pub mod math;

#[cfg(feature = "cuda")]
pub mod cuda;

/// An immutable FFmpeg frame reference. Cloning its Arc never copies pixels.
/// Native frame metadata and hardware-pool ownership survive until the last reader.
pub struct VideoFrame {
    id: u64,
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
            id: next_frame_id(),
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
            id: next_frame_id(),
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

    /// Download device-backed pixels when a CPU consumer needs them. MediaCodec
    /// byte-buffer output already exposes addressable planes.
    pub fn download(&self) -> Result<Option<Video>, String> {
        let source = unsafe { &*self.native.as_ptr() };
        if source.hw_frames_ctx.is_null() {
            if source.data[0].is_null() {
                return Err(
                    "The decoder returned an opaque surface without a readback device".into(),
                );
            }
            return Ok(None);
        }
        let mut frame = Video::empty();
        let status =
            unsafe { sys::av_hwframe_transfer_data(frame.as_mut_ptr(), self.native.as_ptr(), 0) };
        if status < 0 {
            return Err(format!(
                "Download hardware video frame: {}",
                ffmpeg_next::Error::from(status)
            ));
        }
        // The transfer copies planes only. Preserve color range, matrix and HDR metadata.
        let status = unsafe { sys::av_frame_copy_props(frame.as_mut_ptr(), self.native.as_ptr()) };
        if status < 0 {
            return Err(format!(
                "Copy decoded frame properties: {}",
                ffmpeg_next::Error::from(status)
            ));
        }
        Ok(Some(frame))
    }

    /// Decoded content identity, independent of recycled codec/GPU allocations.
    pub fn id(&self) -> u64 {
        self.id
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

fn next_frame_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
        .expect("decoded frame identity overflow")
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
