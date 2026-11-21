use crate::Compositor;
use shrimply_preview_render_core::CaptureTarget;
use shrimply_project_document::project::{ItemAddress, Project};
use shrimply_visual_core::modifier_cache::{self, BakeRequest};

pub fn bake(project: Project, address: ItemAddress, modifier_id: uuid::Uuid) -> Result<(), String> {
    modifier_cache::bake_images(
        project,
        address,
        modifier_id,
        |request: BakeRequest, context| {
            let mut renderer = Compositor::new()?;
            modifier_cache::render_and_store_images(
                request,
                context,
                move |project, position, address| {
                    renderer.capture_rgba(
                        project,
                        position,
                        CaptureTarget::ModifierInput {
                            address: address.clone(),
                            snap_content: false,
                        },
                    )
                },
            )
        },
    )
}
