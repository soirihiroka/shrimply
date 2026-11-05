use shrimply_3dgs_core::TrackingCameraSource;
use shrimply_project_document::project::{ItemAddress, Project, Time, TrackAddress};

use crate::analysis_renderer::{Renderer, Target};

pub use shrimply_visual_core::camera_reconstruction::{
    AnalysisStatus, ReconstructedCameraSample, apply_custom_camera_offset, cancel,
    has_matching_cache, sample, sample_item, status,
};

const ANALYSIS_AUDIO_SAMPLE_RATE: u32 = 48_000;

pub fn analyze(
    project: Project,
    camera_address: ItemAddress,
    source: TrackingCameraSource,
    server_url: String,
) {
    shrimply_visual_core::camera_reconstruction::analyze(
        project,
        camera_address,
        source,
        server_url,
        renderer,
    );
}

fn renderer()
-> Result<Box<dyn shrimply_visual_core::camera_reconstruction::AnalysisFrameRenderer>, String> {
    Renderer::new(ANALYSIS_AUDIO_SAMPLE_RATE).map(|renderer| Box::new(renderer) as Box<_>)
}

impl shrimply_visual_core::camera_reconstruction::AnalysisFrameRenderer for Renderer {
    fn render_jpeg(
        &mut self,
        project: &Project,
        position: Time,
        track: &TrackAddress,
        cancelled: &std::sync::atomic::AtomicBool,
    ) -> Result<Option<Vec<u8>>, String> {
        if cancelled.load(std::sync::atomic::Ordering::Acquire) {
            return Err("camera analysis cancelled".into());
        }
        let Some(rgba) = self.frame(project, position, Target::Track(track))? else {
            return Ok(None);
        };
        shrimply_visual_core::camera_reconstruction::encode_tracking_frame(
            rgba.data(0),
            project.canvas_size,
            rgba.stride(0),
        )
    }
}
