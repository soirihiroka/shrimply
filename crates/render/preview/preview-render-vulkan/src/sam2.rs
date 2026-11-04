use super::Compositor;
use shrimply_preview_render_core::{CaptureTarget, CompositeAccuracy};
use shrimply_visual_core::sam2::analysis::{ProxyFrameRequest, ProxyFrameSource, ProxyFrameStatus};

pub struct Sam2ProxyFrameSource {
    compositor: Compositor,
}

impl Sam2ProxyFrameSource {
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            compositor: Compositor::new()?,
        })
    }
}

impl ProxyFrameSource for Sam2ProxyFrameSource {
    fn frame(&mut self, request: ProxyFrameRequest<'_>) -> Result<ProxyFrameStatus, String> {
        if self.compositor.sam2_target.as_ref() != Some(request.target) {
            self.compositor.invalidate();
            self.compositor
                .set_capture_target(Some(CaptureTarget::Item(request.target.address.clone())));
            self.compositor.sam2_target = Some(request.target.clone());
        }
        self.compositor.sam2_proxy = None;
        self.compositor.scene.request_frame();
        let Some(plan) = self.compositor.prepare(
            request.project,
            request.timeline_position,
            CompositeAccuracy::FULLY_ACCURATE,
        )?
        else {
            return Ok(ProxyFrameStatus::Pending);
        };
        let rendered = self.compositor.render_gpu(plan, 0)?;
        rendered.wait()?;
        if rendered.loading {
            return Ok(ProxyFrameStatus::Pending);
        }
        let proxy = self
            .compositor
            .sam2_proxy
            .take()
            .ok_or("SAM2 modifier did not capture an Intel proxy frame")?;
        Ok(ProxyFrameStatus::Ready(
            self.compositor.renderer.readback(&proxy)?,
        ))
    }
}
