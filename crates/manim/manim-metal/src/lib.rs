#![cfg(target_os = "macos")]
use objc2_metal::MTLDevice;
use shrimply_manim_wgpu::{PreparedAnimation, Renderer as SharedRenderer};
use shrimply_render_metal::{Buffer, Renderer as MetalRenderer};
use std::collections::HashMap;

pub struct Renderer {
    shared: SharedRenderer,
    targets: HashMap<usize, wgpu::Texture>,
}

pub struct Frame {
    pub buffer: Buffer,
    pub row_bytes: usize,
}

impl Renderer {
    pub fn new(metal: &MetalRenderer) -> Result<Self, String> {
        let native = metal.device();
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = wgpu::Backends::METAL;
        let instance = wgpu::Instance::new(descriptor);
        let adapter = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::METAL))
            .into_iter()
            .find(|adapter| {
                // Only inspect Metal adapters, matching the compositor's physical device.
                unsafe { adapter.as_hal::<wgpu::hal::api::Metal>() }
                    .is_some_and(|adapter| adapter.raw_device().registryID() == native.registryID())
            })
            .ok_or("Manim cannot find the compositor's Metal device")?;
        let descriptor = wgpu::DeviceDescriptor {
            label: Some("Manim shared Metal device"),
            ..Default::default()
        };
        let queue = native
            .newCommandQueue()
            .ok_or("Could not create Manim Metal queue")?;
        // Construct WGPU on the exact MTLDevice owned by the compositor, rather
        // than independently selecting another GPU. Metal timestamps are nanoseconds.
        const METAL_TIMESTAMP_PERIOD_NS: f32 = 1.0;
        let hal = unsafe {
            wgpu::hal::OpenDevice::<wgpu::hal::api::Metal> {
                device: wgpu::hal::metal::Device::device_from_raw(
                    native.clone(),
                    descriptor.required_features,
                    &descriptor.required_limits,
                ),
                queue: wgpu::hal::metal::Queue::queue_from_raw(queue, METAL_TIMESTAMP_PERIOD_NS),
            }
        };
        let (device, queue) =
            unsafe { adapter.create_device_from_hal::<wgpu::hal::api::Metal>(hal, &descriptor) }
                .map_err(|e| format!("Create Manim Metal WGPU device: {e}"))?;
        Ok(Self {
            shared: SharedRenderer::from_device(device, queue),
            targets: HashMap::new(),
        })
    }

    /// Returns GPU storage only. Rendering and texture transfer complete on this
    /// worker before the consumer reads it or the target is reused.
    pub fn render(
        &mut self,
        metal: &MetalRenderer,
        slot: usize,
        animation: &PreparedAnimation,
        frame: usize,
    ) -> Result<Frame, String> {
        let descriptor = SharedRenderer::external_frame_descriptor(animation);
        if self.shared.target_descriptor(slot) != Some(descriptor) {
            self.shared.remove_target(slot);
            self.targets.insert(
                slot,
                self.shared.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("Manim Metal output"),
                    size: wgpu::Extent3d {
                        width: descriptor.width,
                        height: descriptor.height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                }),
            );
        }
        let texture = self
            .targets
            .get(&slot)
            .expect("initialized Manim Metal target")
            .clone();
        let row_bytes = descriptor
            .width
            .checked_mul(size_of::<u32>() as u32)
            .ok_or("Manim row size overflow")?
            .div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            .checked_mul(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            .ok_or("Manim row alignment overflow")?;
        let size = u64::from(row_bytes)
            .checked_mul(u64::from(descriptor.height))
            .ok_or("Manim buffer size overflow")?;
        let buffer = metal
            .allocate(usize::try_from(size).map_err(|_| "Manim output exceeds address space")?)?;
        // WGPU and the Slang compositor retain the same native allocation. No
        // mapping, CPU readback, re-encoding or alternate renderer is involved.
        let copy_buffer = unsafe {
            self.shared
                .device
                .create_buffer_from_hal::<wgpu::hal::api::Metal>(
                    wgpu::hal::metal::Device::buffer_from_raw(buffer.metal().clone(), size),
                    &wgpu::BufferDescriptor {
                        label: Some("Manim Metal interop buffer"),
                        size,
                        usage: wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    },
                )
        };
        let mut encoder =
            self.shared
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Manim render and Metal handoff"),
                });
        self.shared
            .encode(slot, animation, frame, texture.clone(), &mut encoder)?;
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &copy_buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row_bytes),
                    rows_per_image: Some(descriptor.height),
                },
            },
            texture.size(),
        );
        let submission = self.shared.queue.submit([encoder.finish()]);
        self.shared
            .device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: None,
            })
            .map_err(|e| format!("Wait for Manim Metal handoff: {e}"))?;
        Ok(Frame {
            buffer,
            row_bytes: row_bytes as usize,
        })
    }

    pub fn retain_slots(&mut self, active: &[usize]) {
        self.targets.retain(|slot, _| {
            if active.contains(slot) {
                true
            } else {
                self.shared.remove_target(*slot);
                false
            }
        });
        self.shared.clear_unused();
    }
}
