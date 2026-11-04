use shrimply_project_document::project::{ItemAddress, Project};
use shrimply_resource_pipeline::JobContext;
use shrimply_visual_core::modifier_cache::encoder::HevcBackend;
use uuid::Uuid;

use crate::analysis_renderer::{Renderer, Target};

pub use shrimply_visual_core::modifier_cache::{Status, effective_item, invalidate, status};

pub fn bake(project: Project, address: ItemAddress, modifier_id: Uuid) -> Result<(), String> {
    shrimply_visual_core::modifier_cache::bake(project, address, modifier_id, bake_inner)
}

fn bake_inner(
    request: shrimply_visual_core::modifier_cache::BakeRequest,
    context: &JobContext<shrimply_visual_core::modifier_cache::Progress>,
) -> Result<(), String> {
    let mut renderer = Renderer::new(48_000)?;
    let alpha_mode = renderer.alpha_mode();
    let backend = HevcBackend::Nvenc;
    #[cfg(target_os = "linux")]
    let backend = if crate::gpu::intel_backend_selected() {
        HevcBackend::Vaapi {
            device: shrimply_render_vulkan::device::Device::get()?
                .drm_path
                .clone(),
        }
    } else {
        backend
    };
    shrimply_visual_core::modifier_cache::render_and_encode(
        request,
        context,
        backend,
        alpha_mode,
        move |project, position, address| {
            renderer.frame(
                project,
                position,
                Target::ModifierInput {
                    address,
                    snap_content: false,
                },
            )
        },
    )
}
