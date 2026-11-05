use std::{
    ffi::{c_char, c_void},
    sync::{Arc, OnceLock},
};

pub type Handle = *mut c_void;

macro_rules! api {
    ($($(#[$meta:meta])* $field:ident: $signature:ty = $symbol:literal),* $(,)?) => {
        pub struct Api { pub _library: libloading::Library, $($(#[$meta])* pub $field: $signature,)* }
        impl Api {
            fn load() -> Result<Self, String> {
                let library = if let Some(path) = std::env::var_os("SHRIMPLY_OIDN_LIBRARY") {
                    unsafe { libloading::Library::new(path) }
                } else if cfg!(windows) {
                    unsafe { libloading::Library::new("OpenImageDenoise.dll") }
                } else if cfg!(target_os = "macos") {
                    unsafe { libloading::Library::new("libOpenImageDenoise.2.dylib") }
                } else {
                    unsafe { libloading::Library::new("libOpenImageDenoise.so.2") }
                }.map_err(|error| format!("Load Open Image Denoise GPU runtime: {error}"))?;
                $($(#[$meta])* let $field = unsafe { *library.get::<$signature>(concat!($symbol, "\0").as_bytes()).map_err(|error| error.to_string())? };)*
                Ok(Self { _library: library, $($(#[$meta])* $field,)* })
            }
        }
    }
}

api! {
    physical_count: unsafe extern "C" fn() -> i32 = "oidnGetNumPhysicalDevices",
    physical_bool: unsafe extern "C" fn(i32, *const c_char) -> bool = "oidnGetPhysicalDeviceBool",
    physical_int: unsafe extern "C" fn(i32, *const c_char) -> i32 = "oidnGetPhysicalDeviceInt",
    physical_data: unsafe extern "C" fn(i32, *const c_char, *mut usize) -> *const c_void = "oidnGetPhysicalDeviceData",
    new_device: unsafe extern "C" fn(i32) -> Handle = "oidnNewDeviceByID",
    cuda_supported: unsafe extern "C" fn(i32) -> bool = "oidnIsCUDADeviceSupported",
    new_cuda: unsafe extern "C" fn(*const i32, *const Handle, i32) -> Handle = "oidnNewCUDADevice",
    device_int: unsafe extern "C" fn(Handle, *const c_char) -> i32 = "oidnGetDeviceInt",
    commit_device: unsafe extern "C" fn(Handle) = "oidnCommitDevice",
    sync_device: unsafe extern "C" fn(Handle) = "oidnSyncDevice",
    release_device: unsafe extern "C" fn(Handle) = "oidnReleaseDevice",
    error: unsafe extern "C" fn(Handle, *mut *const c_char) -> i32 = "oidnGetDeviceError",
    #[cfg(target_os = "linux")]
    new_fd: unsafe extern "C" fn(Handle, u32, i32, usize) -> Handle = "oidnNewSharedBufferFromFD",
    release_buffer: unsafe extern "C" fn(Handle) = "oidnReleaseBuffer",
    new_filter: unsafe extern "C" fn(Handle, *const c_char) -> Handle = "oidnNewFilter",
    release_filter: unsafe extern "C" fn(Handle) = "oidnReleaseFilter",
    set_image: unsafe extern "C" fn(Handle, *const c_char, Handle, i32, usize, usize, usize, usize, usize) = "oidnSetFilterImage",
    set_pointer: unsafe extern "C" fn(Handle, *const c_char, Handle, i32, usize, usize, usize, usize, usize) = "oidnSetSharedFilterImage",
    unset_image: unsafe extern "C" fn(Handle, *const c_char) = "oidnUnsetFilterImage",
    filter_bool: unsafe extern "C" fn(Handle, *const c_char, bool) = "oidnSetFilterBool",
    filter_int: unsafe extern "C" fn(Handle, *const c_char, i32) = "oidnSetFilterInt",
    commit_filter: unsafe extern "C" fn(Handle) = "oidnCommitFilter",
    execute_filter: unsafe extern "C" fn(Handle) = "oidnExecuteFilterAsync",
}

pub fn get() -> Result<Arc<Api>, String> {
    static API: OnceLock<Result<Arc<Api>, String>> = OnceLock::new();
    API.get_or_init(|| Api::load().map(Arc::new)).clone()
}
