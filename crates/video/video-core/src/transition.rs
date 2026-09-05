//! Transition timing and spatial state shared by GPU backends.
use shrimply_project::project::{
    Time, TransitionSide, VideoItem, VisualTransition, VisualTransitionKind,
};

pub fn active_visual_transition(
    item: &VideoItem,
    position: Time,
) -> Option<(TransitionSide, &VisualTransition, f32, f32)> {
    for (side, transition) in [
        (TransitionSide::Intro, item.transitions.intro.as_ref()),
        (TransitionSide::Outro, item.transitions.outro.as_ref()),
    ] {
        let Some(transition) = transition else {
            continue;
        };
        if let Some(progress) = shrimply_math_media::transition_progress(
            item.start,
            item.end,
            transition.duration,
            side,
            position,
        ) {
            let eased = transition.interpolation.value(f64::from(progress)) as f32;
            let visible = match side {
                TransitionSide::Intro => eased,
                TransitionSide::Outro => 1.0 - eased,
            };
            return Some((side, transition, visible, progress));
        }
    }
    None
}

#[derive(Clone, Copy)]
pub struct Spatial {
    pub opacity: f32,
    pub transform: glam::Mat3,
}

pub fn spatial(transition: &VisualTransition, visible: f32, center: glam::Vec2) -> Spatial {
    let mut spatial = Spatial {
        opacity: 1.0,
        transform: glam::Mat3::IDENTITY,
    };
    if matches!(
        transition.kind,
        VisualTransitionKind::Fade | VisualTransitionKind::SlideFade
    ) {
        spatial.opacity *= visible.clamp(0.0, 1.0);
    }
    if matches!(
        transition.kind,
        VisualTransitionKind::Slide | VisualTransitionKind::SlideFade
    ) {
        let offset = shrimply_math_media::polar_degrees(
            transition.slide_distance * (1.0 - visible),
            transition.slide_rotation_degrees,
        );
        spatial.transform = glam::Mat3::from_translation(offset);
    }
    if matches!(
        transition.kind,
        VisualTransitionKind::Zoom | VisualTransitionKind::Spin
    ) {
        let scale =
            shrimply_math_media::lerp(transition.effect_amount.clamp(0.0, 2.0), 1.0, visible);
        let rotation = if transition.kind == VisualTransitionKind::Spin {
            transition.effect_angle_degrees * (1.0 - visible)
        } else {
            0.0
        };
        spatial.transform = glam::Mat3::from_scale_angle_translation(
            glam::Vec2::splat(scale),
            rotation.to_radians(),
            center,
        ) * glam::Mat3::from_translation(-center);
    }
    if transition.effect_fade
        && matches!(
            transition.kind,
            VisualTransitionKind::Blur | VisualTransitionKind::Pixelate
        )
    {
        spatial.opacity *= visible;
    }
    spatial
}

use shrimply_project::project::{VisualClipTransition, VisualClipTransitionKind};
use shrimply_render_core::{
    VisualTransitionMaskKind,
    effects::{PixelEffect, TransitionMask},
};
pub fn raster(
    transition: &VisualTransition,
    visibility: f32,
    center: glam::Vec2,
) -> Option<PixelEffect> {
    let visibility = visibility.clamp(0.0, 1.0);
    match transition.kind {
        VisualTransitionKind::Wipe => Some(PixelEffect::TransitionMask(TransitionMask {
            kind: VisualTransitionMaskKind::Wipe,
            visibility,
            angle_degrees: transition.effect_angle_degrees,
            softness: transition.effect_detail,
            center,
            normalized_center: false,
            grain_size: 1,
            line_variation: 0.0,
        })),
        VisualTransitionKind::Iris => Some(PixelEffect::TransitionMask(TransitionMask {
            kind: VisualTransitionMaskKind::Iris,
            visibility,
            angle_degrees: 0.0,
            softness: transition.effect_detail,
            center: transition.iris_center,
            normalized_center: true,
            grain_size: u32::from(transition.effect_amount >= 0.5),
            line_variation: 0.0,
        })),
        VisualTransitionKind::ClockWipe => Some(PixelEffect::TransitionMask(TransitionMask {
            kind: VisualTransitionMaskKind::ClockWipe,
            visibility,
            angle_degrees: transition.effect_angle_degrees,
            softness: transition.effect_detail,
            center,
            normalized_center: false,
            grain_size: u32::from(transition.effect_amount >= 0.5),
            line_variation: 0.0,
        })),
        VisualTransitionKind::Blur => {
            let radius = (transition.effect_amount * (1.0 - visibility))
                .round()
                .clamp(0.0, 100.0) as u32;
            if radius > 0 {
                return Some(PixelEffect::TransitionBlur(radius));
            }
            None
        }
        VisualTransitionKind::Pixelate => {
            let block_size = shrimply_math_media::lerp(
                transition.effect_amount.clamp(1.0, 512.0),
                1.0,
                visibility,
            )
            .round() as u32;
            if block_size > 1 {
                return Some(PixelEffect::TransitionPixelate(block_size));
            }
            None
        }
        VisualTransitionKind::Dissolve => Some(PixelEffect::TransitionMask(TransitionMask {
            kind: VisualTransitionMaskKind::Dissolve,
            visibility,
            angle_degrees: 0.0,
            softness: 0.0,
            center,
            normalized_center: false,
            grain_size: transition.effect_detail.round().clamp(1.0, 64.0) as u32,
            line_variation: 0.0,
        })),
        VisualTransitionKind::TriangularFold => Some(PixelEffect::TransitionMask(TransitionMask {
            kind: VisualTransitionMaskKind::TriangularFold,
            visibility,
            angle_degrees: transition.effect_angle_degrees,
            softness: transition.effect_amount,
            center,
            normalized_center: false,
            grain_size: transition.effect_detail.round().clamp(32.0, 512.0) as u32,
            line_variation: 0.0,
        })),
        VisualTransitionKind::StreakWipe => Some(PixelEffect::TransitionMask(TransitionMask {
            kind: VisualTransitionMaskKind::StreakWipe,
            visibility,
            angle_degrees: transition.effect_angle_degrees,
            softness: transition.effect_softness,
            center,
            normalized_center: false,
            grain_size: transition.effect_detail.round().clamp(1.0, 256.0) as u32,
            line_variation: transition.effect_amount.clamp(0.0, 1.0),
        })),
        _ => None,
    }
}

pub fn clip_mask(transition: &VisualClipTransition, progress: f32) -> Option<PixelEffect> {
    let (kind, center, normalized_center, grain_size) = match transition.kind {
        VisualClipTransitionKind::Wipe => {
            (VisualTransitionMaskKind::Wipe, glam::Vec2::ZERO, false, 1)
        }
        VisualClipTransitionKind::Iris => (
            VisualTransitionMaskKind::Iris,
            transition.center,
            true,
            u32::from(transition.iris_from_inside),
        ),
        VisualClipTransitionKind::Dissolve => (
            VisualTransitionMaskKind::Dissolve,
            glam::Vec2::ZERO,
            false,
            transition.dissolve_grain_size,
        ),
        VisualClipTransitionKind::ClockWipe => (
            VisualTransitionMaskKind::ClockWipe,
            transition.center,
            true,
            u32::from(transition.clockwise),
        ),
        _ => return None,
    };
    Some(PixelEffect::TransitionMask(TransitionMask {
        kind,
        visibility: progress.clamp(0.0, 1.0),
        angle_degrees: transition.direction_degrees,
        softness: transition.softness,
        center,
        normalized_center,
        grain_size,
        line_variation: 0.0,
    }))
}
