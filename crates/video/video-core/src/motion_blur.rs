use shrimply_evaluation::{
    FrameAudioAnalysis, TransformExpressionCache, resolve_item_transform_with_audio,
};
use shrimply_math_geometry::ComposedTransform2D;
use shrimply_project::project::{
    MAX_MOTION_BLUR_SAMPLES, MAX_MOTION_BLUR_SHUTTER_ANGLE_DEGREES,
    MAX_MOTION_BLUR_SHUTTER_PHASE_DEGREES, MIN_MOTION_BLUR_SAMPLES,
    MIN_MOTION_BLUR_SHUTTER_ANGLE_DEGREES, MIN_MOTION_BLUR_SHUTTER_PHASE_DEGREES, Project, Time,
    VideoItem,
};

const BEST_EFFORT_MOTION_BLUR_SAMPLE_CAP: u32 = 8;

pub struct Request<'a> {
    pub project: &'a Project,
    pub item: &'a VideoItem,
    pub position: Time,
    pub current: ComposedTransform2D,
    pub content_accurate: bool,
}

/// Sample the item's transform at the existing rational shutter positions.
/// Content/media and modifier animation remain at the current frame time.
pub fn sample_transforms(
    request: Request<'_>,
    expressions: &mut TransformExpressionCache,
    mut audio: impl FnMut(Time) -> FrameAudioAnalysis,
) -> Option<Vec<ComposedTransform2D>> {
    let Request {
        project,
        item,
        position,
        current,
        content_accurate,
    } = request;
    if !item.motion_blur.enabled {
        return None;
    }
    let mut samples = item
        .motion_blur
        .samples
        .clamp(MIN_MOTION_BLUR_SAMPLES, MAX_MOTION_BLUR_SAMPLES);
    if !content_accurate {
        samples = samples.min(BEST_EFFORT_MOTION_BLUR_SAMPLE_CAP);
    }
    let positions = shrimply_math_media::motion_blur_sample_positions(
        position,
        item.start,
        item.end,
        project.fps,
        item.motion_blur.shutter_angle_degrees.clamp(
            MIN_MOTION_BLUR_SHUTTER_ANGLE_DEGREES,
            MAX_MOTION_BLUR_SHUTTER_ANGLE_DEGREES,
        ),
        item.motion_blur.shutter_phase_degrees.clamp(
            MIN_MOTION_BLUR_SHUTTER_PHASE_DEGREES,
            MAX_MOTION_BLUR_SHUTTER_PHASE_DEGREES,
        ),
        samples,
    );
    let transforms = positions
        .into_iter()
        .map(|position| {
            resolve_item_transform_with_audio(
                project,
                item,
                position,
                &audio(position),
                expressions,
            )
            .composed()
        })
        .collect::<Vec<_>>();
    (!transforms.iter().all(|sample| *sample == current)).then_some(transforms)
}
