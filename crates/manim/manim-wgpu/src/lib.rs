#[cfg(target_os = "linux")]
mod renderer;
#[cfg(not(target_os = "linux"))]
mod renderer_stub;

#[cfg(target_os = "linux")]
pub use renderer::{
    ExportedFrame, ExternalFrameDescriptor, PreparedAnimation, RenderedExternalFrame,
    RenderedFrame, Renderer,
};
#[cfg(not(target_os = "linux"))]
pub use renderer_stub::{
    ExportedFrame, ExternalFrameDescriptor, PreparedAnimation, RenderedExternalFrame,
    RenderedFrame, Renderer,
};
