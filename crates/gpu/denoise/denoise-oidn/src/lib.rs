//! GPU-only Open Image Denoise interop. No host image allocations or CPU devices.
mod api;
use api::{Api, Handle};
use std::{
    ffi::{CStr, c_void},
    ptr,
    sync::{Arc, Mutex, OnceLock},
};

const CPU_DEVICE: i32 = 1;
const FORMAT_FLOAT3: i32 = 3;
const RGBA_STRIDE: usize = size_of::<[f32; 4]>();
#[cfg(target_os = "linux")]
const OPAQUE_FD: u32 = 1;

pub fn gpu_supported(uuid: [u8; 16]) -> bool {
    api::get().is_ok_and(|api| physical_gpu(&api, uuid).is_some())
}

#[cfg(target_os = "linux")]
pub fn vulkan_supported(uuid: [u8; 16]) -> bool {
    static SUPPORT: OnceLock<Mutex<std::collections::HashMap<[u8; 16], bool>>> = OnceLock::new();
    *SUPPORT.get_or_init(Mutex::default).lock().expect("OIDN capability cache poisoned").entry(uuid).or_insert_with(|| Device::for_uuid(uuid).is_ok_and(|device| unsafe { (device.api.device_int)(device.raw, c"externalMemoryTypes".as_ptr()) } as u32 & OPAQUE_FD != 0))
}

fn physical_gpu(api: &Api, uuid: [u8; 16]) -> Option<i32> {
    (0..unsafe { (api.physical_count)() }).find(|&id| unsafe {
        if (api.physical_int)(id, c"type".as_ptr()) == CPU_DEVICE
            || !(api.physical_bool)(id, c"uuidSupported".as_ptr())
        {
            return false;
        }
        let mut size = 0;
        let data = (api.physical_data)(id, c"uuid".as_ptr(), &mut size);
        size == uuid.len()
            && !data.is_null()
            && std::slice::from_raw_parts(data.cast::<u8>(), size) == uuid
    })
}

pub struct Device {
    api: Arc<Api>,
    raw: Handle,
}
// OIDN device APIs are thread-safe; filters remain owned by their render thread.
unsafe impl Send for Device {}
unsafe impl Sync for Device {}

impl Device {
    pub fn for_uuid(uuid: [u8; 16]) -> Result<Arc<Self>, String> {
        let api = api::get()?;
        let id = physical_gpu(&api, uuid).ok_or("Open Image Denoise does not support this GPU")?;
        let raw = unsafe { (api.new_device)(id) };
        Self::commit(api, raw)
    }

    /// # Safety
    /// The supplied CUDA stream must belong to ordinal and outlive this device.
    pub unsafe fn for_cuda(ordinal: usize, stream: Handle) -> Result<Arc<Self>, String> {
        let api = api::get()?;
        let ordinal = i32::try_from(ordinal).map_err(|error| error.to_string())?;
        if !unsafe { (api.cuda_supported)(ordinal) } {
            return Err("Open Image Denoise does not support this CUDA GPU".into());
        }
        let raw = unsafe { (api.new_cuda)(&ordinal, &stream, 1) };
        Self::commit(api, raw)
    }

    fn commit(api: Arc<Api>, raw: Handle) -> Result<Arc<Self>, String> {
        if raw.is_null() {
            return Err(error(&api, raw)
                .err()
                .unwrap_or_else(|| "Create OIDN GPU device returned null".into()));
        }
        let device = Arc::new(Self { api, raw });
        unsafe {
            (device.api.commit_device)(device.raw);
        }
        device.check()?;
        Ok(device)
    }

    pub fn check(&self) -> Result<(), String> {
        error(&self.api, self.raw)
    }

    pub fn synchronize(&self) -> Result<(), String> {
        unsafe {
            (self.api.sync_device)(self.raw);
        }
        self.check()
    }

    #[cfg(target_os = "linux")]
    pub fn import_fd(
        self: &Arc<Self>,
        fd: std::os::fd::OwnedFd,
        size: usize,
    ) -> Result<SharedBuffer, String> {
        use std::os::fd::{AsRawFd, IntoRawFd};
        if unsafe { (self.api.device_int)(self.raw, c"externalMemoryTypes".as_ptr()) } as u32
            & OPAQUE_FD
            == 0
        {
            return Err("OIDN GPU does not support zero-copy opaque FD memory import".into());
        }
        let raw = unsafe { (self.api.new_fd)(self.raw, OPAQUE_FD, fd.as_raw_fd(), size) };
        if raw.is_null() {
            return Err(self
                .check()
                .err()
                .unwrap_or_else(|| "Import OIDN GPU buffer returned null".into()));
        }
        let _ = fd.into_raw_fd(); // OIDN owns the descriptor after a successful import.
        let buffer = SharedBuffer {
            device: self.clone(),
            raw,
            size,
        };
        self.check()?;
        Ok(buffer)
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        unsafe {
            (self.api.sync_device)(self.raw);
            (self.api.release_device)(self.raw);
        }
    }
}

pub struct SharedBuffer {
    device: Arc<Device>,
    raw: Handle,
    size: usize,
}
unsafe impl Send for SharedBuffer {}
unsafe impl Sync for SharedBuffer {}
impl Drop for SharedBuffer {
    fn drop(&mut self) {
        unsafe {
            (self.device.api.release_buffer)(self.raw);
        }
    }
}

pub enum Image<'a> {
    Cuda(u64),
    Shared {
        buffer: &'a SharedBuffer,
        offset: usize,
    },
}
pub struct Images<'a> {
    pub color: Image<'a>,
    pub output: Image<'a>,
    pub albedo: Option<Image<'a>>,
    pub normal: Option<Image<'a>>,
    pub size: (u32, u32),
}

#[derive(Clone, Copy)]
pub enum Quality {
    Interactive = 4,
    Final = 6,
}

pub struct Filter {
    device: Arc<Device>,
    raw: Handle,
}
// Moving a filter between render threads is safe; execution requires exclusive access.
unsafe impl Send for Filter {}
impl Filter {
    pub fn check(&self) -> Result<(), String> {
        self.device.check()
    }
    pub fn new(device: Arc<Device>) -> Result<Self, String> {
        let raw = unsafe { (device.api.new_filter)(device.raw, c"RT".as_ptr()) };
        if raw.is_null() {
            return Err(device
                .check()
                .err()
                .unwrap_or_else(|| "Create OIDN RT filter returned null".into()));
        }
        let filter = Self { device, raw };
        unsafe {
            (filter.device.api.filter_bool)(raw, c"hdr".as_ptr(), true);
            (filter.device.api.filter_bool)(raw, c"cleanAux".as_ptr(), true);
        }
        filter.device.check()?;
        Ok(filter)
    }

    /// # Safety
    /// Native CUDA images must hold size.x * size.y float4 pixels on this GPU.
    /// All image allocations and the CUDA stream must outlive asynchronous work.
    /// Input writes must finish before filtering; consumers must wait for completion.
    pub unsafe fn execute(&mut self, images: Images<'_>, quality: Quality) -> Result<(), String> {
        let (width, height) = images.size;
        let length = (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(RGBA_STRIDE))
            .filter(|&length| length != 0)
            .ok_or("Invalid OIDN image dimensions")?;
        let api = &self.device.api;
        for (name, image) in [
            (c"color", Some(images.color)),
            (c"output", Some(images.output)),
            (c"albedo", images.albedo),
            (c"normal", images.normal),
        ] {
            match image {
                None => unsafe {
                    (api.unset_image)(self.raw, name.as_ptr());
                },
                Some(Image::Cuda(pointer)) => {
                    if pointer == 0 {
                        return Err("OIDN CUDA image address is null".into());
                    }
                    unsafe {
                        (api.set_pointer)(
                            self.raw,
                            name.as_ptr(),
                            pointer as usize as *mut c_void,
                            FORMAT_FLOAT3,
                            width as usize,
                            height as usize,
                            0,
                            RGBA_STRIDE,
                            width as usize * RGBA_STRIDE,
                        );
                    }
                }
                Some(Image::Shared { buffer, offset }) => {
                    if !Arc::ptr_eq(&buffer.device, &self.device)
                        || offset
                            .checked_add(length - size_of::<f32>())
                            .is_none_or(|end| end > buffer.size)
                    {
                        return Err("Invalid shared OIDN image range or device".into());
                    }
                    unsafe {
                        (api.set_image)(
                            self.raw,
                            name.as_ptr(),
                            buffer.raw,
                            FORMAT_FLOAT3,
                            width as usize,
                            height as usize,
                            offset,
                            RGBA_STRIDE,
                            width as usize * RGBA_STRIDE,
                        );
                    }
                }
            }
        }
        unsafe {
            (api.filter_int)(self.raw, c"quality".as_ptr(), quality as i32);
            (api.commit_filter)(self.raw);
        }
        self.device.check()?;
        unsafe {
            (api.execute_filter)(self.raw);
        }
        self.device.check()
    }
}

impl Drop for Filter {
    fn drop(&mut self) {
        unsafe {
            (self.device.api.sync_device)(self.device.raw);
            (self.device.api.release_filter)(self.raw);
        }
    }
}

fn error(api: &Api, device: Handle) -> Result<(), String> {
    let mut message = ptr::null();
    let code = unsafe { (api.error)(device, &mut message) };
    if code == 0 {
        return Ok(());
    }
    let message = if message.is_null() {
        format!("error {code}")
    } else {
        unsafe { CStr::from_ptr(message) }
            .to_string_lossy()
            .into_owned()
    };
    Err(format!("Open Image Denoise GPU: {message}"))
}
