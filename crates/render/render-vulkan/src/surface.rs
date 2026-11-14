use crate::device::Device;
use ash::{vk, vk::Handle};
use skia_safe::{
    ColorType,
    gpu::{self, vk as skvk},
};
use std::sync::Arc;

const API_VERSION: u32 = vk::API_VERSION_1_2;
const GPU_TIMEOUT_NS: u64 = 5_000_000_000;
const SKIA_CACHE_BYTES: usize = 64 * 1024 * 1024;

/// Constructed, used, and dropped exclusively on the native rendering thread.
pub struct Renderer {
    shared: Arc<Device>,
    surface_api: ash::khr::surface::Instance,
    surface: vk::SurfaceKHR,
    physical: vk::PhysicalDevice,
    device: ash::Device,
    queue: vk::Queue,
    family: u32,
    swapchain_api: ash::khr::swapchain::Device,
    swapchain: vk::SwapchainKHR,
    images: Vec<vk::Image>,
    presented: Vec<bool>,
    rendered: Vec<vk::Semaphore>,
    acquired: vk::Semaphore,
    fence: vk::Fence,
    context: Option<gpu::DirectContext>,
    extent: vk::Extent2D,
    format: vk::Format,
    #[cfg(feature = "renderer")]
    sampled: Option<(skia_safe::Image, crate::Texture)>,
}

impl Renderer {
    /// The caller retains the Android window until after this renderer is dropped.
    #[cfg(target_os = "android")]
    pub fn new(window: &ndk::native_window::NativeWindow, width: u32, height: u32) -> Self {
        unsafe {
            let shared = Device::get().expect("initialize the shared GPU renderer");
            let entry = &shared.entry;
            let instance = &shared.instance;
            let android = ash::khr::android_surface::Instance::new(entry, instance);
            let surface = android
                .create_android_surface(
                    &vk::AndroidSurfaceCreateInfoKHR::default()
                        .window(window.ptr().as_ptr().cast()),
                    None,
                )
                .expect("create Android Vulkan surface");
            Self::from_surface(shared, surface, width, height)
        }
    }

    /// Use an existing native surface with the shared Vulkan device.
    ///
    /// # Safety
    /// The surface must belong to this device's instance. This renderer takes
    /// ownership of it; retain its native window until the renderer is dropped.
    pub unsafe fn from_surface(
        shared: Arc<Device>,
        surface: vk::SurfaceKHR,
        width: u32,
        height: u32,
    ) -> Self {
        unsafe {
            let entry = &shared.entry;
            let instance = shared.instance.clone();
            let device = shared.raw.clone();
            let (physical, family, queue) = (shared.physical, shared.family, shared.queue);
            let surface_api = ash::khr::surface::Instance::new(entry, &instance);
            assert!(
                surface_api
                    .get_physical_device_surface_support(physical, family, surface)
                    .expect("query presentation support"),
                "the shared Vulkan queue cannot present to this window"
            );
            let get_proc = |request| {
                let proc = match request {
                    skvk::GetProcOf::Instance(handle, name) => {
                        entry.get_instance_proc_addr(vk::Instance::from_raw(handle as u64), name)
                    }
                    skvk::GetProcOf::Device(handle, name) => {
                        instance.get_device_proc_addr(vk::Device::from_raw(handle as u64), name)
                    }
                };
                proc.map_or(std::ptr::null(), |proc| proc as *const std::ffi::c_void)
            };
            let backend = skvk::BackendContext::new_builder(
                instance.handle().as_raw() as _,
                physical.as_raw() as _,
                device.handle().as_raw() as _,
                (queue.as_raw() as _, family as usize),
                &get_proc,
                Some(API_VERSION.into()),
            )
            .with_extensions(&["VK_KHR_surface"], &["VK_KHR_swapchain"])
            .build();
            let mut context = gpu::direct_contexts::make_vulkan(&backend, None)
                .expect("create Skia Vulkan context");
            drop(backend);
            context.set_resource_cache_limit(SKIA_CACHE_BYTES);
            let swapchain_api = ash::khr::swapchain::Device::new(&instance, &device);
            let acquired = device
                .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
                .expect("create acquire semaphore");
            let fence = device
                .create_fence(
                    &vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED),
                    None,
                )
                .expect("create frame fence");
            let mut renderer = Self {
                shared,
                surface_api,
                surface,
                physical,
                device,
                queue,
                family,
                swapchain_api,
                swapchain: vk::SwapchainKHR::null(),
                images: Vec::new(),
                presented: Vec::new(),
                rendered: Vec::new(),
                acquired,
                fence,
                context: Some(context),
                extent: vk::Extent2D::default(),
                format: vk::Format::UNDEFINED,
                #[cfg(feature = "renderer")]
                sampled: None,
            };
            renderer.resize(width, height);
            renderer
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        let _queue = self.shared.queue_lock();
        unsafe {
            self.device
                .device_wait_idle()
                .expect("wait before swapchain replacement");
            #[cfg(feature = "renderer")]
            drop(self.sampled.take());
            self.context
                .as_mut()
                .expect("Skia context")
                .free_gpu_resources();
            let caps = self
                .surface_api
                .get_physical_device_surface_capabilities(self.physical, self.surface)
                .expect("surface capabilities");
            let extent = if caps.current_extent.width != u32::MAX {
                caps.current_extent
            } else {
                vk::Extent2D {
                    width: width.clamp(caps.min_image_extent.width, caps.max_image_extent.width),
                    height: height
                        .clamp(caps.min_image_extent.height, caps.max_image_extent.height),
                }
            };
            assert!(
                extent.width > 0 && extent.height > 0,
                "surface must have a positive extent"
            );
            let formats = self
                .surface_api
                .get_physical_device_surface_formats(self.physical, self.surface)
                .expect("surface formats");
            let format = formats
                .iter()
                .find(|format| {
                    [vk::Format::R8G8B8A8_UNORM, vk::Format::B8G8R8A8_UNORM]
                        .contains(&format.format)
                        && format.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR
                })
                .expect("an RGBA/BGRA UNORM surface is required");
            let usage = vk::ImageUsageFlags::COLOR_ATTACHMENT
                | vk::ImageUsageFlags::TRANSFER_SRC
                | vk::ImageUsageFlags::TRANSFER_DST;
            assert!(
                caps.supported_usage_flags.contains(usage),
                "surface must support Skia color and transfer usage"
            );
            let count = (caps.min_image_count + 1).min(if caps.max_image_count == 0 {
                u32::MAX
            } else {
                caps.max_image_count
            });
            let alpha = [
                vk::CompositeAlphaFlagsKHR::OPAQUE,
                vk::CompositeAlphaFlagsKHR::INHERIT,
            ]
            .into_iter()
            .find(|flag| caps.supported_composite_alpha.contains(*flag))
            .expect("opaque or inherited surface alpha");
            // Let Android's compositor apply window rotation; Skia and input use
            // the same upright coordinates rather than implementing pre-rotation.
            assert!(
                caps.supported_transforms
                    .contains(vk::SurfaceTransformFlagsKHR::IDENTITY),
                "an upright Android presentation surface is required"
            );
            let info = vk::SwapchainCreateInfoKHR::default()
                .surface(self.surface)
                .min_image_count(count)
                .image_format(format.format)
                .image_color_space(format.color_space)
                .image_extent(extent)
                .image_array_layers(1)
                .image_usage(usage)
                .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
                .pre_transform(vk::SurfaceTransformFlagsKHR::IDENTITY)
                .composite_alpha(alpha)
                .present_mode(vk::PresentModeKHR::FIFO)
                .clipped(true)
                .old_swapchain(self.swapchain);
            let replacement = self
                .swapchain_api
                .create_swapchain(&info, None)
                .expect("create Android swapchain");
            for semaphore in self.rendered.drain(..) {
                self.device.destroy_semaphore(semaphore, None);
            }
            self.swapchain_api.destroy_swapchain(self.swapchain, None);
            self.swapchain = replacement;
            self.images = self
                .swapchain_api
                .get_swapchain_images(replacement)
                .expect("swapchain images");
            self.presented = vec![false; self.images.len()];
            self.rendered = self
                .images
                .iter()
                .map(|_| {
                    self.device
                        .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
                        .expect("create presentation semaphore")
                })
                .collect();
            self.extent = extent;
            self.format = format.format;
        }
    }

    /// One GPU frame in flight; FIFO presentation and a fence bound resource retention.
    /// False means the caller must recreate the swapchain and redraw the unchanged state.
    pub fn draw(
        &mut self,
        paint: impl FnOnce(&skia_safe::Canvas, u32, u32) -> Option<gpu::BackendTexture>,
    ) -> bool {
        self.draw_at(None, paint)
    }

    /// Borrow a completed compositor texture without copying its pixels. Keep
    /// the allocation alive until the following draw fence or surface teardown.
    /// A timestamp is used for encoder surfaces, in nanoseconds of media time.
    #[cfg(feature = "renderer")]
    pub fn draw_texture(
        &mut self,
        texture: Option<&crate::Texture>,
        timestamp_ns: Option<u64>,
        paint: impl FnOnce(&skia_safe::Canvas, Option<&skia_safe::Image>, u32, u32),
    ) -> bool {
        #[cfg(target_os = "android")]
        assert!(
            timestamp_ns.is_none() || self.shared.presentation_timing,
            "timestamped surfaces require VK_GOOGLE_display_timing"
        );
        #[cfg(not(target_os = "android"))]
        assert!(
            timestamp_ns.is_none(),
            "timestamped surface extension was not enabled"
        );
        assert!(
            timestamp_ns.is_none_or(|time| time > 0 && time <= i64::MAX as u64),
            "surface timestamps must be positive signed nanoseconds"
        );
        let family = self.family;
        let mut sampled = self.sampled.take();
        let valid = self.draw_at(timestamp_ns, |target, width, height| {
            // draw_at has waited for the preceding draw fence. Releasing an old
            // borrowed allocation before that point would race its GPU reads.
            match texture {
                Some(texture)
                    if sampled
                        .as_ref()
                        .is_none_or(|(_, old)| old.id() != texture.id()) =>
                {
                    let mut info = unsafe {
                        skvk::ImageInfo::new(
                            texture.id() as _,
                            skvk::Alloc::default(),
                            skvk::ImageTiling::OPTIMAL,
                            skvk::ImageLayout::GENERAL,
                            skvk::Format::R8G8B8A8_UNORM,
                            texture.levels(),
                            family,
                            None,
                            None,
                            None,
                        )
                    };
                    info.image_usage_flags = (vk::ImageUsageFlags::SAMPLED
                        | vk::ImageUsageFlags::STORAGE
                        | vk::ImageUsageFlags::COLOR_ATTACHMENT
                        | vk::ImageUsageFlags::TRANSFER_SRC
                        | vk::ImageUsageFlags::TRANSFER_DST)
                        .as_raw();
                    let backend = unsafe {
                        gpu::backend_textures::make_vk(
                            (texture.width() as i32, texture.height() as i32),
                            &info,
                            "compositor surface",
                        )
                    };
                    let mut context = target.direct_context().expect("Vulkan surface canvas");
                    let image = gpu::images::borrow_texture_from(
                        &mut context,
                        &backend,
                        gpu::SurfaceOrigin::TopLeft,
                        ColorType::RGBA8888,
                        skia_safe::AlphaType::Unpremul,
                        None,
                    )
                    .expect("borrow compositor texture");
                    sampled = Some((image, texture.clone()));
                }
                None => sampled = None,
                Some(_) => {}
            }
            paint(
                target,
                sampled.as_ref().map(|(image, _)| image),
                width,
                height,
            );
            sampled.as_ref().map(|(image, _)| {
                gpu::images::get_backend_texture_from_image(image, false)
                    .expect("borrowed Vulkan compositor texture")
                    .0
            })
        });
        self.sampled = sampled;
        valid
    }

    fn draw_at(
        &mut self,
        timestamp_ns: Option<u64>,
        paint: impl FnOnce(&skia_safe::Canvas, u32, u32) -> Option<gpu::BackendTexture>,
    ) -> bool {
        unsafe {
            self.device
                .wait_for_fences(&[self.fence], true, GPU_TIMEOUT_NS)
                .expect("timeline GPU fence timed out");
            let (index, suboptimal) = match self.swapchain_api.acquire_next_image(
                self.swapchain,
                GPU_TIMEOUT_NS,
                self.acquired,
                vk::Fence::null(),
            ) {
                Ok(result) => result,
                Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => return false,
                Err(error) => panic!("acquire timeline image: {error}"),
            };
            // Window acquisition and our preceding fence can wait for display
            // pacing. Keep the shared compositor free to submit during that wait.
            let _queue = self.shared.queue_lock();
            let index = index as usize;
            let mut info = skvk::ImageInfo::new(
                self.images[index].as_raw() as _,
                skvk::Alloc::default(),
                skvk::ImageTiling::OPTIMAL,
                if self.presented[index] {
                    skvk::ImageLayout::PRESENT_SRC_KHR
                } else {
                    skvk::ImageLayout::UNDEFINED
                },
                if self.format == vk::Format::R8G8B8A8_UNORM {
                    skvk::Format::R8G8B8A8_UNORM
                } else {
                    skvk::Format::B8G8R8A8_UNORM
                },
                1,
                self.family,
                None,
                None,
                None,
            );
            info.image_usage_flags = (vk::ImageUsageFlags::COLOR_ATTACHMENT
                | vk::ImageUsageFlags::TRANSFER_SRC
                | vk::ImageUsageFlags::TRANSFER_DST)
                .as_raw();
            let target = gpu::backend_render_targets::make_vk(
                (self.extent.width as i32, self.extent.height as i32),
                &info,
            );
            let context = self.context.as_mut().expect("Skia context");
            let mut surface = gpu::surfaces::wrap_backend_render_target(
                context,
                &target,
                gpu::SurfaceOrigin::TopLeft,
                if self.format == vk::Format::R8G8B8A8_UNORM {
                    ColorType::RGBA8888
                } else {
                    ColorType::BGRA8888
                },
                None,
                None,
            )
            .expect("wrap GPU swapchain image in Skia");
            let wait = gpu::backend_semaphores::make_vk(self.acquired.as_raw() as _);
            assert!(surface.wait(&[wait], false), "Skia acquire semaphore wait");
            let sampled = paint(surface.canvas(), self.extent.width, self.extent.height);
            let mut signals = [gpu::backend_semaphores::make_vk(
                self.rendered[index].as_raw() as _,
            )];
            let mut flush = gpu::FlushInfo::default();
            flush.set_signal_semaphores(&mut signals);
            let state = skvk::mutable_texture_states::new_vulkan(
                skvk::ImageLayout::PRESENT_SRC_KHR,
                self.family,
            );
            assert_eq!(
                context.flush_surface_with_texture_state(&mut surface, &flush, Some(&state)),
                gpu::SemaphoresSubmitted::Yes
            );
            if let Some(sampled) = sampled {
                // A later Android surface may borrow this immutable composite
                // in a new Skia context. Leave its advertised layout unchanged.
                let state = skvk::mutable_texture_states::new_vulkan(
                    skvk::ImageLayout::GENERAL,
                    self.family,
                );
                assert!(
                    context.set_backend_texture_state(&sampled, &state),
                    "restore preview texture layout"
                );
            }
            assert!(context.submit(None), "submit timeline drawing");
            drop(surface);
            self.device
                .reset_fences(&[self.fence])
                .expect("reset timeline fence");
            self.device
                .queue_submit(self.queue, &[vk::SubmitInfo::default()], self.fence)
                .expect("fence timeline work");
            self.presented[index] = true;
            let swaps = [self.swapchain];
            let indices = [index as u32];
            let waits = [self.rendered[index]];
            let mut present = vk::PresentInfoKHR::default()
                .wait_semaphores(&waits)
                .swapchains(&swaps)
                .image_indices(&indices);
            let times = [vk::PresentTimeGOOGLE {
                present_id: 0,
                desired_present_time: timestamp_ns.unwrap_or(0),
            }];
            let mut timing = vk::PresentTimesInfoGOOGLE::default().times(&times);
            if timestamp_ns.is_some() {
                present = present.push_next(&mut timing);
            }
            match self.swapchain_api.queue_present(self.queue, &present) {
                Ok(changed) => !changed && !suboptimal,
                Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => false,
                Err(error) => panic!("present timeline: {error}"),
            }
        }
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        let _queue = self.shared.queue_lock();
        unsafe {
            self.device
                .device_wait_idle()
                .expect("finish native surface access");
            #[cfg(feature = "renderer")]
            drop(self.sampled.take());
            drop(self.context.take());
            for semaphore in self.rendered.drain(..) {
                self.device.destroy_semaphore(semaphore, None);
            }
            self.device.destroy_semaphore(self.acquired, None);
            self.device.destroy_fence(self.fence, None);
            self.swapchain_api.destroy_swapchain(self.swapchain, None);
            self.surface_api.destroy_surface(self.surface, None);
        }
    }
}
