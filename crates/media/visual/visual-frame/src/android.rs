//! Bounded decoder surfaces. Images stay acquired until the final GPU reader
//! releases its VideoFrame; retaining an AHardwareBuffer alone is insufficient
//! because the producer could otherwise overwrite its pixels.
use ndk::{
    hardware_buffer::{HardwareBuffer, HardwareBufferRef, HardwareBufferUsage},
    media::image_reader::{AcquireResult, Image, ImageFormat, ImageReader},
    native_window::NativeWindow,
};
use std::{
    sync::{
        Arc, Condvar, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const IMAGE_TIMEOUT: Duration = Duration::from_secs(5);
const CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(10);
const NANOSECONDS_PER_MICROSECOND: i64 = 1_000;

unsafe extern "C" {
    fn av_mediacodec_render_buffer_at_time(buffer: *mut std::ffi::c_void, time: i64) -> i32;
}

pub(crate) struct DecoderOutput {
    pool: Arc<AndroidFramePool>,
    pub(crate) image: OnceLock<Result<AndroidImage, String>>,
}

impl DecoderOutput {
    pub(crate) fn new(pool: Arc<AndroidFramePool>) -> Self {
        Self {
            pool,
            image: OnceLock::new(),
        }
    }
}

impl crate::VideoFrame {
    /// Only the selected output reaches ImageReader. Dropping intervening codec
    /// frames discards them without scheduling hardware images or waiting on the
    /// consumer, preserving the shared decoder's exact frame-selection policy.
    pub fn prepare_android(&self, cancelled: impl Fn() -> bool) -> Result<(), String> {
        let output = self
            .android
            .as_ref()
            .ok_or("Frame has no Android decoder output")?;
        output
            .image
            .get_or_init(|| {
                if cancelled() {
                    return Err("preview request superseded".into());
                }
                let timestamp = unsafe { ffmpeg_next::sys::av_gettime_relative() }
                    .checked_mul(NANOSECONDS_PER_MICROSECOND)
                    .ok_or("Decoder render clock overflow")?;
                let buffer = unsafe { (*self.native.as_ptr()).data[3] };
                let result =
                    unsafe { av_mediacodec_render_buffer_at_time(buffer.cast(), timestamp) };
                if result < 0 {
                    return Err(format!(
                        "Render decoded hardware buffer: {}",
                        ffmpeg_next::Error::from(result)
                    ));
                }
                let image = output
                    .pool
                    .acquire(timestamp, cancelled)?
                    .ok_or("preview request superseded")?;
                let description = image.buffer().describe();
                if description.width < self.native.width()
                    || description.height < self.native.height()
                {
                    return Err("Decoded hardware buffer is smaller than the video frame".into());
                }
                Ok(image)
            })
            .as_ref()
            .map(|_| ())
            .map_err(Clone::clone)
    }
}

struct NativeReader(ImageReader);
// Reader operations and destruction are serialized by AndroidFramePool's mutex.
// The listener only signals its own independent notification state.
unsafe impl Send for NativeReader {}

type AllocationSize = dyn Fn(&HardwareBuffer) -> Result<u64, String> + Send + Sync;

pub struct AndroidFramePool {
    reader: Mutex<NativeReader>,
    ready: Arc<(Mutex<u64>, Condvar)>,
    allocation_size: Box<AllocationSize>,
    producer_claimed: AtomicBool,
}

impl AndroidFramePool {
    /// `allocation_size` queries the consuming GPU's native allocation size.
    /// This keeps opaque decoder allocations in the same memory budget as other
    /// decoded frames, without estimating an unknown private pixel format.
    pub fn new(
        width: u32,
        height: u32,
        maximum_images: u32,
        allocation_size: impl Fn(&HardwareBuffer) -> Result<u64, String> + Send + Sync + 'static,
    ) -> Result<Arc<Self>, String> {
        if width == 0 || height == 0 || maximum_images < 2 {
            return Err(
                "Decoder image pool requires positive dimensions and at least two images".into(),
            );
        }
        let mut reader = ImageReader::new_with_usage(
            i32::try_from(width).map_err(|_| "Decoder width exceeds Android limits")?,
            i32::try_from(height).map_err(|_| "Decoder height exceeds Android limits")?,
            ImageFormat::PRIVATE,
            HardwareBufferUsage::GPU_SAMPLED_IMAGE,
            i32::try_from(maximum_images)
                .map_err(|_| "Decoder image limit exceeds Android limits")?,
        )
        .map_err(|error| format!("Create decoder ImageReader: {error}"))?;
        let ready = Arc::new((Mutex::new(0_u64), Condvar::new()));
        let notification = ready.clone();
        reader
            .set_image_listener(Box::new(move |_| {
                let (generation, ready) = &*notification;
                let mut generation = generation
                    .lock()
                    .expect("Decoder image notification poisoned");
                *generation = generation
                    .checked_add(1)
                    .expect("Decoder image generation overflow");
                ready.notify_all();
            }))
            .map_err(|error| format!("Observe decoder images: {error}"))?;
        Ok(Arc::new(Self {
            reader: Mutex::new(NativeReader(reader)),
            ready,
            allocation_size: Box::new(allocation_size),
            producer_claimed: AtomicBool::new(false),
        }))
    }

    /// A pool belongs to one decoder for its entire lifetime, including frames
    /// retained after that decoder closes. Reopening requires a fresh pool.
    pub fn claim_decoder(&self) -> Result<(), String> {
        self.producer_claimed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| ())
            .map_err(|_| "Each MediaCodec session requires its own hardware image pool".into())
    }

    pub fn window(&self) -> Result<NativeWindow, String> {
        self.reader
            .lock()
            .expect("Decoder image reader poisoned")
            .0
            .window()
            .map_err(|error| format!("Get decoder output window: {error}"))
    }

    /// Match the explicit render timestamp, discarding late images from a
    /// superseded seek. Cancellation never turns an old image into a new frame.
    pub fn acquire(
        self: &Arc<Self>,
        timestamp: i64,
        cancelled: impl Fn() -> bool,
    ) -> Result<Option<AndroidImage>, String> {
        let deadline = Instant::now() + IMAGE_TIMEOUT;
        loop {
            if cancelled() {
                return Ok(None);
            }
            let (generation, ready) = &*self.ready;
            let observed = *generation
                .lock()
                .expect("Decoder image notification poisoned");
            let next = self
                .reader
                .lock()
                .expect("Decoder image reader poisoned")
                .0
                .acquire_next_image()
                .map_err(|error| format!("Acquire decoder image: {error}"))?;
            match next {
                AcquireResult::Image(image) => {
                    let actual = image.timestamp().map_err(|error| format!("Read decoder image timestamp: {error}"))?;
                    if actual != timestamp { continue; }
                    let buffer = image.hardware_buffer().map_err(|error| format!("Get decoder hardware buffer: {error}"))?.acquire();
                    let bytes = (self.allocation_size)(&buffer)?;
                    if bytes == 0 { return Err("Decoder hardware buffer has no allocation".into()); }
                    return Ok(Some(AndroidImage { _image: image, buffer, _pool: self.clone(), bytes }));
                }
                AcquireResult::MaxImagesAcquired => return Err("Decoder image pool is full; release retained GPU frames before decoding more images".into()),
                AcquireResult::NoBufferAvailable => {},
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err("Timed out waiting for the decoded hardware image".into());
            }
            let generation = generation
                .lock()
                .expect("Decoder image notification poisoned");
            if *generation == observed {
                drop(
                    ready
                        .wait_timeout(generation, remaining.min(CANCEL_POLL_INTERVAL))
                        .expect("Wait for decoder image"),
                );
            }
        }
    }
}

pub struct AndroidImage {
    // Field order matters: return the acquired image before deleting its reader.
    _image: Image,
    buffer: HardwareBufferRef,
    _pool: Arc<AndroidFramePool>,
    bytes: u64,
}
// Acquired images are immutable. No pixel mapping or mutable NDK image handle
// escapes; native buffer references and the pool outlive all GPU submissions.
unsafe impl Send for AndroidImage {}
unsafe impl Sync for AndroidImage {}

impl AndroidImage {
    pub fn buffer(&self) -> &HardwareBuffer {
        &self.buffer
    }
    pub fn bytes(&self) -> u64 {
        self.bytes
    }
    pub fn crop_rect(&self) -> Result<[i32; 4], String> {
        let crop = self
            ._image
            .crop_rect()
            .map_err(|error| format!("Read decoder crop: {error}"))?;
        Ok([crop.left, crop.top, crop.right, crop.bottom])
    }
}
