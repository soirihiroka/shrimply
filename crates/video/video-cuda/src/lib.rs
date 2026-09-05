pub(crate) mod alpha_mask;
mod background_render;
mod blender_render;
pub mod camera_reconstruction;
pub mod compositor;
pub mod decode;
mod gaussian_render;
pub mod gpu;
pub mod image_decode;
pub mod layer;
mod layered_image_render;
pub use layered_image_render::load_layered_image;
mod manim_render;
pub mod modifier_cache;
pub mod modifiers;
mod obj_render;
mod paint_render;
pub(crate) use shrimply_video_core::path_transition;
mod pdf_render;
pub mod preview;
pub(crate) use shrimply_video_core::shaky_path;
pub use shrimply_video_modifiers::sam2_analysis;
pub mod shape_render;
pub use shrimply_project::svg_color;
pub mod svg_render;
pub use shrimply_video_core::text_layout;
pub mod text_render;
pub mod transparent_fill_analysis;
pub(crate) use shrimply_video_core::vector_morph;
pub mod video_stabilization;
pub mod visual_bounds;
pub mod visual_source;
pub(crate) mod visual_transition;

#[doc(hidden)]
pub mod video_shader {
    include!(concat!(env!("OUT_DIR"), "/slang_bindings.rs"));
}

use shrimply_math_media as math;

pub use shrimply_project::project;

pub fn validate_sam2_cache(project: &project::Project) -> Result<(), String> {
    modifiers::sam2::validate_cache(project)
}

pub fn validate_transparent_fill_cache(project: &project::Project) -> Result<(), String> {
    modifiers::transparent_fill::validate_cache(project)
}
