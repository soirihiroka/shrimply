use super::*;

struct ProxyFrameSource {
    sessions: RenderSessions,
    render_cache: RenderCache,
    compositor: CudaVideoCompositor,
}

impl ProxyFrameSource {
    fn new() -> Result<Self, String> {
        Ok(Self {
            sessions: RenderSessions::default(),
            render_cache: RenderCache::default(),
            compositor: CudaVideoCompositor::new()?,
        })
    }
}

impl crate::sam2_analysis::ProxyFrameSource for ProxyFrameSource {
    fn frame(
        &mut self,
        request: crate::sam2_analysis::ProxyFrameRequest<'_>,
    ) -> Result<crate::sam2_analysis::ProxyFrameStatus, String> {
        self.compositor.begin_sam2_analysis(request.target.clone());
        let result = (|| {
            let volume_revision = self.sessions.volume_revision;
            let audio_analysis = FrameAudioAnalysis {
                volume: self.sessions.volume.sample(
                    request.project,
                    request.timeline_position,
                    volume_revision,
                ),
                mouth: self.sessions.mouth.sample(
                    request.project,
                    request.timeline_position,
                    volume_revision,
                ),
            };
            let rendered = render_project_frame(
                request.project,
                request.timeline_position,
                &mut self.sessions,
                &mut self.render_cache,
                &mut self.compositor,
                RenderMode::Preview {
                    accuracy: CompositeAccuracy::FULLY_ACCURATE,
                },
                &audio_analysis,
                None,
                None,
                Some(&request.target.address),
                false,
                None,
                None,
            );
            shrimply_manim_state::validate_render_updates(rendered.manim_updates)?;
            if !rendered.errors.is_empty() {
                return Err(format!(
                    "SAM2 clip analysis failed at {}: {}",
                    request.timeline_position.as_label(),
                    rendered.errors.join("\n")
                ));
            }
            if rendered.loading {
                return Ok(crate::sam2_analysis::ProxyFrameStatus::Pending);
            }
            self.compositor
                .take_sam2_proxy()
                .map(crate::sam2_analysis::ProxyFrameStatus::Ready)
                .ok_or_else(|| "SAM2 modifier did not capture a proxy frame".to_string())
        })();
        self.compositor.end_sam2_analysis();
        result
    }
}

pub(super) fn schedule_analysis(
    project: &Project,
    scheduler: &mut crate::sam2_analysis::Scheduler,
    event_tx: SyncSender<VideoEvent>,
) -> Result<bool, String> {
    scheduler.schedule_next_with(project, ProxyFrameSource::new, move |error| {
        let _ = event_tx.try_send(VideoEvent::Error(error));
    })
}
