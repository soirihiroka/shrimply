use crate::device::Device;
use ash::vk;
use std::{
    os::fd::{FromRawFd, OwnedFd},
    sync::Arc,
};

#[derive(Clone)]
pub struct Buffer(Arc<Storage>);

struct Storage {
    device: Arc<Device>,
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    size: u64,
    allocation_size: u64,
    address: u64,
    export: bool,
    host_visible: bool,
    ready: Option<vk::Semaphore>,
}

impl Buffer {
    pub(crate) fn new(
        device: Arc<Device>,
        size: usize,
        upload: bool,
        export: bool,
    ) -> Result<Self, String> {
        if size == 0 {
            return Err("Vulkan buffers must not be empty".into());
        }
        let mut external = vk::ExternalMemoryBufferCreateInfo::default()
            .handle_types(vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD);
        let mut info = vk::BufferCreateInfo::default()
            .size(size as u64)
            .usage(
                vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS
                    | vk::BufferUsageFlags::STORAGE_BUFFER
                    | vk::BufferUsageFlags::UNIFORM_BUFFER
                    | vk::BufferUsageFlags::TRANSFER_SRC
                    | vk::BufferUsageFlags::TRANSFER_DST,
            )
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        if export {
            info = info.push_next(&mut external);
        }
        let buffer = unsafe { device.raw.create_buffer(&info, None) }
            .map_err(|e| format!("create Vulkan buffer: {e:?}"))?;
        let requirements = unsafe { device.raw.get_buffer_memory_requirements(buffer) };
        let flags = if upload {
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT
        } else {
            vk::MemoryPropertyFlags::DEVICE_LOCAL
        };
        let index = (0..device.memory.memory_type_count)
            .filter(|&index| requirements.memory_type_bits & (1 << index) != 0)
            .filter(|&index| {
                device.memory.memory_types[index as usize]
                    .property_flags
                    .contains(flags)
            })
            .max_by_key(|&index| {
                device.memory.memory_types[index as usize]
                    .property_flags
                    .contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
            });
        let Some(index) = index else {
            unsafe {
                device.raw.destroy_buffer(buffer, None);
            }
            return Err(format!(
                "Intel Vulkan buffer has no memory type for {flags:?}"
            ));
        };
        let mut address =
            vk::MemoryAllocateFlagsInfo::default().flags(vk::MemoryAllocateFlags::DEVICE_ADDRESS);
        let mut external = vk::ExportMemoryAllocateInfo::default()
            .handle_types(vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD);
        let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().buffer(buffer);
        let mut info = vk::MemoryAllocateInfo::default()
            .allocation_size(requirements.size)
            .memory_type_index(index)
            .push_next(&mut address);
        if export {
            info = info.push_next(&mut external).push_next(&mut dedicated);
        }
        let memory = match unsafe { device.raw.allocate_memory(&info, None) } {
            Ok(memory) => memory,
            Err(e) => {
                unsafe {
                    device.raw.destroy_buffer(buffer, None);
                }
                return Err(format!("allocate Intel Vulkan memory: {e:?}"));
            }
        };
        if let Err(e) = unsafe { device.raw.bind_buffer_memory(buffer, memory, 0) } {
            unsafe {
                device.raw.destroy_buffer(buffer, None);
                device.raw.free_memory(memory, None);
            }
            return Err(format!("bind Intel Vulkan memory: {e:?}"));
        }
        let address = unsafe {
            device
                .raw
                .get_buffer_device_address(&vk::BufferDeviceAddressInfo::default().buffer(buffer))
        };
        if address == 0 {
            unsafe {
                device.raw.destroy_buffer(buffer, None);
                device.raw.free_memory(memory, None);
            }
            return Err("Intel Vulkan buffer device addresses are unavailable".into());
        }
        let host_visible = device.memory.memory_types[index as usize]
            .property_flags
            .contains(
                vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
            );
        let ready = if export {
            let mut external = vk::ExportSemaphoreCreateInfo::default()
                .handle_types(vk::ExternalSemaphoreHandleTypeFlags::OPAQUE_FD);
            match unsafe {
                device.raw.create_semaphore(
                    &vk::SemaphoreCreateInfo::default().push_next(&mut external),
                    None,
                )
            } {
                Ok(semaphore) => Some(semaphore),
                Err(e) => {
                    unsafe {
                        device.raw.destroy_buffer(buffer, None);
                        device.raw.free_memory(memory, None);
                    }
                    return Err(format!("Create Vulkan presentation semaphore: {e:?}"));
                }
            }
        } else {
            None
        };
        Ok(Self(Arc::new(Storage {
            device,
            buffer,
            memory,
            size: size as u64,
            allocation_size: requirements.size,
            address,
            export,
            host_visible,
            ready,
        })))
    }

    pub fn address(&self) -> u64 {
        self.0.address
    }
    pub fn len(&self) -> usize {
        self.0.size as usize
    }
    pub fn is_empty(&self) -> bool {
        self.0.size == 0
    }
    pub(crate) fn raw(&self) -> vk::Buffer {
        self.0.buffer
    }
    pub(crate) fn ready(&self) -> Option<vk::Semaphore> {
        self.0.ready
    }

    pub(crate) fn write(&self, bytes: &[u8]) -> Result<(), String> {
        if bytes.len() != self.len() || !self.0.host_visible {
            return Err("Invalid Vulkan upload buffer".into());
        }
        let destination = unsafe {
            self.0
                .device
                .raw
                .map_memory(self.0.memory, 0, self.0.size, vk::MemoryMapFlags::empty())
        }
        .map_err(|e| format!("map Vulkan upload: {e:?}"))?;
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), destination.cast(), bytes.len());
            self.0.device.raw.unmap_memory(self.0.memory);
        }
        Ok(())
    }

    /// Explicit capture only. Presentation imports GPU memory into OpenGL instead.
    pub fn copy_bytes(&self) -> Result<Vec<u8>, String> {
        if !self.0.host_visible {
            return Err("Vulkan capture requires a readback buffer".into());
        }
        let pointer = unsafe {
            self.0
                .device
                .raw
                .map_memory(self.0.memory, 0, self.0.size, vk::MemoryMapFlags::empty())
        }
        .map_err(|e| format!("map Vulkan capture: {e:?}"))?;
        let bytes = unsafe { std::slice::from_raw_parts(pointer.cast(), self.len()).to_vec() };
        unsafe {
            self.0.device.raw.unmap_memory(self.0.memory);
        }
        Ok(bytes)
    }

    pub fn export(&self) -> Result<(OwnedFd, u64, OwnedFd), String> {
        if !self.0.export {
            return Err("Vulkan buffer was not allocated for external presentation".into());
        }
        let extension =
            ash::khr::external_memory_fd::Device::new(&self.0.device.instance, &self.0.device.raw);
        let fd = unsafe {
            extension.get_memory_fd(
                &vk::MemoryGetFdInfoKHR::default()
                    .memory(self.0.memory)
                    .handle_type(vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD),
            )
        }
        .map_err(|e| format!("export Vulkan frame to GTK: {e:?}"))?;
        let memory = unsafe { OwnedFd::from_raw_fd(fd) };
        let extension = ash::khr::external_semaphore_fd::Device::new(
            &self.0.device.instance,
            &self.0.device.raw,
        );
        let semaphore = self
            .0
            .ready
            .ok_or("Vulkan presentation semaphore is missing")?;
        let fd = unsafe {
            extension.get_semaphore_fd(
                &vk::SemaphoreGetFdInfoKHR::default()
                    .semaphore(semaphore)
                    .handle_type(vk::ExternalSemaphoreHandleTypeFlags::OPAQUE_FD),
            )
        }
        .map_err(|e| format!("Export Vulkan presentation semaphore: {e:?}"))?;
        Ok((memory, self.0.allocation_size, unsafe {
            OwnedFd::from_raw_fd(fd)
        }))
    }
}

impl Drop for Storage {
    fn drop(&mut self) {
        unsafe {
            self.device.raw.destroy_buffer(self.buffer, None);
            self.device.raw.free_memory(self.memory, None);
            if let Some(semaphore) = self.ready {
                self.device.raw.destroy_semaphore(semaphore, None);
            }
        }
    }
}
