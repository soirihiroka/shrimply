use std::{ptr, sync::Arc};

use ffmpeg_next::{self as ffmpeg, sys};

#[derive(Clone)]
pub enum DecoderBackend {
    Hardware(HardwareDevice),
    #[cfg(target_os = "android")]
    Android {
        device: HardwareDevice,
        pool: Arc<shrimply_visual_frame::android::AndroidFramePool>,
    },
}

/// Retains the exact native device used by the consuming GPU backend.
#[derive(Clone)]
pub struct HardwareDevice(Arc<DeviceReference>);

struct DeviceReference(*mut sys::AVBufferRef);

// FFmpeg device references are immutable here and its AVBuffer reference count is atomic.
unsafe impl Send for DeviceReference {}
unsafe impl Sync for DeviceReference {}

impl Drop for DeviceReference {
    fn drop(&mut self) {
        unsafe { sys::av_buffer_unref(&mut self.0) };
    }
}

impl HardwareDevice {
    /// # Safety
    /// `reference` must be a live AVHWDeviceContext reference for this call.
    /// The returned value retains its own reference, independent of the caller.
    pub unsafe fn retain(reference: *mut sys::AVBufferRef) -> Result<Self, String> {
        if reference.is_null() {
            return Err("video decoder device is null".into());
        }
        let reference = unsafe { sys::av_buffer_ref(reference) };
        if reference.is_null() {
            return Err("could not retain video decoder device".into());
        }
        let device = Self(Arc::new(DeviceReference(reference)));
        if pixel_format(device.kind()) == sys::AVPixelFormat::AV_PIX_FMT_NONE {
            return Err(format!(
                "Unsupported video decoder device {:?}",
                device.kind()
            ));
        }
        Ok(device)
    }

    fn kind(&self) -> sys::AVHWDeviceType {
        unsafe { (*(*self.0.0).data.cast::<sys::AVHWDeviceContext>()).type_ }
    }
}

impl DecoderBackend {
    #[cfg(feature = "cuda")]
    pub fn cuda() -> Result<Self, String> {
        Self::device(sys::AVHWDeviceType::AV_HWDEVICE_TYPE_CUDA)
    }

    fn device(kind: sys::AVHWDeviceType) -> Result<Self, String> {
        let cuda = kind == sys::AVHWDeviceType::AV_HWDEVICE_TYPE_CUDA;
        let mut options = ffmpeg::Dictionary::new();
        if cuda {
            options.set("primary_ctx", "1");
        }
        #[cfg(feature = "cuda")]
        let name = cuda.then(|| {
            std::ffi::CString::new(shrimply_gpu_cuda::selected_device().to_string())
                .expect("numeric CUDA device")
        });
        #[cfg(not(feature = "cuda"))]
        let name: Option<std::ffi::CString> = None;
        let mut reference = ptr::null_mut();
        let result = unsafe {
            sys::av_hwdevice_ctx_create(
                &mut reference,
                kind,
                name.as_ref().map_or(ptr::null(), |name| name.as_ptr()),
                options.as_mut_ptr(),
                0,
            )
        };
        if result < 0 {
            return Err(format!(
                "create {kind:?} device: {}",
                ffmpeg::Error::from(result)
            ));
        }
        let retained = unsafe { HardwareDevice::retain(reference) };
        unsafe { sys::av_buffer_unref(&mut reference) };
        retained.map(Self::Hardware)
    }

    #[cfg(target_os = "macos")]
    pub fn videotoolbox() -> Result<Self, String> {
        Self::device(sys::AVHWDeviceType::AV_HWDEVICE_TYPE_VIDEOTOOLBOX)
    }

    /// Hardware decoding for consumers which need addressable pixels. A MediaCodec
    /// device without an output surface delivers its decoded YUV buffers to FFmpeg.
    pub fn hardware_for_readback(parameters: &ffmpeg::codec::Parameters) -> Result<Self, String> {
        use sys::AVHWDeviceType::*;
        #[cfg(target_os = "android")]
        let kinds = [AV_HWDEVICE_TYPE_MEDIACODEC];
        #[cfg(target_os = "macos")]
        let kinds = [AV_HWDEVICE_TYPE_VIDEOTOOLBOX];
        #[cfg(target_os = "windows")]
        let kinds = [AV_HWDEVICE_TYPE_D3D11VA, AV_HWDEVICE_TYPE_CUDA];
        #[cfg(target_os = "linux")]
        let kinds = [AV_HWDEVICE_TYPE_CUDA, AV_HWDEVICE_TYPE_VAAPI];
        #[cfg(not(any(
            target_os = "linux",
            target_os = "windows",
            target_os = "android",
            target_os = "macos"
        )))]
        let kinds = [];
        let mut reasons = Vec::new();
        for kind in kinds {
            if Self::codec(parameters, kind).is_none() {
                continue;
            }
            match Self::device(kind) {
                Ok(device) => return Ok(device),
                Err(error) => reasons.push(error),
            }
        }
        Err(format!(
            "No usable hardware video decoder for {:?}: {reasons:?}",
            parameters.id()
        ))
    }

    fn codec(
        parameters: &ffmpeg::codec::Parameters,
        kind: sys::AVHWDeviceType,
    ) -> Option<ffmpeg::Codec> {
        let mut iterator = ptr::null_mut();
        loop {
            let codec = unsafe { sys::av_codec_iterate(&mut iterator) };
            if codec.is_null() {
                return None;
            }
            if unsafe { sys::av_codec_is_decoder(codec) } == 0
                || unsafe { (*codec).id } != parameters.id().into()
            {
                continue;
            }
            for index in 0.. {
                let config = unsafe { sys::avcodec_get_hw_config(codec, index).as_ref() };
                let Some(config) = config else { break };
                if config.device_type == kind
                    && config.pix_fmt == pixel_format(kind)
                    && config.methods & sys::AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX as i32 != 0
                {
                    return Some(unsafe { ffmpeg::Codec::wrap(codec) });
                }
            }
        }
    }

    pub(crate) fn open(
        &self,
        parameters: &ffmpeg::codec::Parameters,
        time_base: ffmpeg::Rational,
    ) -> Result<ffmpeg::decoder::Video, String> {
        let device = match self {
            Self::Hardware(device) => device,
            #[cfg(target_os = "android")]
            Self::Android { device, pool } => {
                pool.claim_decoder()?;
                device
            }
        };
        let codec = Self::codec(parameters, device.kind()).ok_or_else(|| {
            format!(
                "No {:?} hardware decoder available for {:?}",
                device.kind(),
                parameters.id()
            )
        })?;
        let mut context = ffmpeg::codec::Context::from_parameters(parameters.clone())
            .map_err(|error| error.to_string())?
            .decoder();
        context.set_packet_time_base(time_base);
        {
            let reference = unsafe { sys::av_buffer_ref(device.0.0) };
            if reference.is_null() {
                return Err("could not retain video decoder device".into());
            }
            unsafe {
                (*context.as_mut_ptr()).hw_device_ctx = reference;
                (*context.as_mut_ptr()).get_format = Some(hardware_format);
            }
        }
        let mut options = ffmpeg::Dictionary::new();
        if device.kind() == sys::AVHWDeviceType::AV_HWDEVICE_TYPE_MEDIACODEC {
            options.set("ndk_codec", "1");
        }
        context
            .open_as_with(codec, options)
            .and_then(|opened| opened.video())
            .map_err(|error| format!("open {} video decoder: {error}", codec.name()))
    }
}

fn pixel_format(kind: sys::AVHWDeviceType) -> sys::AVPixelFormat {
    match kind {
        sys::AVHWDeviceType::AV_HWDEVICE_TYPE_CUDA => sys::AVPixelFormat::AV_PIX_FMT_CUDA,
        sys::AVHWDeviceType::AV_HWDEVICE_TYPE_D3D11VA => sys::AVPixelFormat::AV_PIX_FMT_D3D11,
        sys::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI => sys::AVPixelFormat::AV_PIX_FMT_VAAPI,
        sys::AVHWDeviceType::AV_HWDEVICE_TYPE_MEDIACODEC => {
            sys::AVPixelFormat::AV_PIX_FMT_MEDIACODEC
        }
        sys::AVHWDeviceType::AV_HWDEVICE_TYPE_VIDEOTOOLBOX => {
            sys::AVPixelFormat::AV_PIX_FMT_VIDEOTOOLBOX
        }
        _ => sys::AVPixelFormat::AV_PIX_FMT_NONE,
    }
}

unsafe extern "C" fn hardware_format(
    context: *mut sys::AVCodecContext,
    formats: *const sys::AVPixelFormat,
) -> sys::AVPixelFormat {
    let kind = unsafe {
        (*(*(*context).hw_device_ctx)
            .data
            .cast::<sys::AVHWDeviceContext>())
        .type_
    };
    let requested = pixel_format(kind);
    let mut cursor = formats;
    while !cursor.is_null() && unsafe { *cursor } != sys::AVPixelFormat::AV_PIX_FMT_NONE {
        let format = unsafe { *cursor };
        if format == requested {
            return format;
        }
        cursor = unsafe { cursor.add(1) };
    }
    sys::AVPixelFormat::AV_PIX_FMT_NONE
}
