use std::{
    ffi::{CStr, CString, c_char, c_void},
    marker::PhantomData,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

pub mod sys;

use sys::{CUcontext, CUdevice, CUdeviceptr, CUevent, CUfunction, CUmodule, CUresult, CUstream};

const DEVICE_UUID_BYTES: usize = 16;

unsafe extern "C" {
    fn shrimply_cuda_init(flags: u32) -> CUresult;
    fn shrimply_cuda_device_get(device: *mut CUdevice, ordinal: i32) -> CUresult;
    fn shrimply_cuda_device_uuid(uuid: *mut u8, device: CUdevice) -> CUresult;
    fn shrimply_cuda_primary_retain(context: *mut CUcontext, device: CUdevice) -> CUresult;
    fn shrimply_cuda_primary_release(device: CUdevice) -> CUresult;
    fn shrimply_cuda_context_get_current(context: *mut CUcontext) -> CUresult;
    fn shrimply_cuda_context_set_current(context: CUcontext) -> CUresult;
    fn shrimply_cuda_context_synchronize() -> CUresult;
    fn shrimply_cuda_stream_create(stream: *mut CUstream) -> CUresult;
    fn shrimply_cuda_stream_destroy(stream: CUstream) -> CUresult;
    fn shrimply_cuda_stream_synchronize(stream: CUstream) -> CUresult;
    fn shrimply_cuda_stream_wait_event(stream: CUstream, event: CUevent) -> CUresult;
    fn shrimply_cuda_event_create(event: *mut CUevent, flags: u32) -> CUresult;
    fn shrimply_cuda_event_destroy(event: CUevent) -> CUresult;
    fn shrimply_cuda_event_record(event: CUevent, stream: CUstream) -> CUresult;
    fn shrimply_cuda_event_synchronize(event: CUevent) -> CUresult;
    fn shrimply_cuda_event_elapsed(
        milliseconds: *mut f32,
        start: CUevent,
        end: CUevent,
    ) -> CUresult;
    fn shrimply_cuda_module_load(module: *mut CUmodule, image: *const c_void) -> CUresult;
    fn shrimply_cuda_module_unload(module: CUmodule) -> CUresult;
    fn shrimply_cuda_module_function(
        function: *mut CUfunction,
        module: CUmodule,
        name: *const c_char,
    ) -> CUresult;
    fn shrimply_cuda_launch(
        function: CUfunction,
        gx: u32,
        gy: u32,
        gz: u32,
        bx: u32,
        by: u32,
        bz: u32,
        shared: u32,
        stream: CUstream,
        arguments: *mut *mut c_void,
    ) -> CUresult;
    fn shrimply_cuda_mem_alloc(pointer: *mut CUdeviceptr, bytes: usize) -> CUresult;
    fn shrimply_cuda_mem_free(pointer: CUdeviceptr) -> CUresult;
    fn shrimply_cuda_memcpy_htod_async(
        destination: CUdeviceptr,
        source: *const c_void,
        bytes: usize,
        stream: CUstream,
    ) -> CUresult;
    fn shrimply_cuda_memcpy_htod(
        destination: CUdeviceptr,
        source: *const c_void,
        bytes: usize,
    ) -> CUresult;
    fn shrimply_cuda_memcpy_dtoh_async(
        destination: *mut c_void,
        source: CUdeviceptr,
        bytes: usize,
        stream: CUstream,
    ) -> CUresult;
    fn shrimply_cuda_memcpy_dtod_async(
        destination: CUdeviceptr,
        source: CUdeviceptr,
        bytes: usize,
        stream: CUstream,
    ) -> CUresult;
    fn shrimply_cuda_memset_async(
        destination: CUdeviceptr,
        value: u8,
        bytes: usize,
        stream: CUstream,
    ) -> CUresult;
    fn shrimply_cuda_error_name(result: CUresult, name: *mut *const c_char) -> CUresult;
    fn shrimply_cuda_error_string(result: CUresult, description: *mut *const c_char) -> CUresult;
}

fn check(result: CUresult) -> Result<(), DriverError> {
    if result == sys::CUDA_SUCCESS {
        Ok(())
    } else {
        Err(DriverError(result))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct DriverError(pub CUresult);

impl DriverError {
    pub fn error_name(&self) -> Result<&CStr, Self> {
        let mut value = std::ptr::null();
        check(unsafe { shrimply_cuda_error_name(self.0, &mut value) })?;
        Ok(unsafe { CStr::from_ptr(value) })
    }
    pub fn error_string(&self) -> Result<&CStr, Self> {
        let mut value = std::ptr::null();
        check(unsafe { shrimply_cuda_error_string(self.0, &mut value) })?;
        Ok(unsafe { CStr::from_ptr(value) })
    }
}
impl std::fmt::Debug for DriverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("DriverError")
            .field(&self.0)
            .field(&self.error_string().ok())
            .finish()
    }
}
impl std::fmt::Display for DriverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, f)
    }
}
impl std::error::Error for DriverError {}

#[derive(Debug)]
pub struct CudaContext {
    device: CUdevice,
    context: CUcontext,
    ordinal: usize,
    stream_count: AtomicUsize,
}
unsafe impl Send for CudaContext {}
unsafe impl Sync for CudaContext {}
impl CudaContext {
    pub fn new(ordinal: usize) -> Result<Arc<Self>, DriverError> {
        check(unsafe { shrimply_cuda_init(0) })?;
        let mut device = 0;
        check(unsafe { shrimply_cuda_device_get(&mut device, ordinal as i32) })?;
        let mut context = std::ptr::null_mut();
        check(unsafe { shrimply_cuda_primary_retain(&mut context, device) })?;
        let result = Arc::new(Self {
            device,
            context,
            ordinal,
            stream_count: AtomicUsize::new(0),
        });
        result.bind_to_thread()?;
        Ok(result)
    }
    pub fn ordinal(&self) -> usize {
        self.ordinal
    }
    pub fn cu_device(&self) -> CUdevice {
        self.device
    }
    pub fn device_uuid(&self) -> Result<[u8; DEVICE_UUID_BYTES], DriverError> {
        let mut uuid = [0; DEVICE_UUID_BYTES];
        check(unsafe { shrimply_cuda_device_uuid(uuid.as_mut_ptr(), self.device) })?;
        Ok(uuid)
    }
    pub fn cu_ctx(&self) -> CUcontext {
        self.context
    }
    pub fn bind_to_thread(&self) -> Result<(), DriverError> {
        let mut current = std::ptr::null_mut();
        check(unsafe { shrimply_cuda_context_get_current(&mut current) })?;
        if current != self.context {
            check(unsafe { shrimply_cuda_context_set_current(self.context) })?;
        }
        Ok(())
    }
    pub fn synchronize(&self) -> Result<(), DriverError> {
        self.bind_to_thread()?;
        check(unsafe { shrimply_cuda_context_synchronize() })
    }
    pub fn default_stream(self: &Arc<Self>) -> Arc<CudaStream> {
        Arc::new(CudaStream {
            stream: std::ptr::null_mut(),
            context: self.clone(),
        })
    }
    pub fn new_stream(self: &Arc<Self>) -> Result<Arc<CudaStream>, DriverError> {
        self.bind_to_thread()?;
        if self.stream_count.fetch_add(1, Ordering::Relaxed) == 0
            && let Err(error) = self.synchronize()
        {
            self.stream_count.fetch_sub(1, Ordering::Relaxed);
            return Err(error);
        }
        let mut stream = std::ptr::null_mut();
        if let Err(error) = check(unsafe { shrimply_cuda_stream_create(&mut stream) }) {
            self.stream_count.fetch_sub(1, Ordering::Relaxed);
            return Err(error);
        }
        Ok(Arc::new(CudaStream {
            stream,
            context: self.clone(),
        }))
    }
    pub fn new_event(self: &Arc<Self>, flags: Option<u32>) -> Result<CudaEvent, DriverError> {
        self.bind_to_thread()?;
        let mut event = std::ptr::null_mut();
        check(unsafe {
            shrimply_cuda_event_create(&mut event, flags.unwrap_or(sys::CU_EVENT_DISABLE_TIMING))
        })?;
        Ok(CudaEvent {
            event,
            context: self.clone(),
        })
    }
    pub fn load_module_from_image(
        self: &Arc<Self>,
        image: &[u8],
    ) -> Result<Arc<CudaModule>, DriverError> {
        self.bind_to_thread()?;
        let mut bytes = image.to_vec();
        if bytes.last() != Some(&0) {
            bytes.push(0);
        }
        let mut module = std::ptr::null_mut();
        check(unsafe { shrimply_cuda_module_load(&mut module, bytes.as_ptr().cast()) })?;
        Ok(Arc::new(CudaModule {
            module,
            context: self.clone(),
        }))
    }
}
impl Drop for CudaContext {
    fn drop(&mut self) {
        let _ = self.bind_to_thread();
        let _ = check(unsafe { shrimply_cuda_primary_release(self.device) });
    }
}

#[derive(Debug)]
pub struct CudaStream {
    stream: CUstream,
    context: Arc<CudaContext>,
}
unsafe impl Send for CudaStream {}
unsafe impl Sync for CudaStream {}
impl CudaStream {
    pub fn cu_stream(&self) -> CUstream {
        self.stream
    }
    pub fn context(&self) -> &Arc<CudaContext> {
        &self.context
    }
    pub fn synchronize(&self) -> Result<(), DriverError> {
        self.context.bind_to_thread()?;
        check(unsafe { shrimply_cuda_stream_synchronize(self.stream) })
    }
    pub fn wait(&self, event: &CudaEvent) -> Result<(), DriverError> {
        self.context.bind_to_thread()?;
        check(unsafe { shrimply_cuda_stream_wait_event(self.stream, event.event) })
    }
}
impl Drop for CudaStream {
    fn drop(&mut self) {
        if !self.stream.is_null() {
            self.context.stream_count.fetch_sub(1, Ordering::Relaxed);
            let _ = self.context.bind_to_thread();
            let _ = check(unsafe { shrimply_cuda_stream_destroy(self.stream) });
        }
    }
}

#[derive(Debug)]
pub struct CudaEvent {
    event: CUevent,
    context: Arc<CudaContext>,
}
unsafe impl Send for CudaEvent {}
unsafe impl Sync for CudaEvent {}
impl CudaEvent {
    pub fn cu_event(&self) -> CUevent {
        self.event
    }
    pub fn record(&self, stream: &CudaStream) -> Result<(), DriverError> {
        self.context.bind_to_thread()?;
        check(unsafe { shrimply_cuda_event_record(self.event, stream.stream) })
    }
    pub fn synchronize(&self) -> Result<(), DriverError> {
        self.context.bind_to_thread()?;
        check(unsafe { shrimply_cuda_event_synchronize(self.event) })
    }
    pub fn elapsed_ms(&self, end: &Self) -> Result<f32, DriverError> {
        self.synchronize()?;
        end.synchronize()?;
        let mut value = 0.0;
        check(unsafe { shrimply_cuda_event_elapsed(&mut value, self.event, end.event) })?;
        Ok(value)
    }
}
impl Drop for CudaEvent {
    fn drop(&mut self) {
        let _ = self.context.bind_to_thread();
        let _ = check(unsafe { shrimply_cuda_event_destroy(self.event) });
    }
}

#[derive(Debug)]
pub struct CudaModule {
    module: CUmodule,
    context: Arc<CudaContext>,
}
unsafe impl Send for CudaModule {}
unsafe impl Sync for CudaModule {}
impl CudaModule {
    pub fn context(&self) -> &Arc<CudaContext> {
        &self.context
    }
    pub fn load_function(self: &Arc<Self>, name: &str) -> Result<CudaFunction, DriverError> {
        self.context.bind_to_thread()?;
        let name = CString::new(name).expect("CUDA kernel name contains NUL");
        let mut function = std::ptr::null_mut();
        check(unsafe { shrimply_cuda_module_function(&mut function, self.module, name.as_ptr()) })?;
        Ok(CudaFunction {
            function,
            _module: self.clone(),
        })
    }
}
impl Drop for CudaModule {
    fn drop(&mut self) {
        let _ = self.context.bind_to_thread();
        let _ = check(unsafe { shrimply_cuda_module_unload(self.module) });
    }
}
pub struct CudaFunction {
    function: CUfunction,
    _module: Arc<CudaModule>,
}

#[derive(Clone, Copy, Debug)]
pub struct LaunchConfig {
    pub grid_dim: (u32, u32, u32),
    pub block_dim: (u32, u32, u32),
    pub shared_mem_bytes: u32,
}
impl LaunchConfig {
    pub fn for_num_elems(n: u32) -> Self {
        const BLOCK: u32 = 256;
        Self {
            grid_dim: (n.div_ceil(BLOCK), 1, 1),
            block_dim: (BLOCK, 1, 1),
            shared_mem_bytes: 0,
        }
    }
}

/// A type whose value can be copied to or from CUDA memory byte-for-byte.
///
/// # Safety
///
/// Every bit pattern copied from device memory must be valid for the type, and the type must not
/// contain host-only references or require drop glue.
pub unsafe trait DeviceCopy: Copy {}
macro_rules! device_copy {($($t:ty),*)=>{$(unsafe impl DeviceCopy for $t{})*}}
device_copy!(
    (),
    i8,
    i16,
    i32,
    i64,
    i128,
    isize,
    u8,
    u16,
    u32,
    u64,
    u128,
    usize,
    f32,
    f64,
    half::f16,
    half::bf16
);
unsafe impl<T: DeviceCopy, const N: usize> DeviceCopy for [T; N] {}
unsafe impl<T: ?Sized> DeviceCopy for *const T {}
unsafe impl<T: ?Sized> DeviceCopy for *mut T {}

pub struct DeviceBuffer<T> {
    pointer: CUdeviceptr,
    length: usize,
    context: Arc<CudaContext>,
    marker: PhantomData<T>,
}
unsafe impl<T: Send> Send for DeviceBuffer<T> {}
unsafe impl<T: Send + Sync> Sync for DeviceBuffer<T> {}
impl<T> DeviceBuffer<T> {
    pub fn cu_deviceptr(&self) -> CUdeviceptr {
        self.pointer
    }
    pub fn len(&self) -> usize {
        self.length
    }
    pub fn is_empty(&self) -> bool {
        self.length == 0
    }
    pub fn context(&self) -> &Arc<CudaContext> {
        &self.context
    }
    pub fn num_bytes(&self) -> usize {
        self.length
            .checked_mul(std::mem::size_of::<T>())
            .expect("CUDA buffer size overflow")
    }
    /// Takes ownership of an existing CUDA allocation.
    ///
    /// # Safety
    ///
    /// `pointer` must be zero or a CUDA allocation owned by `context`, large enough for `length`
    /// values of `T`, and ownership must not be transferred anywhere else.
    pub unsafe fn from_raw_parts(
        pointer: CUdeviceptr,
        length: usize,
        context: Arc<CudaContext>,
    ) -> Self {
        Self {
            pointer,
            length,
            context,
            marker: PhantomData,
        }
    }
    pub fn cast_elem<A>(self) -> DeviceBuffer<A> {
        assert_eq!(
            std::mem::size_of::<T>(),
            std::mem::size_of::<A>(),
            "cast_elem requires the same element size"
        );
        assert_eq!(
            std::mem::align_of::<T>(),
            std::mem::align_of::<A>(),
            "cast_elem requires the same element alignment"
        );
        let this = std::mem::ManuallyDrop::new(self);
        DeviceBuffer {
            pointer: this.pointer,
            length: this.length,
            context: this.context.clone(),
            marker: PhantomData,
        }
    }
    pub fn cast_chunks<A>(self) -> Result<DeviceBuffer<A>, Self> {
        let bytes = self.num_bytes();
        let element_size = std::mem::size_of::<A>();
        let alignment = std::mem::align_of::<A>();
        if element_size == 0
            || !bytes.is_multiple_of(element_size)
            || !self.pointer.is_multiple_of(alignment as u64)
        {
            return Err(self);
        }
        let this = std::mem::ManuallyDrop::new(self);
        Ok(DeviceBuffer {
            pointer: this.pointer,
            length: bytes / std::mem::size_of::<A>(),
            context: this.context.clone(),
            marker: PhantomData,
        })
    }
}
impl<T: DeviceCopy> DeviceBuffer<T> {
    pub fn zeroed(stream: &CudaStream, length: usize) -> Result<Self, DriverError> {
        let bytes = length
            .checked_mul(std::mem::size_of::<T>())
            .expect("CUDA buffer size overflow");
        let context = stream.context.clone();
        if bytes == 0 {
            return Ok(Self {
                pointer: 0,
                length,
                context,
                marker: PhantomData,
            });
        }
        let mut pointer = 0;
        check(unsafe { shrimply_cuda_mem_alloc(&mut pointer, bytes) })?;
        let buffer = Self {
            pointer,
            length,
            context,
            marker: PhantomData,
        };
        check(unsafe { shrimply_cuda_memset_async(pointer, 0, bytes, stream.stream) })?;
        Ok(buffer)
    }
    pub fn from_host(stream: &CudaStream, data: &[T]) -> Result<Self, DriverError> {
        let mut result = Self::zeroed(stream, data.len())?;
        if result.is_empty() {
            return Ok(result);
        }
        result.copy_from_host(stream, data)?;
        Ok(result)
    }
    pub fn copy_from_host(&mut self, stream: &CudaStream, data: &[T]) -> Result<(), DriverError> {
        assert_eq!(data.len(), self.length);
        if self.is_empty() {
            return Ok(());
        }
        check(unsafe {
            shrimply_cuda_memcpy_htod_async(
                self.pointer,
                data.as_ptr().cast(),
                self.num_bytes(),
                stream.stream,
            )
        })?;
        stream.synchronize()
    }
    pub fn to_host_vec(&self, stream: &CudaStream) -> Result<Vec<T>, DriverError> {
        if self.is_empty() {
            return Ok(Vec::new());
        }
        let mut output = Vec::<T>::with_capacity(self.length);
        check(unsafe {
            shrimply_cuda_memcpy_dtoh_async(
                output.as_mut_ptr().cast(),
                self.pointer,
                self.num_bytes(),
                stream.stream,
            )
        })?;
        stream.synchronize()?;
        unsafe { output.set_len(self.length) };
        Ok(output)
    }
    pub fn copy_from_device_async(
        &mut self,
        other: &Self,
        stream: &CudaStream,
    ) -> Result<(), DriverError> {
        assert_eq!(self.length, other.length);
        if self.is_empty() {
            return Ok(());
        }
        check(unsafe {
            shrimply_cuda_memcpy_dtod_async(
                self.pointer,
                other.pointer,
                self.num_bytes(),
                stream.stream,
            )
        })
    }
    pub fn zero_async(&mut self, stream: &CudaStream) -> Result<(), DriverError> {
        if self.is_empty() {
            return Ok(());
        }
        check(unsafe {
            shrimply_cuda_memset_async(self.pointer, 0, self.num_bytes(), stream.stream)
        })
    }
}
impl<T> Drop for DeviceBuffer<T> {
    fn drop(&mut self) {
        if self.pointer != 0 {
            let _ = self.context.bind_to_thread();
            let _ = self.context.synchronize();
            let _ = check(unsafe { shrimply_cuda_mem_free(self.pointer) });
        }
    }
}

pub mod memory {
    use super::*;
    /// Copies host memory into CUDA memory synchronously.
    ///
    /// # Safety
    ///
    /// `source` and `destination` must each be valid for `bytes` readable and writable bytes,
    /// respectively.
    pub unsafe fn memcpy_htod_sync<T>(
        destination: CUdeviceptr,
        source: *const T,
        bytes: usize,
    ) -> Result<(), DriverError> {
        check(unsafe { shrimply_cuda_memcpy_htod(destination, source.cast(), bytes) })
    }
    /// Enqueues a copy from CUDA memory into host memory.
    ///
    /// # Safety
    ///
    /// `source` and `destination` must each be valid for `bytes` readable and writable bytes,
    /// respectively, and `destination` must remain valid until `stream` completes the copy.
    pub unsafe fn memcpy_dtoh_async<T>(
        destination: *mut T,
        source: CUdeviceptr,
        bytes: usize,
        stream: CUstream,
    ) -> Result<(), DriverError> {
        check(unsafe { shrimply_cuda_memcpy_dtoh_async(destination.cast(), source, bytes, stream) })
    }
    /// Enqueues a copy between CUDA allocations.
    ///
    /// # Safety
    ///
    /// `source` and `destination` must each be valid for `bytes` readable and writable bytes,
    /// respectively, until `stream` completes the copy.
    pub unsafe fn memcpy_dtod_async(
        destination: CUdeviceptr,
        source: CUdeviceptr,
        bytes: usize,
        stream: CUstream,
    ) -> Result<(), DriverError> {
        check(unsafe { shrimply_cuda_memcpy_dtod_async(destination, source, bytes, stream) })
    }
}

/// Launches a CUDA kernel with already encoded argument storage.
///
/// # Safety
///
/// `arguments` must point to values with the exact ABI expected by `name`, and all argument storage
/// and referenced device memory must remain valid until the launch has been submitted.
pub unsafe fn launch_raw(
    module: &Arc<CudaModule>,
    stream: &CudaStream,
    config: LaunchConfig,
    name: &str,
    mut arguments: Vec<*mut c_void>,
) -> Result<(), DriverError> {
    let function = module.load_function(name)?;
    let (gx, gy, gz) = config.grid_dim;
    let (bx, by, bz) = config.block_dim;
    check(unsafe {
        shrimply_cuda_launch(
            function.function,
            gx,
            gy,
            gz,
            bx,
            by,
            bz,
            config.shared_mem_bytes,
            stream.stream,
            arguments.as_mut_ptr(),
        )
    })
}

#[macro_export]
macro_rules! __cuda_arguments {
    ($arguments:ident, $launch:expr;) => { $launch };
    ($arguments:ident, $launch:expr; slice($value:expr) $(, $($rest:tt)*)?) => {{
        let value = $value;
        let mut pointer = value.cu_deviceptr();
        let mut length = u64::try_from(value.len()).expect("CUDA slice length exceeds u64");
        $arguments.push(::std::ptr::from_mut(&mut pointer).cast::<::std::ffi::c_void>());
        $arguments.push(::std::ptr::from_mut(&mut length).cast::<::std::ffi::c_void>());
        $crate::__cuda_arguments!($arguments, $launch; $($($rest)*)?)
    }};
    ($arguments:ident, $launch:expr; slice_mut($value:expr) $(, $($rest:tt)*)?) => {{
        let value = $value;
        let mut pointer = value.cu_deviceptr();
        let mut length = u64::try_from(value.len()).expect("CUDA slice length exceeds u64");
        $arguments.push(::std::ptr::from_mut(&mut pointer).cast::<::std::ffi::c_void>());
        $arguments.push(::std::ptr::from_mut(&mut length).cast::<::std::ffi::c_void>());
        $crate::__cuda_arguments!($arguments, $launch; $($($rest)*)?)
    }};
    ($arguments:ident, $launch:expr; $value:expr $(, $($rest:tt)*)?) => {{
        let mut value = $value;
        if ::std::mem::size_of_val(&value) != 0 {
            $arguments.push(::std::ptr::from_mut(&mut value).cast::<::std::ffi::c_void>());
        }
        $crate::__cuda_arguments!($arguments, $launch; $($($rest)*)?)
    }};
}

#[macro_export]
macro_rules! cuda_launch { (kernel:$kernel:path,stream:$stream:expr,module:$module:expr,config:$config:expr,args:[$($args:tt)*] $(,)?) => {{ let mut arguments=Vec::<*mut ::std::ffi::c_void>::new();$crate::__cuda_arguments!(arguments,$crate::launch_raw($module,$stream,$config,stringify!($kernel).rsplit("::").next().unwrap(),arguments);$($args)*) }}; }
