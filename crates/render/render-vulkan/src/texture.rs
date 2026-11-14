use crate::device::Device;
use ash::{vk, vk::Handle};
use std::sync::Arc;
#[cfg(target_os = "linux")]
use std::{
    os::fd::{FromRawFd, OwnedFd},
    sync::Mutex,
};

#[derive(Clone)]
pub struct Texture(Arc<Storage>);

impl PartialEq for Texture {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for Texture {}

struct Storage {
    device: Arc<Device>,
    image: vk::Image,
    memory: vk::DeviceMemory,
    #[cfg(target_os = "linux")]
    exports: Mutex<Vec<(vk::Semaphore, vk::Fence)>>,
    #[cfg(target_os = "linux")]
    allocation_size: u64,
    width: u32,
    height: u32,
    levels: u32,
}

impl Texture {
    pub(crate) fn new(device: Arc<Device>, width: u32, height: u32) -> Result<Self, String> {
        if width == 0 || height == 0 {
            return Err("Vulkan presentation dimensions must be positive".into());
        }
        let levels = width.max(height).ilog2() + 1;
        #[cfg(target_os = "linux")]
        let mut external = vk::ExternalMemoryImageCreateInfo::default()
            .handle_types(vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD);
        let info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(vk::Format::R8G8B8A8_UNORM)
            .extent(vk::Extent3D {
                width,
                height,
                depth: 1,
            })
            .mip_levels(levels)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(
                vk::ImageUsageFlags::SAMPLED
                    | vk::ImageUsageFlags::STORAGE
                    | vk::ImageUsageFlags::COLOR_ATTACHMENT
                    | vk::ImageUsageFlags::TRANSFER_SRC
                    | vk::ImageUsageFlags::TRANSFER_DST,
            )
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        #[cfg(target_os = "linux")]
        let info = info.push_next(&mut external);
        let image = unsafe { device.raw.create_image(&info, None) }
            .map_err(|e| format!("Create Intel presentation texture: {e:?}"))?;
        let requirements = unsafe { device.raw.get_image_memory_requirements(image) };
        let index = (0..device.memory.memory_type_count).find(|&index| {
            requirements.memory_type_bits & (1 << index) != 0
                && device.memory.memory_types[index as usize]
                    .property_flags
                    .contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
        });
        let Some(index) = index else {
            unsafe {
                device.raw.destroy_image(image, None);
            }
            return Err("Intel texture has no device-local memory type".into());
        };
        #[cfg(target_os = "linux")]
        let mut export = vk::ExportMemoryAllocateInfo::default()
            .handle_types(vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD);
        let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(image);
        let info = vk::MemoryAllocateInfo::default()
            .allocation_size(requirements.size)
            .memory_type_index(index)
            .push_next(&mut dedicated);
        #[cfg(target_os = "linux")]
        let info = info.push_next(&mut export);
        let memory = match unsafe { device.raw.allocate_memory(&info, None) } {
            Ok(memory) => memory,
            Err(e) => {
                unsafe {
                    device.raw.destroy_image(image, None);
                }
                return Err(format!("Allocate Intel presentation texture: {e:?}"));
            }
        };
        if let Err(e) = unsafe { device.raw.bind_image_memory(image, memory, 0) } {
            unsafe {
                device.raw.destroy_image(image, None);
                device.raw.free_memory(memory, None);
            }
            return Err(format!("Bind Intel presentation texture: {e:?}"));
        }
        Ok(Self(Arc::new(Storage {
            device,
            image,
            memory,
            #[cfg(target_os = "linux")]
            exports: Mutex::new(Vec::new()),
            #[cfg(target_os = "linux")]
            allocation_size: requirements.size,
            width,
            height,
            levels,
        })))
    }
    pub fn id(&self) -> u64 {
        self.0.image.as_raw()
    }
    pub fn width(&self) -> u32 {
        self.0.width
    }
    pub fn height(&self) -> u32 {
        self.0.height
    }
    pub fn levels(&self) -> u32 {
        self.0.levels
    }
    pub fn import_uuids(&self) -> ([u8; vk::UUID_SIZE], [u8; vk::UUID_SIZE]) {
        let mut identity = vk::PhysicalDeviceIDProperties::default();
        let mut properties = vk::PhysicalDeviceProperties2::default().push_next(&mut identity);
        unsafe {
            self.0
                .device
                .instance
                .get_physical_device_properties2(self.0.device.physical, &mut properties);
        }
        (identity.device_uuid, identity.driver_uuid)
    }
    pub(crate) fn image(&self) -> vk::Image {
        self.0.image
    }
    #[cfg(target_os = "linux")]
    pub fn export(&self) -> Result<(OwnedFd, u64, OwnedFd), String> {
        let memory =
            ash::khr::external_memory_fd::Device::new(&self.0.device.instance, &self.0.device.raw);
        let fd = unsafe {
            memory.get_memory_fd(
                &vk::MemoryGetFdInfoKHR::default()
                    .memory(self.0.memory)
                    .handle_type(vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD),
            )
        }
        .map_err(|e| format!("Export Intel texture memory: {e:?}"))?;
        let memory = unsafe { OwnedFd::from_raw_fd(fd) };
        // Each GL import consumes one binary semaphore signal. A fresh signal
        // permits multiple preview surfaces and recreated OpenGL contexts.
        let mut export = vk::ExportSemaphoreCreateInfo::default()
            .handle_types(vk::ExternalSemaphoreHandleTypeFlags::OPAQUE_FD);
        let semaphore = unsafe {
            self.0.device.raw.create_semaphore(
                &vk::SemaphoreCreateInfo::default().push_next(&mut export),
                None,
            )
        }
        .map_err(|e| format!("Create Intel presentation semaphore: {e:?}"))?;
        let fence = match unsafe {
            self.0
                .device
                .raw
                .create_fence(&vk::FenceCreateInfo::default(), None)
        } {
            Ok(fence) => fence,
            Err(error) => {
                unsafe {
                    self.0.device.raw.destroy_semaphore(semaphore, None);
                }
                return Err(format!("Create Intel export fence: {error:?}"));
            }
        };
        if let Err(error) = self.0.device.submit(&[], fence, &[semaphore]) {
            unsafe {
                self.0.device.raw.destroy_fence(fence, None);
                self.0.device.raw.destroy_semaphore(semaphore, None);
            }
            return Err(error);
        }
        self.0
            .exports
            .lock()
            .expect("Intel export mutex poisoned")
            .push((semaphore, fence));
        let external = ash::khr::external_semaphore_fd::Device::new(
            &self.0.device.instance,
            &self.0.device.raw,
        );
        let fd = unsafe {
            external.get_semaphore_fd(
                &vk::SemaphoreGetFdInfoKHR::default()
                    .semaphore(semaphore)
                    .handle_type(vk::ExternalSemaphoreHandleTypeFlags::OPAQUE_FD),
            )
        }
        .map_err(|e| format!("Export Intel texture semaphore: {e:?}"))?;
        Ok((memory, self.0.allocation_size, unsafe {
            OwnedFd::from_raw_fd(fd)
        }))
    }
}

impl Drop for Storage {
    fn drop(&mut self) {
        #[cfg(target_os = "linux")]
        for &(semaphore, fence) in self
            .exports
            .get_mut()
            .expect("Intel export mutex poisoned")
            .iter()
        {
            unsafe {
                if self
                    .device
                    .raw
                    .wait_for_fences(&[fence], true, u64::MAX)
                    .is_err()
                {
                    std::process::abort();
                }
                self.device.raw.destroy_fence(fence, None);
                self.device.raw.destroy_semaphore(semaphore, None);
            }
        }
        unsafe {
            self.device.raw.destroy_image(self.image, None);
            self.device.raw.free_memory(self.memory, None);
        }
    }
}
