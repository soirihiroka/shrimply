use super::{Compositor, ExternalMasks};
use shrimply_preview_render_core::{Layer, RasterMorph};
use shrimply_render_core::Nv12LayerParams;
use shrimply_render_vulkan::{Buffer, OpticalFlow, Submission};

pub(super) struct Cached {
    source: (Nv12LayerParams, Buffer),
    target: (Nv12LayerParams, Buffer),
    flow: OpticalFlow,
}

impl Compositor {
    pub(super) fn raster_morph(
        &mut self,
        morph: &RasterMorph,
        size: (u32, u32),
        masks: &ExternalMasks,
        submissions: &mut Vec<Submission>,
    ) -> Result<Vec<(Nv12LayerParams, Buffer)>, String> {
        let cached = match self.morphs.remove(&morph.key) {
            Some(cached) => cached,
            None => {
                let source = self.morph_endpoint(&morph.outgoing, size, masks, submissions)?;
                let target = self.morph_endpoint(&morph.incoming, size, masks, submissions)?;
                let flow = self.renderer.optical_flow(
                    source.1.clone(),
                    target.1.clone(),
                    size,
                    submissions,
                )?;
                Cached {
                    source,
                    target,
                    flow,
                }
            }
        };
        let progress = morph.progress.clamp(0.0, 1.0);
        let (source, submission) = self.renderer.mesh_flow_buffer(
            cached.source.1.clone(),
            size,
            cached.flow.grid,
            cached.flow.forward.clone(),
            -progress,
        )?;
        submissions.push(submission);
        let (target, submission) = self.renderer.mesh_flow_buffer(
            cached.target.1.clone(),
            size,
            cached.flow.grid,
            cached.flow.backward.clone(),
            -(1.0 - progress),
        )?;
        submissions.push(submission);
        let source_parameters = cached.source.0;
        let mut target_parameters = cached.target.0;
        target_parameters.opacity *= progress;
        if morph.cacheable {
            self.used_morphs.insert(morph.key.clone());
            self.morphs.insert(morph.key.clone(), cached);
        }
        Ok(vec![
            (source_parameters, source),
            (target_parameters, target),
        ])
    }

    fn morph_endpoint(
        &mut self,
        layer: &Layer,
        size: (u32, u32),
        masks: &ExternalMasks,
        submissions: &mut Vec<Submission>,
    ) -> Result<(Nv12LayerParams, Buffer), String> {
        let rendered = self.layers(std::slice::from_ref(layer), size, masks, submissions)?;
        if rendered.len() != 1 {
            return Err("Morph endpoint did not produce one raster layer".into());
        }
        let (parameters, buffer) = rendered.into_iter().next().expect("one morph endpoint");
        let (source, baked) =
            shrimply_render_core::effects::materialization(parameters, size.0, size.1);
        let (buffer, submission) = self
            .renderer
            .composite_buffers(&[(source, buffer)], size.0, size.1, 0)?
            .into_parts();
        submissions.push(submission);
        Ok((baked, buffer))
    }
}
