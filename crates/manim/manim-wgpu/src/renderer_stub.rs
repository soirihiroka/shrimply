//! Non-Linux stub for the Manim WGPU renderer. The real implementation exports
//! frames through Vulkan external memory for CUDA consumption, which only
//! exists on Linux; every operation reports an error on other platforms.

use std::sync::Arc;

use shrimply_manim_ir::CompiledAnimation;

pub struct PreparedAnimation {
    animation: Arc<CompiledAnimation>,
}

impl PreparedAnimation {
    pub fn new(animation: Arc<CompiledAnimation>) -> Result<Self, String> {
        Ok(Self { animation })
    }

    pub fn scene(&self) -> &shrimply_manim_ir::SceneHeader {
        self.animation.scene()
    }

    pub fn frame_count(&self) -> usize {
        self.animation.frames().len()
    }

    pub fn is_empty(&self) -> bool {
        self.animation.frames().is_empty()
    }
}

pub struct RenderedFrame {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExternalFrameDescriptor {
    pub width: u32,
    pub height: u32,
    pub samples: u32,
}

pub struct ExportedFrame {
    pub fd: std::os::fd::OwnedFd,
    pub semaphore_fd: std::os::fd::OwnedFd,
    pub allocation_size: u64,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RenderedExternalFrame {
    pub descriptor: ExternalFrameDescriptor,
    pub semaphore_value: u64,
}

impl RenderedFrame {
    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub fn into_pixels(self) -> Vec<u8> {
        self.pixels
    }
}

const NOT_SUPPORTED: &str =
    "Manim WGPU rendering requires Linux with Vulkan external memory support";

pub struct Renderer {}

impl Renderer {
    pub fn new() -> Result<Self, String> {
        Err(NOT_SUPPORTED.to_string())
    }

    pub fn render_rgba_for_validation(
        &mut self,
        animation: &PreparedAnimation,
        frame_index: usize,
    ) -> Result<RenderedFrame, String> {
        let _ = (animation, frame_index);
        Err(NOT_SUPPORTED.to_string())
    }

    pub fn external_frame_descriptor(animation: &PreparedAnimation) -> ExternalFrameDescriptor {
        let _ = animation;
        ExternalFrameDescriptor {
            width: 0,
            height: 0,
            samples: 0,
        }
    }

    pub fn target_descriptor(&self, slot: usize) -> Option<ExternalFrameDescriptor> {
        let _ = slot;
        None
    }

    pub fn release_render_surfaces(&mut self) -> bool {
        false
    }

    pub fn release_gpu_animation_resources(&mut self) -> bool {
        false
    }

    pub fn render_external(
        &mut self,
        slot: usize,
        animation: &PreparedAnimation,
        frame_index: usize,
    ) -> Result<RenderedExternalFrame, String> {
        let _ = (slot, animation, frame_index);
        Err(NOT_SUPPORTED.to_string())
    }

    pub fn export_frame(&self, slot: usize) -> Result<ExportedFrame, String> {
        let _ = slot;
        Err(NOT_SUPPORTED.to_string())
    }

    pub fn remove_target(&mut self, slot: usize) -> bool {
        let _ = slot;
        false
    }

    pub fn clear_unused(&mut self) -> bool {
        false
    }
}
