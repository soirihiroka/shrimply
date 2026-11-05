use shrimply_project_document::project::{ItemAddress, Project, Time};
use uuid::Uuid;

use crate::analysis_renderer::{Renderer, Target};

pub use shrimply_visual_core::transparent_fill::analysis::{
    FrameSource, FrameStatus, PreparedStatus, RunId, Status, active_run_prepared, cancel,
    prepare_status, status, status_for_run, status_prepared,
};

struct AnalysisFrameSource {
    renderer: Renderer,
}

impl FrameSource for AnalysisFrameSource {
    fn frame(
        &mut self,
        project: &Project,
        position: Time,
        address: &ItemAddress,
        width: u32,
        height: u32,
    ) -> Result<FrameStatus, String> {
        let Some(output) = self.renderer.frame(
            project,
            position,
            Target::ModifierInput {
                address,
                snap_content: true,
            },
        )?
        else {
            return Ok(FrameStatus::Pending);
        };
        if output.width() != width || output.height() != height {
            return Err("Transparent fill input dimensions changed".into());
        }
        let row_bytes = width as usize * 4;
        let stride = output.stride(0);
        let mut rgba = Vec::with_capacity(row_bytes * height as usize);
        for row in output.data(0).chunks_exact(stride).take(height as usize) {
            rgba.extend_from_slice(&row[..row_bytes]);
        }
        Ok(FrameStatus::Ready(rgba))
    }
}

pub fn analyze(
    project: Project,
    address: &ItemAddress,
    modifier_id: Uuid,
) -> Result<RunId, String> {
    shrimply_visual_core::transparent_fill::analysis::analyze_with(
        project,
        address,
        modifier_id,
        || Renderer::new(48_000).map(|renderer| AnalysisFrameSource { renderer }),
    )
}
