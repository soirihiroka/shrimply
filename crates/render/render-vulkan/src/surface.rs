use crate::device::Device;
use ash::{vk, vk::Handle};
use skia_safe::{
    ColorType,
    gpu::{self, vk as skvk},
};
use std::sync::Arc;

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
    recreate: bool,
    format: vk::Format,
    #[cfg(feature = "renderer")]
    sampled: Option<(skia_safe::Image, crate::Texture)>,
}

impl Renderer {
    /// Retain the Android window until this renderer is dropped or rebound.
    #[cfg(target_os = "android")]
    pub fn new(window: &ndk::native_window::NativeWindow, width: u32, height: u32) -> Self {
        Self::try_new(window, width, height).expect("initialize Android surface renderer")
    }

    #[cfg(target_os = "android")]
    pub fn try_new(
        window: &ndk::native_window::NativeWindow,
        width: u32,
        height: u32,
    ) -> Result<Self, String> {
        let shared = Device::get()?;
        let surface = android_surface(&shared, window)?;
        unsafe { Self::try_from_surface(shared, surface, width, height) }
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
        unsafe { Self::try_from_surface(shared, surface, width, height) }
            .expect("initialize surface renderer")
    }

    unsafe fn try_from_surface(
        shared: Arc<Device>,
        surface: vk::SurfaceKHR,
        width: u32,
        height: u32,
    ) -> Result<Self, String> {
        let _measure = shrimply_profiling::measure("GPU / Surface context creation");
        unsafe {
            let entry = &shared.entry;
            let instance = shared.instance.clone();
            let device = shared.raw.clone();
            let (physical, family, queue) = (shared.physical, shared.family, shared.queue);
            let surface_api = ash::khr::surface::Instance::new(entry, &instance);
            let swapchain_api = ash::khr::swapchain::Device::new(&instance, &device);
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
                acquired: vk::Semaphore::null(),
                fence: vk::Fence::null(),
                context: None,
                extent: vk::Extent2D::default(),
                recreate: true,
                format: vk::Format::UNDEFINED,
                #[cfg(feature = "renderer")]
                sampled: None,
            };
            if !renderer
                .surface_api
                .get_physical_device_surface_support(physical, family, surface)
                .map_err(|error| format!("Query Vulkan presentation support: {error}"))?
            {
                return Err("The shared Vulkan queue cannot present to this window".into());
            }
            let mut context = renderer.shared.skia_context(true)?;
            context.set_resource_cache_limit(SKIA_CACHE_BYTES);
            renderer.context = Some(context);
            shrimply_profiling::increment("GPU / Surface contexts created");
            renderer.acquired = renderer
                .device
                .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
                .map_err(|error| format!("Create acquire semaphore: {error}"))?;
            renderer.fence = renderer
                .device
                .create_fence(
                    &vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED),
                    None,
                )
                .map_err(|error| format!("Create frame fence: {error}"))?;
            renderer.try_resize(width, height)?;
            Ok(renderer)
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.try_resize(width, height)
            .expect("resize surface renderer");
    }

    #[cfg(target_os = "android")]
    pub fn size(&self) -> (u32, u32) {
        (self.extent.width, self.extent.height)
    }

    fn try_resize(&mut self, width: u32, height: u32) -> Result<(), String> {
        if !self.recreate && self.extent.width == width && self.extent.height == height {
            return Ok(());
        }
        let _measure = shrimply_profiling::measure("GPU / Surface swapchain rebuild");
        shrimply_profiling::increment("GPU / Surface swapchains rebuilt");
        self.shared
            .wait_idle()
            .map_err(|error| format!("Wait before swapchain replacement: {error}"))?;
        unsafe {
            // Swapchain images are borrowed by frame-local Skia surfaces. Keep
            // the context's glyph/image caches and the independent compositor
            // texture alive across resizing instead of rebuilding them each frame.
            let caps = self
                .surface_api
                .get_physical_device_surface_capabilities(self.physical, self.surface)
                .map_err(|error| format!("Query surface capabilities: {error}"))?;
            // Android permits a chosen extent, and its reported current extent
            // can lag a SurfaceView resize. Follow the supplied pixel size so
            // SurfaceFlinger does not stretch a stale buffer into the new view.
            let extent = if cfg!(target_os = "android") || caps.current_extent.width == u32::MAX {
                vk::Extent2D {
                    width: width.clamp(caps.min_image_extent.width, caps.max_image_extent.width),
                    height: height
                        .clamp(caps.min_image_extent.height, caps.max_image_extent.height),
                }
            } else {
                caps.current_extent
            };
            if extent.width == 0 || extent.height == 0 {
                return Err("Surface must have a positive extent".into());
            }
            let formats = self
                .surface_api
                .get_physical_device_surface_formats(self.physical, self.surface)
                .map_err(|error| format!("Query surface formats: {error}"))?;
            let format = formats
                .iter()
                .find(|format| {
                    [vk::Format::R8G8B8A8_UNORM, vk::Format::B8G8R8A8_UNORM]
                        .contains(&format.format)
                        && format.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR
                })
                .ok_or("An RGBA/BGRA UNORM surface is required")?;
            let usage = vk::ImageUsageFlags::COLOR_ATTACHMENT
                | vk::ImageUsageFlags::TRANSFER_SRC
                | vk::ImageUsageFlags::TRANSFER_DST;
            if !caps.supported_usage_flags.contains(usage) {
                return Err("Surface must support Skia color and transfer usage".into());
            }
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
            .ok_or("Surface must support opaque or inherited alpha")?;
            // Let Android's compositor apply window rotation; Skia and input use
            // the same upright coordinates rather than implementing pre-rotation.
            if !caps
                .supported_transforms
                .contains(vk::SurfaceTransformFlagsKHR::IDENTITY)
            {
                return Err("An upright presentation surface is required".into());
            }
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
                .map_err(|error| format!("Create Vulkan swapchain: {error}"))?;
            for semaphore in self.rendered.drain(..) {
                self.device.destroy_semaphore(semaphore, None);
            }
            self.swapchain_api.destroy_swapchain(self.swapchain, None);
            self.swapchain = replacement;
            self.images = self
                .swapchain_api
                .get_swapchain_images(replacement)
                .map_err(|error| format!("Get swapchain images: {error}"))?;
            self.presented = vec![false; self.images.len()];
            for _ in &self.images {
                self.rendered.push(
                    self.device
                        .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
                        .map_err(|error| format!("Create presentation semaphore: {error}"))?,
                );
            }
            self.extent = extent;
            self.format = format.format;
            self.recreate = false;
        }
        Ok(())
    }

    /// Rebind presentation while retaining this rendering thread's Skia resource cache.
    ///
    /// # Safety
    /// A non-null surface must belong to this renderer's instance. Ownership is transferred;
    /// keep its native window alive until replacement has completed.
    pub unsafe fn replace_surface(&mut self, surface: vk::SurfaceKHR, width: u32, height: u32) {
        if self.surface == surface {
            if surface != vk::SurfaceKHR::null() {
                self.resize(width, height);
            }
            return;
        }
        if surface != vk::SurfaceKHR::null() {
            assert!(
                unsafe {
                    self.surface_api
                        .get_physical_device_surface_support(self.physical, self.family, surface)
                        .expect("query replacement presentation support")
                },
                "the shared Vulkan queue cannot present to this window"
            );
        }
        self.release_surface();
        self.surface = surface;
        if surface != vk::SurfaceKHR::null() {
            shrimply_profiling::increment("GPU / Surface context rebinds");
            self.resize(width, height);
        }
    }

    #[cfg(target_os = "android")]
    pub fn set_window(
        &mut self,
        window: Option<&ndk::native_window::NativeWindow>,
        width: u32,
        height: u32,
    ) {
        let surface = window.map_or(vk::SurfaceKHR::null(), |window| {
            android_surface(&self.shared, window)
                .expect("create replacement Android Vulkan surface")
        });
        unsafe { self.replace_surface(surface, width, height) };
    }

    fn release_surface(&mut self) {
        if self.surface == vk::SurfaceKHR::null() {
            return;
        }
        if self.swapchain != vk::SwapchainKHR::null()
            && let Err(error) = self.shared.wait_idle()
        {
            tracing::warn!(%error, "Wait for native surface teardown");
        }
        unsafe {
            #[cfg(feature = "renderer")]
            drop(self.sampled.take());
            for semaphore in self.rendered.drain(..) {
                self.device.destroy_semaphore(semaphore, None);
            }
            self.images.clear();
            self.presented.clear();
            self.swapchain_api.destroy_swapchain(self.swapchain, None);
            self.surface_api.destroy_surface(self.surface, None);
            self.swapchain = vk::SwapchainKHR::null();
            self.surface = vk::SurfaceKHR::null();
            self.recreate = true;
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
        let _sampling = texture.map(crate::Texture::sampling_lock);
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
        let _measure = shrimply_profiling::measure("GPU / Surface acquire through present");
        unsafe {
            let stage = shrimply_profiling::measure("GPU / Surface fence wait");
            self.device
                .wait_for_fences(&[self.fence], true, GPU_TIMEOUT_NS)
                .expect("timeline GPU fence timed out");
            drop(stage);
            let stage = shrimply_profiling::measure("GPU / Surface acquire image");
            let (index, suboptimal) = match self.swapchain_api.acquire_next_image(
                self.swapchain,
                GPU_TIMEOUT_NS,
                self.acquired,
                vk::Fence::null(),
            ) {
                Ok(result) => result,
                Err(vk::Result::ERROR_OUT_OF_DATE_KHR | vk::Result::ERROR_SURFACE_LOST_KHR) => {
                    self.recreate = true;
                    return false;
                }
                Err(error) => panic!("acquire timeline image: {error}"),
            };
            drop(stage);
            // Skia's proc provider serializes its actual queue operations,
            // including implicit submits during drawing. Context-local CPU
            // recording must not block other surfaces or the compositor.
            let stage = shrimply_profiling::measure("GPU / Surface draw and submit");
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
            let waiting = shrimply_profiling::measure("GPU / Surface wait for queue");
            let _queue = self.shared.queue_lock();
            drop(waiting);
            self.device
                .reset_fences(&[self.fence])
                .expect("reset timeline fence");
            self.device
                .queue_submit(self.queue, &[vk::SubmitInfo::default()], self.fence)
                .expect("fence timeline work");
            drop(stage);
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
            let stage = shrimply_profiling::measure("GPU / Surface queue present");
            let valid = match self.swapchain_api.queue_present(self.queue, &present) {
                Ok(changed) => !changed && !suboptimal,
                Err(vk::Result::ERROR_OUT_OF_DATE_KHR | vk::Result::ERROR_SURFACE_LOST_KHR) => {
                    false
                }
                Err(error) => panic!("present timeline: {error}"),
            };
            drop(stage);
            self.recreate |= !valid;
            valid
        }
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        shrimply_profiling::increment("GPU / Surface contexts destroyed");
        self.release_surface();
        unsafe {
            // release_surface already finished this context's submissions.
            drop(self.context.take());
            self.device.destroy_semaphore(self.acquired, None);
            self.device.destroy_fence(self.fence, None);
        }
    }
}

#[cfg(target_os = "android")]
fn android_surface(
    shared: &Device,
    window: &ndk::native_window::NativeWindow,
) -> Result<vk::SurfaceKHR, String> {
    unsafe {
        ash::khr::android_surface::Instance::new(&shared.entry, &shared.instance)
            .create_android_surface(
                &vk::AndroidSurfaceCreateInfoKHR::default().window(window.ptr().as_ptr().cast()),
                None,
            )
            .map_err(|error| format!("Create Android Vulkan surface: {error}"))
    }
}
