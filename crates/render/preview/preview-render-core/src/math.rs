/// An opaque native video already covers the canvas without compositing changes.
#[cfg(target_os = "linux")]
pub fn unmodified_video(
    plan: &crate::FramePlan,
) -> Option<&std::sync::Arc<shrimply_visual_frame::VideoFrame>> {
    use shrimply_render_core::{LayerBlendMode, VideoSampleMethod, math::Mat3};
    let [layer] = plan.layers.as_slice() else {
        return None;
    };
    let crate::Source::Video(video) = &layer.source else {
        return None;
    };
    let parameters = layer.parameters;
    if !plan.external_layers.is_empty()
        || !layer.effects.is_empty()
        || !layer.transitions.is_empty()
        || layer.motion_blur.is_some()
        || layer.alpha_mask.is_some()
        || layer.video_mask.is_some()
        || layer.stabilization.is_some()
        || layer.morph_scene.is_some()
        || layer.render_size != (plan.width, plan.height)
        || layer.transform != Mat3::IDENTITY
        || layer.output_transform != Mat3::IDENTITY
        || parameters.opacity != 1.0
        || parameters.crop != [0.0; 4]
        || parameters.padding != [0.0; 4]
        || parameters.motion_sample_count != 0
        || parameters.motion_transform_count != 0
        || (parameters.source_width, parameters.source_height) != (plan.width, plan.height)
        || !matches!(
            parameters.blend_mode,
            LayerBlendMode::Normal | LayerBlendMode::PassThrough
        )
        || !matches!(
            parameters.sample_method,
            VideoSampleMethod::Nearest | VideoSampleMethod::Bilinear | VideoSampleMethod::Bicubic
        )
        || (video.native().width(), video.native().height()) != (plan.width, plan.height)
    {
        None
    } else {
        Some(video)
    }
}
