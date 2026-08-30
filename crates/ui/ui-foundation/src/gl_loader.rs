use std::ffi::{CString, c_void};

#[cfg(target_os = "linux")]
#[link(name = "GL")]
unsafe extern "C" {
    fn glXGetProcAddressARB(proc_name: *const u8) -> *const c_void;
}

#[cfg(target_os = "linux")]
pub fn proc_address(symbol: &str) -> *const c_void {
    let Ok(symbol) = CString::new(symbol) else {
        return std::ptr::null();
    };
    unsafe { glXGetProcAddressARB(symbol.as_ptr().cast()) }
}

// macOS has no GLX; resolve GL symbols through the OpenGL framework instead.
// GTK's macOS GLArea still provides a legacy (4.1) GL context.
#[cfg(not(target_os = "linux"))]
pub fn proc_address(symbol: &str) -> *const c_void {
    let Ok(symbol) = CString::new(symbol) else {
        return std::ptr::null();
    };
    static FRAMEWORK: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    let handle = *FRAMEWORK.get_or_init(|| {
        let path = CString::new("/System/Library/Frameworks/OpenGL.framework/OpenGL").unwrap();
        unsafe { libc::dlopen(path.as_ptr(), libc::RTLD_LAZY | libc::RTLD_GLOBAL) as usize }
    });
    if handle == 0 {
        return std::ptr::null();
    }
    unsafe { libc::dlsym(handle as *mut c_void, symbol.as_ptr()) }
}

pub fn context() -> glow::Context {
    unsafe { glow::Context::from_loader_function(proc_address) }
}
