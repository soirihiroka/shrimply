use std::any::Any;
use std::hash::{Hash, Hasher};
use std::ops::{Deref, DerefMut};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use hashbrown::{HashMap, HashSet};
use shrimply_gpu_cuda::{CudaContext, CudaStream, DeviceBuffer, DeviceCopy, sys};

const RESERVE_DIVISOR: u64 = 16;
const RESIDENCY_HOST: u8 = 0;
const RESIDENCY_DEVICE: u8 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AllocationClass {
    Persistent,
    Cached,
    Transient,
    External,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryKind {
    Device,
    Managed,
}

#[derive(Clone, Copy)]
enum ManagedLocation {
    Host,
    Device(i32),
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Telemetry {
    pub host_budget_bytes: u64,
    pub host_reserved_bytes: u64,
    pub device_local_bytes: u64,
    pub managed_bytes: u64,
    pub bytes_prefetched_to_gpu: u64,
    pub bytes_prefetched_to_host: u64,
    pub managed_allocation_events: u64,
    pub migration_events: u64,
    pub reconstructible_resources_released: u64,
    pub manim_render_surface_releases: u64,
    pub manim_gpu_animation_releases: u64,
    pub last_resort_cleanup_events: u64,
    pub last_resort_recovered_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceKey {
    source: PathBuf,
    discriminator: Vec<u8>,
}

impl ResourceKey {
    pub fn new(source: PathBuf, discriminator: Vec<u8>) -> Self {
        Self {
            source,
            discriminator,
        }
    }
}

impl Hash for ResourceKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.source.hash(state);
        self.discriminator.hash(state);
    }
}

struct Shared {
    host_budget: AtomicU64,
    host_reserved: AtomicU64,
    device_local: AtomicU64,
    managed: AtomicU64,
    prefetched_gpu: AtomicU64,
    prefetched_host: AtomicU64,
    managed_allocations: AtomicU64,
    migrations: AtomicU64,
    resources_released: AtomicU64,
    manim_surfaces_released: AtomicU64,
    manim_animation_released: AtomicU64,
    last_resort_events: AtomicU64,
    last_resort_bytes: AtomicU64,
    access: AtomicU64,
    frame_epoch: AtomicU64,
    allocations: Mutex<Vec<Weak<AllocationRecord>>>,
    resources: Mutex<ResourceState>,
}

impl Default for Shared {
    fn default() -> Self {
        Self {
            host_budget: AtomicU64::new(default_host_budget_bytes()),
            host_reserved: AtomicU64::new(0),
            device_local: AtomicU64::new(0),
            managed: AtomicU64::new(0),
            prefetched_gpu: AtomicU64::new(0),
            prefetched_host: AtomicU64::new(0),
            managed_allocations: AtomicU64::new(0),
            migrations: AtomicU64::new(0),
            resources_released: AtomicU64::new(0),
            manim_surfaces_released: AtomicU64::new(0),
            manim_animation_released: AtomicU64::new(0),
            last_resort_events: AtomicU64::new(0),
            last_resort_bytes: AtomicU64::new(0),
            access: AtomicU64::new(0),
            frame_epoch: AtomicU64::new(0),
            allocations: Mutex::new(Vec::new()),
            resources: Mutex::new(ResourceState::default()),
        }
    }
}

pub struct GpuMemoryManager {
    shared: Arc<Shared>,
}

pub struct HostReservation {
    shared: Arc<Shared>,
    bytes: u64,
}

impl Drop for HostReservation {
    fn drop(&mut self) {
        self.shared
            .host_reserved
            .fetch_sub(self.bytes, Ordering::AcqRel);
        publish_telemetry(&self.shared);
    }
}

struct AllocationRecord {
    shared: Arc<Shared>,
    ptr: AtomicU64,
    bytes: u64,
    class: AllocationClass,
    description: String,
    device_ordinal: usize,
    context_handle: usize,
    managed: bool,
    residency: AtomicU8,
    advice: AtomicU8,
    last_access: AtomicU64,
    last_frame_epoch: AtomicU64,
    active: Mutex<bool>,
    counted: AtomicBool,
    _reservation: Option<HostReservation>,
}

impl AllocationRecord {
    fn activate(&self) {
        *self.active.lock().expect("GPU allocation mutex poisoned") = true;
    }

    fn touch(&self) {
        self.last_access
            .store(next_access(&self.shared), Ordering::Release);
        self.last_frame_epoch.store(
            self.shared.frame_epoch.load(Ordering::Acquire),
            Ordering::Release,
        );
        if self.managed {
            self.residency.store(RESIDENCY_DEVICE, Ordering::Release);
        }
    }

    fn deactivate(&self) {
        *self.active.lock().expect("GPU allocation mutex poisoned") = false;
    }
}

impl Drop for AllocationRecord {
    fn drop(&mut self) {
        if self.counted.load(Ordering::Acquire) {
            if self.managed {
                self.shared.managed.fetch_sub(self.bytes, Ordering::AcqRel);
            } else {
                self.shared
                    .device_local
                    .fetch_sub(self.bytes, Ordering::AcqRel);
            }
            publish_telemetry(&self.shared);
        }
    }
}

pub struct GpuBuffer<T> {
    buffer: Option<DeviceBuffer<T>>,
    record: Arc<AllocationRecord>,
    memory_kind: MemoryKind,
}

impl<T> GpuBuffer<T> {
    fn new(
        buffer: DeviceBuffer<T>,
        record: Arc<AllocationRecord>,
        memory_kind: MemoryKind,
    ) -> Self {
        record.activate();
        Self {
            buffer: Some(buffer),
            record,
            memory_kind,
        }
    }

    pub fn cu_deviceptr(&self) -> sys::CUdeviceptr {
        self.record.touch();
        self.buffer
            .as_ref()
            .expect("GPU buffer ownership was transferred")
            .cu_deviceptr()
    }

    pub fn memory_kind(&self) -> MemoryKind {
        self.memory_kind
    }

    pub fn allocation_class(&self) -> AllocationClass {
        self.record.class
    }

    pub fn prefetch_to_device(&self, stream: &CudaStream) -> Result<(), String> {
        if !self.record.managed {
            self.record.touch();
            return Ok(());
        }
        if self.record.advice.load(Ordering::Acquire) == RESIDENCY_DEVICE
            && self.record.residency.load(Ordering::Acquire) == RESIDENCY_DEVICE
        {
            self.record.touch();
            return Ok(());
        }
        if self.record.device_ordinal != stream.context().ordinal()
            || self.record.context_handle != stream.context().cu_ctx() as usize
        {
            return Err("prefetch managed buffer with a different CUDA context".to_string());
        }
        stream
            .context()
            .bind_to_thread()
            .map_err(|error| format!("bind CUDA context for managed prefetch: {error}"))?;
        let pointer = self.record.ptr.load(Ordering::Acquire);
        let bytes = usize::try_from(self.record.bytes)
            .map_err(|_| "managed prefetch size exceeds usize".to_string())?;
        let location = ManagedLocation::Device(
            i32::try_from(self.record.device_ordinal)
                .map_err(|_| "CUDA device ordinal exceeds i32".to_string())?,
        );
        unsafe { advise_preferred_location(pointer, bytes, location) }
            .map_err(|error| format!("advise managed buffer for GPU reuse: {error}"))?;
        unsafe { prefetch_managed_async(pointer, bytes, location, stream.cu_stream()) }
            .map_err(|error| format!("prefetch managed buffer for GPU reuse: {error}"))?;
        self.record
            .advice
            .store(RESIDENCY_DEVICE, Ordering::Release);
        self.record.touch();
        self.record
            .shared
            .prefetched_gpu
            .fetch_add(self.record.bytes, Ordering::AcqRel);
        publish_telemetry(&self.record.shared);
        Ok(())
    }

    pub fn cast_elem<A>(mut self) -> GpuBuffer<A> {
        self.record.deactivate();
        let buffer = self
            .buffer
            .take()
            .expect("GPU buffer ownership was transferred")
            .cast_elem();
        GpuBuffer::new(buffer, self.record.clone(), self.memory_kind)
    }

    pub fn cast_chunks<A>(mut self) -> Result<GpuBuffer<A>, Self> {
        self.record.deactivate();
        let buffer = self
            .buffer
            .take()
            .expect("GPU buffer ownership was transferred");
        match buffer.cast_chunks() {
            Ok(buffer) => Ok(GpuBuffer::new(
                buffer,
                self.record.clone(),
                self.memory_kind,
            )),
            Err(buffer) => {
                self.buffer = Some(buffer);
                self.record.activate();
                Err(self)
            }
        }
    }
}

impl<T> Deref for GpuBuffer<T> {
    type Target = DeviceBuffer<T>;

    fn deref(&self) -> &Self::Target {
        self.record.touch();
        self.buffer
            .as_ref()
            .expect("GPU buffer ownership was transferred")
    }
}

impl<T> DerefMut for GpuBuffer<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.record.touch();
        self.buffer
            .as_mut()
            .expect("GPU buffer ownership was transferred")
    }
}

impl<T> Drop for GpuBuffer<T> {
    fn drop(&mut self) {
        if self.buffer.is_some() {
            self.record.deactivate();
        }
    }
}

struct ResourceEntry {
    value: Box<dyn Any + Send + Sync>,
    lease: Weak<()>,
    last_access: u64,
    retained: bool,
}

struct ResourceValue<T> {
    value: T,
    _reservation: Option<HostReservation>,
}

pub struct ResidentResource<T> {
    value: Arc<ResourceValue<T>>,
    lease: Arc<()>,
}

impl<T> Clone for ResidentResource<T> {
    fn clone(&self) -> Self {
        Self {
            value: self.value.clone(),
            lease: self.lease.clone(),
        }
    }
}

impl<T> Deref for ResidentResource<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value.value
    }
}

#[derive(Default)]
struct ResourceState {
    entries: HashMap<ResourceKey, ResourceEntry>,
    loading: HashSet<ResourceKey>,
    failed: HashMap<ResourceKey, String>,
}

impl GpuMemoryManager {
    fn new() -> Self {
        Self {
            shared: Arc::new(Shared::default()),
        }
    }

    pub fn configure(&self, host_budget_bytes: u64) {
        self.shared
            .host_budget
            .store(host_budget_bytes, Ordering::Release);
        publish_telemetry(&self.shared);
    }

    pub fn begin_frame(&self) {
        self.shared.frame_epoch.fetch_add(1, Ordering::AcqRel);
    }

    pub fn reserve_host(&self, bytes: u64) -> Result<HostReservation, String> {
        if bytes == 0 {
            return Ok(HostReservation {
                shared: self.shared.clone(),
                bytes: 0,
            });
        }
        loop {
            if let Some(reservation) = try_reserve(&self.shared, bytes) {
                return Ok(reservation);
            }
            if !self.release_one_resource() {
                let used = self.shared.host_reserved.load(Ordering::Acquire);
                let budget = self.shared.host_budget.load(Ordering::Acquire);
                return Err(format!(
                    "GPU host memory budget exhausted: requested {bytes} bytes, {used}/{budget} bytes reserved"
                ));
            }
        }
    }

    pub fn allocate_buffer<T: DeviceCopy>(
        &self,
        stream: &CudaStream,
        length: usize,
        class: AllocationClass,
        description: impl Into<String>,
    ) -> Result<GpuBuffer<T>, String> {
        let description = description.into();
        stream
            .context()
            .bind_to_thread()
            .map_err(|error| format!("bind CUDA context to allocate {description}: {error}"))?;
        let bytes = length
            .checked_mul(std::mem::size_of::<T>())
            .and_then(|bytes| u64::try_from(bytes).ok())
            .ok_or_else(|| format!("{description}: allocation byte size overflow"))?;
        if bytes == 0 {
            let buffer = DeviceBuffer::zeroed(stream, length)
                .map_err(|error| format!("{description}: {error}"))?;
            let record = self.record(stream, class, &description, 0, false, None);
            *record.active.lock().expect("GPU allocation mutex poisoned") = true;
            return Ok(GpuBuffer::new(buffer, record, MemoryKind::Device));
        }
        let (free, total) = memory_info()?;
        let reserve = total / RESERVE_DIVISOR;
        let pressured = free < reserve.saturating_add(bytes);
        let host_enabled = self.shared.host_budget.load(Ordering::Acquire) != 0;
        if pressured {
            tracing::trace!(
                allocation_description = description,
                allocation_class = ?class,
                requested_bytes = bytes,
                free_vram_bytes = free,
                total_vram_bytes = total,
                reserve_bytes = reserve,
                managed_bytes = self.shared.managed.load(Ordering::Acquire),
                host_reserved_bytes = self.shared.host_reserved.load(Ordering::Acquire),
                host_budget_bytes = self.shared.host_budget.load(Ordering::Acquire),
                migrated_bytes = 0_u64,
                relief_level = "managed allocation routing",
                "CUDA allocation reached the VRAM reserve"
            );
        }

        if matches!(class, AllocationClass::Persistent | AllocationClass::Cached) && host_enabled {
            match self.allocate_managed(stream, length, class, &description, bytes, pressured) {
                Ok(buffer) => return Ok(buffer),
                Err(managed_error) => {
                    return self
                        .allocate_device(stream, length, class, &description, bytes)
                        .map_err(|initial_error| {
                            allocation_error(AllocationFailure {
                                description: &description,
                                bytes,
                                free,
                                total,
                                reserve,
                                initial: &initial_error,
                                managed: &managed_error,
                                shared: &self.shared,
                            })
                        });
                }
            }
        }

        if class == AllocationClass::Transient && pressured && host_enabled {
            match self.allocate_managed(stream, length, class, &description, bytes, true) {
                Ok(buffer) => return Ok(buffer),
                Err(managed_error) => {
                    return self
                        .allocate_device(stream, length, class, &description, bytes)
                        .map_err(|initial_error| {
                            allocation_error(AllocationFailure {
                                description: &description,
                                bytes,
                                free,
                                total,
                                reserve,
                                initial: &initial_error,
                                managed: &managed_error,
                                shared: &self.shared,
                            })
                        });
                }
            }
        }

        match self.allocate_device(stream, length, class, &description, bytes) {
            Ok(buffer) => Ok(buffer),
            Err(initial_error) if class != AllocationClass::External && host_enabled => self
                .allocate_managed(stream, length, class, &description, bytes, true)
                .map_err(|managed_error| {
                    allocation_error(AllocationFailure {
                        description: &description,
                        bytes,
                        free,
                        total,
                        reserve,
                        initial: &initial_error,
                        managed: &managed_error,
                        shared: &self.shared,
                    })
                }),
            Err(initial_error) if class != AllocationClass::External => {
                Err(allocation_error(AllocationFailure {
                    description: &description,
                    bytes,
                    free,
                    total,
                    reserve,
                    initial: &initial_error,
                    managed: "managed host spilling disabled by zero host budget",
                    shared: &self.shared,
                }))
            }
            Err(error) => Err(error),
        }
    }

    fn allocate_device<T: DeviceCopy>(
        &self,
        stream: &CudaStream,
        length: usize,
        class: AllocationClass,
        description: &str,
        bytes: u64,
    ) -> Result<GpuBuffer<T>, String> {
        let record = self.record(stream, class, description, bytes, false, None);
        let buffer = DeviceBuffer::zeroed(stream, length)
            .map_err(|error| format!("allocate {description} in device memory: {error}"))?;
        record.ptr.store(buffer.cu_deviceptr(), Ordering::Release);
        record.counted.store(true, Ordering::Release);
        *record.active.lock().expect("GPU allocation mutex poisoned") = true;
        self.shared.device_local.fetch_add(bytes, Ordering::AcqRel);
        self.register(&record);
        publish_telemetry(&self.shared);
        Ok(GpuBuffer::new(buffer, record, MemoryKind::Device))
    }

    fn allocate_managed<T: DeviceCopy>(
        &self,
        stream: &CudaStream,
        length: usize,
        class: AllocationClass,
        description: &str,
        bytes: u64,
        prefer_host: bool,
    ) -> Result<GpuBuffer<T>, String> {
        let reservation = self.reserve_host(bytes)?;
        let record = self.record(stream, class, description, bytes, true, Some(reservation));
        let mut ptr = 0;
        let result = unsafe {
            sys::cuMemAllocManaged(
                &mut ptr,
                usize::try_from(bytes)
                    .map_err(|_| format!("{description}: managed allocation exceeds usize"))?,
                sys::CUmemAttach_flags_enum_CU_MEM_ATTACH_GLOBAL,
            )
        };
        if result != sys::cudaError_enum_CUDA_SUCCESS {
            return Err(format!(
                "allocate {description} as managed memory: {result:?}"
            ));
        }
        let buffer = unsafe { DeviceBuffer::from_raw_parts(ptr, length, stream.context().clone()) };
        unsafe { std::ptr::write_bytes(ptr as *mut u8, 0, bytes as usize) };
        let ptr = buffer.cu_deviceptr();
        let location = if prefer_host {
            ManagedLocation::Host
        } else {
            ManagedLocation::Device(
                i32::try_from(stream.context().ordinal())
                    .map_err(|_| "CUDA device ordinal exceeds i32".to_string())?,
            )
        };
        unsafe { advise_preferred_location(ptr, bytes as usize, location) }
            .map_err(|error| format!("advise {description} managed location: {error}"))?;
        if !prefer_host {
            unsafe { prefetch_managed_async(ptr, bytes as usize, location, stream.cu_stream()) }
                .map_err(|error| format!("prefetch {description} to GPU: {error}"))?;
            self.shared
                .prefetched_gpu
                .fetch_add(bytes, Ordering::AcqRel);
        }
        record.ptr.store(ptr, Ordering::Release);
        record.residency.store(
            if prefer_host {
                RESIDENCY_HOST
            } else {
                RESIDENCY_DEVICE
            },
            Ordering::Release,
        );
        record.advice.store(
            if prefer_host {
                RESIDENCY_HOST
            } else {
                RESIDENCY_DEVICE
            },
            Ordering::Release,
        );
        record.counted.store(true, Ordering::Release);
        *record.active.lock().expect("GPU allocation mutex poisoned") = true;
        self.shared.managed.fetch_add(bytes, Ordering::AcqRel);
        self.shared
            .managed_allocations
            .fetch_add(1, Ordering::AcqRel);
        self.register(&record);
        publish_telemetry(&self.shared);
        Ok(GpuBuffer::new(buffer, record, MemoryKind::Managed))
    }

    fn record(
        &self,
        stream: &CudaStream,
        class: AllocationClass,
        description: &str,
        bytes: u64,
        managed: bool,
        reservation: Option<HostReservation>,
    ) -> Arc<AllocationRecord> {
        Arc::new(AllocationRecord {
            shared: self.shared.clone(),
            ptr: AtomicU64::new(0),
            bytes,
            class,
            description: description.to_string(),
            device_ordinal: stream.context().ordinal(),
            context_handle: stream.context().cu_ctx() as usize,
            managed,
            residency: AtomicU8::new(RESIDENCY_HOST),
            advice: AtomicU8::new(RESIDENCY_HOST),
            last_access: AtomicU64::new(next_access(&self.shared)),
            last_frame_epoch: AtomicU64::new(self.shared.frame_epoch.load(Ordering::Acquire)),
            active: Mutex::new(false),
            counted: AtomicBool::new(false),
            _reservation: reservation,
        })
    }

    fn register(&self, record: &Arc<AllocationRecord>) {
        let mut allocations = self
            .shared
            .allocations
            .lock()
            .expect("GPU allocation registry mutex poisoned");
        allocations.retain(|allocation| allocation.strong_count() != 0);
        allocations.push(Arc::downgrade(record));
    }

    pub fn relieve_vram_pressure(
        &self,
        context: &Arc<CudaContext>,
        stream: &CudaStream,
        required_bytes: u64,
        protect_current_frame: bool,
        generation_check: Option<(&AtomicU64, u64)>,
    ) -> Result<u64, String> {
        macro_rules! abort_if_superseded {
            ($action:expr) => {
                if generation_check
                    .is_some_and(|(latest, expected)| latest.load(Ordering::Acquire) != expected)
                {
                    $action
                }
            };
        }

        abort_if_superseded!(return Ok(0));
        let (initial_free, total) = memory_info()?;
        let target = total / RESERVE_DIVISOR;
        let migration_target = target
            .saturating_add(required_bytes)
            .saturating_sub(initial_free);
        if migration_target == 0 {
            return Ok(0);
        }
        if let Err(error) = stream.synchronize() {
            if error.0 != sys::cudaError_enum_CUDA_ERROR_OUT_OF_MEMORY {
                return Err(format!("synchronize before managed migration: {error}"));
            }
            tracing::warn!(?error, "CUDA reported OOM before managed migration");
        }
        abort_if_superseded!(return Ok(0));
        let current_frame_epoch = self.shared.frame_epoch.load(Ordering::Acquire);
        let mut candidates: Vec<_> = self
            .shared
            .allocations
            .lock()
            .expect("GPU allocation registry mutex poisoned")
            .iter()
            .filter_map(Weak::upgrade)
            .filter(|record| {
                record.managed
                    && record.device_ordinal == context.ordinal()
                    && record.context_handle == context.cu_ctx() as usize
                    && record.residency.load(Ordering::Acquire) == RESIDENCY_DEVICE
                    && (!protect_current_frame
                        || record.last_frame_epoch.load(Ordering::Acquire) != current_frame_epoch)
            })
            .collect();
        candidates.sort_by_key(|record| record.last_access.load(Ordering::Acquire));
        let mut migrated = 0_u64;
        let mut pending = Vec::new();
        let mut active = Vec::new();
        for record in &candidates {
            if migrated >= migration_target {
                break;
            }
            abort_if_superseded!(break);
            let guard = record.active.lock().expect("GPU allocation mutex poisoned");
            if !*guard {
                continue;
            }
            let ptr = record.ptr.load(Ordering::Acquire);
            unsafe { advise_preferred_location(ptr, record.bytes as usize, ManagedLocation::Host) }
                .map_err(|error| {
                    format!("advise {} for host migration: {error}", record.description)
                })?;
            record.advice.store(RESIDENCY_HOST, Ordering::Release);
            unsafe {
                prefetch_managed_async(
                    ptr,
                    record.bytes as usize,
                    ManagedLocation::Host,
                    stream.cu_stream(),
                )
            }
            .map_err(|error| format!("prefetch {} to host memory: {error}", record.description))?;
            migrated = migrated
                .checked_add(record.bytes)
                .ok_or("managed migration byte count overflow")?;
            pending.push(record.clone());
            active.push(guard);
        }
        if pending.is_empty() {
            return Ok(0);
        }
        stream
            .synchronize()
            .map_err(|error| format!("finish managed migrations to host memory: {error}"))?;
        for record in &pending {
            record.residency.store(RESIDENCY_HOST, Ordering::Release);
        }
        drop(active);
        self.shared
            .prefetched_host
            .fetch_add(migrated, Ordering::AcqRel);
        self.shared
            .migrations
            .fetch_add(pending.len() as u64, Ordering::AcqRel);
        let free = memory_info()?.0;
        let recovered = free.saturating_sub(initial_free);
        let telemetry = self.telemetry();
        tracing::trace!(
            requested_bytes = required_bytes,
            free_vram_bytes = free,
            total_vram_bytes = total,
            reserve_bytes = target,
            managed_bytes = telemetry.managed_bytes,
            host_reserved_bytes = telemetry.host_reserved_bytes,
            host_budget_bytes = telemetry.host_budget_bytes,
            migrated_ranges = pending.len(),
            migrated_bytes = migrated,
            recovered_bytes = recovered,
            protect_current_frame,
            relief_level = "managed LRU migration",
            "prefetched least-recently-used managed CUDA ranges to host memory"
        );
        publish_telemetry(&self.shared);
        Ok(migrated)
    }

    pub fn telemetry(&self) -> Telemetry {
        Telemetry {
            host_budget_bytes: self.shared.host_budget.load(Ordering::Acquire),
            host_reserved_bytes: self.shared.host_reserved.load(Ordering::Acquire),
            device_local_bytes: self.shared.device_local.load(Ordering::Acquire),
            managed_bytes: self.shared.managed.load(Ordering::Acquire),
            bytes_prefetched_to_gpu: self.shared.prefetched_gpu.load(Ordering::Acquire),
            bytes_prefetched_to_host: self.shared.prefetched_host.load(Ordering::Acquire),
            managed_allocation_events: self.shared.managed_allocations.load(Ordering::Acquire),
            migration_events: self.shared.migrations.load(Ordering::Acquire),
            reconstructible_resources_released: self
                .shared
                .resources_released
                .load(Ordering::Acquire),
            manim_render_surface_releases: self
                .shared
                .manim_surfaces_released
                .load(Ordering::Acquire),
            manim_gpu_animation_releases: self
                .shared
                .manim_animation_released
                .load(Ordering::Acquire),
            last_resort_cleanup_events: self.shared.last_resort_events.load(Ordering::Acquire),
            last_resort_recovered_bytes: self.shared.last_resort_bytes.load(Ordering::Acquire),
        }
    }

    pub fn note_manim_render_surface_release(&self) {
        self.shared
            .manim_surfaces_released
            .fetch_add(1, Ordering::AcqRel);
        publish_telemetry(&self.shared);
    }

    pub fn note_manim_gpu_animation_release(&self) {
        self.shared
            .manim_animation_released
            .fetch_add(1, Ordering::AcqRel);
        publish_telemetry(&self.shared);
    }

    pub fn note_last_resort_cleanup(&self, recovered_bytes: u64) {
        self.shared
            .last_resort_events
            .fetch_add(1, Ordering::AcqRel);
        self.shared
            .last_resort_bytes
            .fetch_add(recovered_bytes, Ordering::AcqRel);
        publish_telemetry(&self.shared);
    }

    pub fn get_resource<T: Send + Sync + 'static>(
        &self,
        key: &ResourceKey,
    ) -> Result<Option<ResidentResource<T>>, String> {
        let mut state = self
            .shared
            .resources
            .lock()
            .expect("resource residency mutex poisoned");
        if let Some(error) = state.failed.get(key) {
            return Err(error.clone());
        }
        if state.entries.get(key).is_some_and(|entry| !entry.retained) {
            let entry = state
                .entries
                .remove(key)
                .expect("one-shot resource disappeared");
            return entry
                .value
                .downcast::<ResidentResource<T>>()
                .map(|resource| Some(*resource))
                .map_err(|_| "resource residency type mismatch".to_string());
        }
        let access = next_access(&self.shared);
        let Some(entry) = state.entries.get_mut(key) else {
            return Ok(None);
        };
        entry.last_access = access;
        entry
            .value
            .downcast_ref::<ResidentResource<T>>()
            .cloned()
            .map(Some)
            .ok_or_else(|| "resource residency type mismatch".to_string())
    }

    pub fn contains_resource(&self, key: &ResourceKey) -> bool {
        let state = self
            .shared
            .resources
            .lock()
            .expect("resource residency mutex poisoned");
        state.entries.contains_key(key)
            || state.loading.contains(key)
            || state.failed.contains_key(key)
    }

    pub fn begin_resource_load(&self, key: ResourceKey) -> bool {
        let mut state = self
            .shared
            .resources
            .lock()
            .expect("resource residency mutex poisoned");
        if state.entries.contains_key(&key)
            || state.loading.contains(&key)
            || state.failed.contains_key(&key)
        {
            return false;
        }
        state.loading.insert(key);
        true
    }

    pub fn finish_resource_load<T: Send + Sync + 'static>(
        &self,
        key: ResourceKey,
        bytes: u64,
        result: Result<T, String>,
    ) -> Result<(), String> {
        match result {
            Ok(value) => self.insert_resource(key, bytes, value),
            Err(error) => {
                let mut state = self
                    .shared
                    .resources
                    .lock()
                    .expect("resource residency mutex poisoned");
                state.loading.remove(&key);
                state.failed.insert(key, error);
                Ok(())
            }
        }
    }

    pub fn insert_resource<T: Send + Sync + 'static>(
        &self,
        key: ResourceKey,
        bytes: u64,
        value: T,
    ) -> Result<(), String> {
        let reservation = self.reserve_host(bytes).ok();
        let retained = reservation.is_some();
        let lease = Arc::new(());
        let value = ResidentResource {
            value: Arc::new(ResourceValue {
                value,
                _reservation: reservation,
            }),
            lease: lease.clone(),
        };
        let mut state = self
            .shared
            .resources
            .lock()
            .expect("resource residency mutex poisoned");
        state.loading.remove(&key);
        state.failed.remove(&key);
        state.entries.insert(
            key,
            ResourceEntry {
                value: Box::new(value),
                lease: Arc::downgrade(&lease),
                last_access: next_access(&self.shared),
                retained,
            },
        );
        Ok(())
    }

    pub fn clear_resources(&self) {
        *self
            .shared
            .resources
            .lock()
            .expect("resource residency mutex poisoned") = ResourceState::default();
    }

    fn release_one_resource(&self) -> bool {
        let mut state = self
            .shared
            .resources
            .lock()
            .expect("resource residency mutex poisoned");
        let Some(key) = state
            .entries
            .iter()
            .filter(|(_, entry)| entry.retained && entry.lease.strong_count() == 1)
            .min_by_key(|(_, entry)| entry.last_access)
            .map(|(key, _)| key.clone())
        else {
            return false;
        };
        state.entries.remove(&key);
        self.shared
            .resources_released
            .fetch_add(1, Ordering::AcqRel);
        publish_telemetry(&self.shared);
        true
    }
}

pub fn global() -> &'static GpuMemoryManager {
    static MANAGER: OnceLock<GpuMemoryManager> = OnceLock::new();
    MANAGER.get_or_init(GpuMemoryManager::new)
}

pub fn configure(host_budget_bytes: u64) {
    global().configure(host_budget_bytes);
}

pub fn physical_system_memory_bytes() -> u64 {
    #[cfg(unix)]
    {
        let pages = unsafe { libc::sysconf(libc::_SC_PHYS_PAGES) };
        let page_bytes = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        let pages = u64::try_from(pages)
            .expect("detect physical system RAM: sysconf(_SC_PHYS_PAGES) failed");
        let page_bytes = u64::try_from(page_bytes)
            .expect("detect physical system RAM: sysconf(_SC_PAGESIZE) failed");
        pages
            .checked_mul(page_bytes)
            .expect("detect physical system RAM: byte count overflowed")
    }
    #[cfg(windows)]
    {
        use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        let mut memory = MEMORYSTATUSEX {
            dwLength: u32::try_from(std::mem::size_of::<MEMORYSTATUSEX>())
                .expect("memory status structure size fits u32"),
            ..Default::default()
        };
        unsafe { GlobalMemoryStatusEx(&mut memory) }.expect("detect physical system RAM");
        memory.ullTotalPhys
    }
}

pub fn default_host_budget_bytes() -> u64 {
    physical_system_memory_bytes() / 2
}

fn try_reserve(shared: &Arc<Shared>, bytes: u64) -> Option<HostReservation> {
    let budget = shared.host_budget.load(Ordering::Acquire);
    let mut used = shared.host_reserved.load(Ordering::Acquire);
    loop {
        let next = used.checked_add(bytes)?;
        if next > budget {
            return None;
        }
        match shared.host_reserved.compare_exchange_weak(
            used,
            next,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => {
                publish_telemetry(shared);
                return Some(HostReservation {
                    shared: shared.clone(),
                    bytes,
                });
            }
            Err(actual) => used = actual,
        }
    }
}

fn memory_info() -> Result<(u64, u64), String> {
    let mut free = 0_usize;
    let mut total = 0_usize;
    let result = unsafe { sys::cuMemGetInfo_v2(&mut free, &mut total) };
    if result != sys::cudaError_enum_CUDA_SUCCESS {
        return Err(format!("query CUDA memory availability: {result:?}"));
    }
    Ok((free as u64, total as u64))
}

unsafe fn advise_preferred_location(
    pointer: sys::CUdeviceptr,
    bytes: usize,
    location: ManagedLocation,
) -> Result<(), String> {
    #[cfg(cuda_uses_mem_location)]
    let result = unsafe {
        sys::cuMemAdvise_v2(
            pointer,
            bytes,
            sys::CUmem_advise_enum_CU_MEM_ADVISE_SET_PREFERRED_LOCATION,
            managed_location(location),
        )
    };
    #[cfg(not(cuda_uses_mem_location))]
    let result = unsafe {
        sys::cuMemAdvise(
            pointer,
            bytes,
            sys::CUmem_advise_enum_CU_MEM_ADVISE_SET_PREFERRED_LOCATION,
            match location {
                ManagedLocation::Host => -1,
                ManagedLocation::Device(device) => device,
            },
        )
    };
    if result == sys::cudaError_enum_CUDA_SUCCESS {
        Ok(())
    } else {
        Err(format!("{result:?}"))
    }
}

unsafe fn prefetch_managed_async(
    pointer: sys::CUdeviceptr,
    bytes: usize,
    location: ManagedLocation,
    stream: sys::CUstream,
) -> Result<(), String> {
    #[cfg(cuda_uses_mem_location)]
    let result = unsafe {
        sys::cuMemPrefetchAsync_v2(pointer, bytes, managed_location(location), 0, stream)
    };
    #[cfg(not(cuda_uses_mem_location))]
    let result = unsafe {
        sys::cuMemPrefetchAsync(
            pointer,
            bytes,
            match location {
                ManagedLocation::Host => -1,
                ManagedLocation::Device(device) => device,
            },
            stream,
        )
    };
    if result == sys::cudaError_enum_CUDA_SUCCESS {
        Ok(())
    } else {
        Err(format!("{result:?}"))
    }
}

#[cfg(cuda_uses_mem_location)]
fn managed_location(location: ManagedLocation) -> sys::CUmemLocation {
    let (type_, id) = match location {
        ManagedLocation::Host => (sys::CUmemLocationType_enum_CU_MEM_LOCATION_TYPE_HOST, 0),
        ManagedLocation::Device(device) => (
            sys::CUmemLocationType_enum_CU_MEM_LOCATION_TYPE_DEVICE,
            device,
        ),
    };
    sys::CUmemLocation {
        type_,
        __bindgen_anon_1: sys::CUmemLocation_st__bindgen_ty_1 { id },
    }
}

fn next_access(shared: &Shared) -> u64 {
    shared
        .access
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
            value.checked_add(1)
        })
        .expect("GPU memory access counter overflow")
        + 1
}

struct AllocationFailure<'a> {
    description: &'a str,
    bytes: u64,
    free: u64,
    total: u64,
    reserve: u64,
    initial: &'a str,
    managed: &'a str,
    shared: &'a Shared,
}

fn allocation_error(failure: AllocationFailure<'_>) -> String {
    format!(
        "allocate {} ({} bytes) failed; free/total VRAM {}/{}, reserve {}, host budget {}/{}; device error: {}; managed error: {}",
        failure.description,
        failure.bytes,
        failure.free,
        failure.total,
        failure.reserve,
        failure.shared.host_reserved.load(Ordering::Acquire),
        failure.shared.host_budget.load(Ordering::Acquire),
        failure.initial,
        failure.managed,
    )
}

fn publish_telemetry(shared: &Shared) {
    shrimply_profiling::set_counter(
        "GPU memory / Host budget bytes",
        shared.host_budget.load(Ordering::Acquire),
    );
    shrimply_profiling::set_counter(
        "GPU memory / Host reserved bytes",
        shared.host_reserved.load(Ordering::Acquire),
    );
    shrimply_profiling::set_counter(
        "GPU memory / Device-local CUDA bytes",
        shared.device_local.load(Ordering::Acquire),
    );
    shrimply_profiling::set_counter(
        "GPU memory / Managed CUDA bytes",
        shared.managed.load(Ordering::Acquire),
    );
    shrimply_profiling::set_counter(
        "GPU memory / Bytes prefetched to GPU",
        shared.prefetched_gpu.load(Ordering::Acquire),
    );
    shrimply_profiling::set_counter(
        "GPU memory / Bytes prefetched to host",
        shared.prefetched_host.load(Ordering::Acquire),
    );
    shrimply_profiling::set_counter(
        "GPU memory / Managed allocation events",
        shared.managed_allocations.load(Ordering::Acquire),
    );
    shrimply_profiling::set_counter(
        "GPU memory / Migration events",
        shared.migrations.load(Ordering::Acquire),
    );
    shrimply_profiling::set_counter(
        "GPU memory / Reconstructible resources released",
        shared.resources_released.load(Ordering::Acquire),
    );
    shrimply_profiling::set_counter(
        "GPU memory / Manim render-surface releases",
        shared.manim_surfaces_released.load(Ordering::Acquire),
    );
    shrimply_profiling::set_counter(
        "GPU memory / Manim GPU-animation releases",
        shared.manim_animation_released.load(Ordering::Acquire),
    );
    shrimply_profiling::set_counter(
        "GPU memory / Last-resort cleanup events",
        shared.last_resort_events.load(Ordering::Acquire),
    );
    shrimply_profiling::set_counter(
        "GPU memory / Last-resort recovered bytes",
        shared.last_resort_bytes.load(Ordering::Acquire),
    );
}
