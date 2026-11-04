mod renderer;
mod source;

pub use renderer::{ExternalFrameDescriptor, PreparedAnimation, RenderedFrame, Renderer};
pub use shrimply_manim_state::{SourceIdentity, Update};
pub use source::{CompiledFrame, Source, SourceStatus, effective_fps, loading_pixels};

#[cfg(all(feature = "vulkan", any(target_os = "linux", windows)))]
pub mod vulkan;
