use super::{DEVICE_UUID_BYTES, DriverError, check, sys};
use std::{ffi::CStr, sync::OnceLock};

static SELECTED: OnceLock<usize> = OnceLock::new();

pub struct DeviceInfo {
    pub ordinal: usize,
    pub name: String,
    pub uuid: [u8; DEVICE_UUID_BYTES],
}

pub fn selected_device() -> usize {
    SELECTED.get().copied().unwrap_or(0)
}

pub fn select_device(ordinal: usize) -> Result<(), String> {
    if SELECTED.set(ordinal).is_err() && selected_device() != ordinal {
        return Err("CUDA device cannot change after startup".into());
    }
    Ok(())
}

pub fn devices() -> Result<Vec<DeviceInfo>, DriverError> {
    unsafe extern "C" {
        fn shrimply_cuda_device_count(count: *mut i32) -> sys::CUresult;
        fn shrimply_cuda_device_name(
            name: *mut std::ffi::c_char,
            length: i32,
            device: sys::CUdevice,
        ) -> sys::CUresult;
    }
    check(unsafe { super::shrimply_cuda_init(0) })?;
    let mut count = 0;
    check(unsafe { shrimply_cuda_device_count(&mut count) })?;
    (0..count)
        .map(|ordinal| {
            let mut device = 0;
            let mut name = [0; 256];
            let mut uuid = [0; DEVICE_UUID_BYTES];
            check(unsafe { super::shrimply_cuda_device_get(&mut device, ordinal) })?;
            check(unsafe {
                shrimply_cuda_device_name(name.as_mut_ptr(), name.len() as i32, device)
            })?;
            check(unsafe { super::shrimply_cuda_device_uuid(uuid.as_mut_ptr(), device) })?;
            Ok(DeviceInfo {
                ordinal: ordinal as usize,
                name: unsafe { CStr::from_ptr(name.as_ptr()) }
                    .to_string_lossy()
                    .into_owned(),
                uuid,
            })
        })
        .collect()
}
