//! Analysis uses the same accurate GPU scene composition as preview and export.
use crate::Compositor;
use shrimply_preview_render_core::CaptureTarget;
use shrimply_project_document::project::{ItemAddress, Project, Time, TrackAddress};
use shrimply_visual_core::{camera_reconstruction, transparent_fill::analysis as fill};
use std::sync::atomic::AtomicBool;

pub fn transparent_fill(
    project: Project,
    address: &ItemAddress,
    modifier_id: uuid::Uuid,
) -> Result<fill::RunId, String> {
    fill::analyze_with(project, address, modifier_id, Compositor::new)
}

impl fill::FrameSource for Compositor {
    fn frame(
        &mut self,
        project: &Project,
        position: Time,
        address: &ItemAddress,
        width: u32,
        height: u32,
    ) -> Result<fill::FrameStatus, String> {
        let Some(pixels) = self.capture_rgba(
            project,
            position,
            CaptureTarget::ModifierInput {
                address: address.clone(),
                snap_content: true,
            },
        )?
        else {
            return Ok(fill::FrameStatus::Pending);
        };
        if pixels.len() != width as usize * height as usize * size_of::<u32>() {
            return Err("Transparent fill frame dimensions changed during analysis".into());
        }
        Ok(fill::FrameStatus::Ready(pixels))
    }
}

pub fn camera(
    project: Project,
    address: ItemAddress,
    source: shrimply_3dgs_core::TrackingCameraSource,
    server_url: String,
) {
    camera_reconstruction::analyze(project, address, source, server_url, || {
        Compositor::new().map(|renderer| Box::new(renderer) as Box<_>)
    });
}

impl camera_reconstruction::AnalysisFrameRenderer for Compositor {
    fn render_jpeg(
        &mut self,
        project: &Project,
        position: Time,
        track: &TrackAddress,
        cancelled: &AtomicBool,
    ) -> Result<Option<Vec<u8>>, String> {
        let frame = self.render_rgba(
            project,
            position,
            Some(CaptureTarget::Track(track.clone())),
            cancelled,
        )?;
        camera_reconstruction::encode_tracking_frame(
            &frame.pixels,
            project.canvas_size,
            project.canvas_size.width as usize * size_of::<u32>(),
        )
    }
}
