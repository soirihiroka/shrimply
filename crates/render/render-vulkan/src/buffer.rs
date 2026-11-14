use crate::device::Device;
use ash::vk;
use std::sync::Arc;
#[cfg(target_os = "linux")]
use std::sync::OnceLock;

#[derive(Clone)]
pub struct Buffer(Arc<Storage>);

struct Storage {
    #[cfg(target_os = "linux")]
    oidn: OnceLock<Result<shrimply_denoise_oidn::SharedBuffer, String>>,
    device: Arc<Device>,
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    size: u64,
    address: u64,
    host_visible: bool,
    exportable: bool,
    #[cfg(target_os = "linux")]
    allocation_size: usize,
}

impl Buffer {
    pub(crate) fn new(
        device: Arc<Device>,
        size: usize,
        upload: bool,
        exportable: bool,
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
                    | vk::BufferUsageFlags::TRANSFER_SRC
                    | vk::BufferUsageFlags::TRANSFER_DST,
            )
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        if exportable {
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
        let mut export = vk::ExportMemoryAllocateInfo::default()
            .handle_types(vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD);
        let mut info = vk::MemoryAllocateInfo::default()
            .allocation_size(requirements.size)
            .memory_type_index(index)
            .push_next(&mut address);
        if exportable {
            info = info.push_next(&mut export);
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
        Ok(Self(Arc::new(Storage {
            #[cfg(target_os = "linux")]
            oidn: OnceLock::new(),
            device,
            buffer,
            memory,
            size: size as u64,
            address,
            host_visible,
            exportable,
            #[cfg(target_os = "linux")]
            allocation_size: requirements.size as usize,
        })))
    }

    pub(crate) fn exportable(&self) -> bool {
        self.0.exportable
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn oidn(
        &self,
        device: &Arc<shrimply_denoise_oidn::Device>,
    ) -> Result<&shrimply_denoise_oidn::SharedBuffer, String> {
        self.0
            .oidn
            .get_or_init(|| {
                use std::os::fd::{FromRawFd, OwnedFd};
                if !self.0.exportable {
                    return Err("Vulkan buffer was not allocated for external GPU access".into());
                }
                let loader = ash::khr::external_memory_fd::Device::new(
                    &self.0.device.instance,
                    &self.0.device.raw,
                );
                let fd = unsafe {
                    loader.get_memory_fd(
                        &vk::MemoryGetFdInfoKHR::default()
                            .memory(self.0.memory)
                            .handle_type(vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD),
                    )
                }
                .map_err(|error| error.to_string())?;
                device.import_fd(unsafe { OwnedFd::from_raw_fd(fd) }, self.0.allocation_size)
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    pub fn address(&self) -> u64 {
        self.0.address
    }
    pub(crate) fn unused(&self) -> bool {
        // Submissions retain their resources until their GPU fence completes.
        // The pool's sole reference therefore proves there are no pending users.
        Arc::strong_count(&self.0) == 1
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
}

impl Drop for Storage {
    fn drop(&mut self) {
        #[cfg(target_os = "linux")]
        drop(self.oidn.take());
        unsafe {
            self.device.raw.destroy_buffer(self.buffer, None);
            self.device.raw.free_memory(self.memory, None);
        }
    }
}
