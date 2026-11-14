use std::{ptr, sync::Arc};

use ffmpeg_next::{self as ffmpeg, sys};

#[derive(Clone)]
pub enum DecoderBackend {
    Software,
    Hardware(HardwareDevice),
    PreferHardware(HardwareDevice),
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
        device.pixel_format()?;
        Ok(device)
    }

    fn kind(&self) -> sys::AVHWDeviceType {
        unsafe { (*(*self.0.0).data.cast::<sys::AVHWDeviceContext>()).type_ }
    }

    fn pixel_format(&self) -> Result<sys::AVPixelFormat, String> {
        match self.kind() {
            sys::AVHWDeviceType::AV_HWDEVICE_TYPE_CUDA => Ok(sys::AVPixelFormat::AV_PIX_FMT_CUDA),
            sys::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI => Ok(sys::AVPixelFormat::AV_PIX_FMT_VAAPI),
            sys::AVHWDeviceType::AV_HWDEVICE_TYPE_MEDIACODEC => {
                Ok(sys::AVPixelFormat::AV_PIX_FMT_MEDIACODEC)
            }
            sys::AVHWDeviceType::AV_HWDEVICE_TYPE_VIDEOTOOLBOX => {
                Ok(sys::AVPixelFormat::AV_PIX_FMT_VIDEOTOOLBOX)
            }
            kind => Err(format!("unsupported video decoder device {kind:?}")),
        }
    }
}

impl DecoderBackend {
    #[cfg(feature = "cuda")]
    pub fn cuda() -> Result<Self, String> {
        let mut reference = crate::cuda::create_cuda_device_context()?;
        let retained = unsafe { HardwareDevice::retain(reference) };
        unsafe { sys::av_buffer_unref(&mut reference) };
        retained.map(Self::Hardware)
    }

    #[cfg(target_os = "macos")]
    pub fn videotoolbox() -> Result<Self, String> {
        let mut reference = ptr::null_mut();
        let result = unsafe {
            sys::av_hwdevice_ctx_create(
                &mut reference,
                sys::AVHWDeviceType::AV_HWDEVICE_TYPE_VIDEOTOOLBOX,
                ptr::null(),
                ptr::null_mut(),
                0,
            )
        };
        if result < 0 {
            return Err(format!(
                "create VideoToolbox device: {}",
                ffmpeg::Error::from(result)
            ));
        }
        let retained = unsafe { HardwareDevice::retain(reference) };
        unsafe { sys::av_buffer_unref(&mut reference) };
        retained.map(Self::Hardware)
    }

    pub(crate) fn open(
        &self,
        parameters: &ffmpeg::codec::Parameters,
        time_base: ffmpeg::Rational,
    ) -> Result<ffmpeg::decoder::Video, String> {
        #[cfg(target_os = "android")]
        if let Self::Android { device, pool } = self {
            pool.claim_decoder()?;
            return Self::Hardware(device.clone()).open(parameters, time_base);
        }
        if let Self::PreferHardware(device) = self {
            return Self::Hardware(device.clone()).open(parameters, time_base).or_else(|error| {
                tracing::info!(%error, "hardware video decoding unavailable; using software decoding");
                Self::Software.open(parameters, time_base)
            });
        }
        let codec = match self {
            Self::Software => {
                ffmpeg::codec::decoder::find(parameters.id()).ok_or("video decoder not found")?
            }
            Self::PreferHardware(_) => unreachable!("preferred hardware handled above"),
            #[cfg(target_os = "android")]
            Self::Android { .. } => unreachable!("Android hardware handled above"),
            Self::Hardware(device) => {
                let mut iterator = ptr::null_mut();
                'codecs: loop {
                    let codec = unsafe { sys::av_codec_iterate(&mut iterator) };
                    if codec.is_null() {
                        return Err(format!(
                            "no {:?} video decoder available for {:?}",
                            device.kind(),
                            parameters.id()
                        ));
                    }
                    if unsafe { sys::av_codec_is_decoder(codec) } == 0
                        || unsafe { (*codec).id } != parameters.id().into()
                    {
                        continue;
                    }
                    for index in 0.. {
                        let config = unsafe { sys::avcodec_get_hw_config(codec, index) };
                        let Some(config) = (unsafe { config.as_ref() }) else {
                            break;
                        };
                        if config.device_type == device.kind()
                            && config.pix_fmt == device.pixel_format()?
                            && config.methods & sys::AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX as i32
                                != 0
                        {
                            break 'codecs unsafe { ffmpeg::Codec::wrap(codec) };
                        }
                    }
                }
            }
        };
        let mut context = ffmpeg::codec::Context::from_parameters(parameters.clone())
            .map_err(|error| error.to_string())?
            .decoder();
        context.set_packet_time_base(time_base);
        if let Self::Hardware(device) = self {
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
        if matches!(self, Self::Hardware(device) if device.kind() == sys::AVHWDeviceType::AV_HWDEVICE_TYPE_MEDIACODEC)
        {
            options.set("ndk_codec", "1");
        }
        context
            .open_as_with(codec, options)
            .and_then(|opened| opened.video())
            .map_err(|error| format!("open {} video decoder: {error}", codec.name()))
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
    let requested = match kind {
        sys::AVHWDeviceType::AV_HWDEVICE_TYPE_CUDA => sys::AVPixelFormat::AV_PIX_FMT_CUDA,
        sys::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI => sys::AVPixelFormat::AV_PIX_FMT_VAAPI,
        sys::AVHWDeviceType::AV_HWDEVICE_TYPE_MEDIACODEC => {
            sys::AVPixelFormat::AV_PIX_FMT_MEDIACODEC
        }
        sys::AVHWDeviceType::AV_HWDEVICE_TYPE_VIDEOTOOLBOX => {
            sys::AVPixelFormat::AV_PIX_FMT_VIDEOTOOLBOX
        }
        _ => return sys::AVPixelFormat::AV_PIX_FMT_NONE,
    };
    let mut cursor = formats;
    let mut software = sys::AVPixelFormat::AV_PIX_FMT_NONE;
    while !cursor.is_null() && unsafe { *cursor } != sys::AVPixelFormat::AV_PIX_FMT_NONE {
        let format = unsafe { *cursor };
        if format == requested {
            return format;
        }
        let descriptor = unsafe { sys::av_pix_fmt_desc_get(format) };
        if software == sys::AVPixelFormat::AV_PIX_FMT_NONE
            && !descriptor.is_null()
            && unsafe { (*descriptor).flags } & sys::AV_PIX_FMT_FLAG_HWACCEL as u64 == 0
        {
            software = format;
        }
        cursor = unsafe { cursor.add(1) };
    }
    // Preserve the existing macOS fallback policy. CUDA and VA-API must never
    // silently switch to CPU frames when hardware initialization fails (including MediaCodec).
    if kind == sys::AVHWDeviceType::AV_HWDEVICE_TYPE_VIDEOTOOLBOX {
        software
    } else {
        sys::AVPixelFormat::AV_PIX_FMT_NONE
    }
}
