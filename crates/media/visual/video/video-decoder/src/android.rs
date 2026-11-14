//! MediaCodec setup only. Container reading, seek/flush, frame selection and
//! cancellation continue to use the shared VideoDecoderSession.
use crate::backend::{DecoderBackend, HardwareDevice};
use ffmpeg_next::sys;
use ndk::native_window::NativeWindow;
use shrimply_visual_frame::android::AndroidFramePool;
use std::sync::Arc;

unsafe extern "C" {
    fn av_jni_set_java_vm(vm: *mut std::ffi::c_void, log_context: *mut std::ffi::c_void) -> i32;
    fn av_jni_get_java_vm(log_context: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
}

/// # Safety
/// `vm` must be the process's live JavaVM, supplied by the native Android host.
pub unsafe fn initialize_java_vm(vm: *mut std::ffi::c_void) -> Result<(), String> {
    let result = unsafe { av_jni_set_java_vm(vm, std::ptr::null_mut()) };
    if result < 0 {
        return Err(format!(
            "Register media Java VM: {}",
            ffmpeg_next::Error::from(result)
        ));
    }
    Ok(())
}

unsafe extern "C" fn release_window(context: *mut sys::AVHWDeviceContext) {
    unsafe {
        let window = (*context).user_opaque.cast::<NativeWindow>();
        if !window.is_null() {
            drop(Box::from_raw(window));
            (*context).user_opaque = std::ptr::null_mut();
        }
    }
}

impl DecoderBackend {
    /// The window survives all FFmpeg references, including retained output
    /// frames; it is released by AVHWDeviceContext's final owner.
    pub fn mediacodec(pool: Arc<AndroidFramePool>) -> Result<Self, String> {
        // FFmpeg's codec enumeration rejects software-only codecs. Without a
        // registered VM it instead chooses by MIME, which cannot ensure that.
        if unsafe { av_jni_get_java_vm(std::ptr::null_mut()) }.is_null() {
            return Err(
                "Register the Java VM before selecting a hardware MediaCodec decoder".into(),
            );
        }
        let window = pool.window()?;
        let mut reference =
            unsafe { sys::av_hwdevice_ctx_alloc(sys::AVHWDeviceType::AV_HWDEVICE_TYPE_MEDIACODEC) };
        if reference.is_null() {
            return Err("allocate MediaCodec device context".into());
        }
        unsafe {
            let context = &mut *(*reference).data.cast::<sys::AVHWDeviceContext>();
            let native = &mut *context.hwctx.cast::<sys::AVMediaCodecDeviceContext>();
            let window = Box::new(window);
            native.native_window = window.ptr().as_ptr().cast();
            context.user_opaque = Box::into_raw(window).cast();
            context.free = Some(release_window);
            let result = sys::av_hwdevice_ctx_init(reference);
            if result < 0 {
                sys::av_buffer_unref(&mut reference);
                return Err(format!(
                    "initialize MediaCodec device: {}",
                    ffmpeg_next::Error::from(result)
                ));
            }
            let retained = HardwareDevice::retain(reference);
            sys::av_buffer_unref(&mut reference);
            retained.map(|device| Self::Android { device, pool })
        }
    }
}
