use std::ffi::{CString, c_void};

#[cfg(target_os = "linux")]
#[link(name = "GL")]
unsafe extern "C" {
    fn glXGetProcAddressARB(proc_name: *const u8) -> *const c_void;
}

#[cfg(target_os = "macos")]
#[link(name = "OpenGL", kind = "framework")]
unsafe extern "C" {}

pub fn proc_address(symbol: &str) -> *const c_void {
    let Ok(symbol) = CString::new(symbol) else {
        return std::ptr::null();
    };
    #[cfg(target_os = "linux")]
    unsafe {
        glXGetProcAddressARB(symbol.as_ptr().cast())
    }
    #[cfg(target_os = "macos")]
    unsafe {
        libc::dlsym(libc::RTLD_DEFAULT, symbol.as_ptr()).cast_const()
    }
}

pub fn context() -> glow::Context {
    unsafe { glow::Context::from_loader_function(proc_address) }
}
