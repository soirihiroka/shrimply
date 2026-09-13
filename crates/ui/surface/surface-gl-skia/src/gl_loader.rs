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
    #[cfg(windows)]
    unsafe {
        use windows::{
            Win32::{Graphics::OpenGL::wglGetProcAddress, System::LibraryLoader::*},
            core::PCSTR,
        };
        let symbol = PCSTR(symbol.as_ptr().cast());
        if let Some(address) = wglGetProcAddress(symbol) {
            let address = address as *const c_void;
            if !matches!(address as usize, 1..=3 | usize::MAX) {
                return address;
            }
        }
        LoadLibraryA(windows::core::s!("opengl32.dll"))
            .ok()
            .and_then(|module| GetProcAddress(module, symbol))
            .map_or(std::ptr::null(), |address| address as *const c_void)
    }
}

pub fn context() -> glow::Context {
    unsafe { glow::Context::from_loader_function(proc_address) }
}
