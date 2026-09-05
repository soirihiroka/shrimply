//! Shared clip-pair selection and transition state, preserving CUDA ordering.
use super::transition::Spatial;
use shrimply_project::{
    project::{CanvasSize, Time, VideoItem, VisualClipTransition, VisualClipTransitionKind},
    timeline_search,
};
use uuid::Uuid;

pub struct ActiveItem<'a> {
    pub item: &'a VideoItem,
    pub clip_transition: Option<ActiveClipTransition>,
    pub previous: Option<&'a VideoItem>,
}

#[derive(Clone, Copy)]
pub struct ActiveClipTransition {
    pub definition: VisualClipTransition,
    pub progress: f32,
    pub role: ClipTransitionRole,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum ClipTransitionRole {
    Outgoing,
    Incoming,
}

pub fn active_items<'a>(
    items: &'a [VideoItem],
    position: Time,
    item_ids: Option<&[Uuid]>,
) -> Vec<ActiveItem<'a>> {
    for pair in items.windows(2) {
        let outgoing = &pair[0];
        let incoming = &pair[1];
        let Some(transition) = outgoing.transitions.to_next.as_ref() else {
            continue;
        };
        if transition.target_item_id != incoming.id || outgoing.end != incoming.start {
            continue;
        }
        let Some(progress) = shrimply_math_media::clip_transition_progress(
            outgoing.end,
            transition.duration,
            position,
        ) else {
            continue;
        };
        let progress = (transition.interpolation.value(f64::from(progress)) as f32).clamp(0.0, 1.0);
        return [
            (outgoing, ClipTransitionRole::Outgoing),
            (incoming, ClipTransitionRole::Incoming),
        ]
        .into_iter()
        .filter(|(item, _)| item_ids.is_none_or(|item_ids| item_ids.contains(&item.id)))
        .map(|(item, role)| ActiveItem {
            item,
            previous: None,
            clip_transition: Some(ActiveClipTransition {
                definition: *transition,
                progress,
                role,
            }),
        })
        .collect();
    }
    timeline_search::overlapping(items, position, position)
        .filter(|(_, item)| {
            position >= item.start
                && position < item.end
                && item_ids.is_none_or(|item_ids| item_ids.contains(&item.id))
        })
        .map(|(_, item)| ActiveItem {
            item,
            clip_transition: None,
            previous: items
                .iter()
                .position(|candidate| candidate.id == item.id)
                .and_then(|index| index.checked_sub(1))
                .map(|index| &items[index]),
        })
        .collect()
}

pub fn spatial(transition: ActiveClipTransition, render_canvas: CanvasSize) -> Spatial {
    let mut spatial = Spatial {
        opacity: 1.0,
        transform: glam::Mat3::IDENTITY,
    };
    let definition = &transition.definition;
    let incoming = transition.role == ClipTransitionRole::Incoming;
    match definition.kind {
        VisualClipTransitionKind::CrossFade if incoming => {
            spatial.opacity *= transition.progress;
        }
        VisualClipTransitionKind::FadeThroughColor if incoming => {
            spatial.opacity *= (transition.progress * 2.0 - 1.0).max(0.0);
        }
        VisualClipTransitionKind::Slide if incoming => {
            let distance = shrimply_math_media::clip_transition_travel_distance(
                render_canvas,
                definition.direction_degrees,
            );
            let offset = shrimply_math_media::polar_degrees(
                distance * (1.0 - transition.progress),
                definition.direction_degrees,
            );
            spatial.transform = glam::Mat3::from_translation(offset);
        }
        VisualClipTransitionKind::Push => {
            let distance = shrimply_math_media::clip_transition_travel_distance(
                render_canvas,
                definition.direction_degrees,
            );
            let offset = shrimply_math_media::polar_degrees(
                match transition.role {
                    ClipTransitionRole::Outgoing => -distance * transition.progress,
                    ClipTransitionRole::Incoming => distance * (1.0 - transition.progress),
                },
                definition.direction_degrees,
            );
            spatial.transform = glam::Mat3::from_translation(offset);
        }
        VisualClipTransitionKind::Zoom if incoming => {
            let center = definition.center
                * glam::Vec2::new(
                    render_canvas.width.max(1) as f32,
                    render_canvas.height.max(1) as f32,
                );
            let scale =
                shrimply_math_media::lerp(definition.zoom_start_scale, 1.0, transition.progress);
            spatial.transform =
                glam::Mat3::from_scale_angle_translation(glam::Vec2::splat(scale), 0.0, center)
                    * glam::Mat3::from_translation(-center);
        }
        VisualClipTransitionKind::Morph
        | VisualClipTransitionKind::CrossFade
        | VisualClipTransitionKind::FadeThroughColor
        | VisualClipTransitionKind::Wipe
        | VisualClipTransitionKind::Iris
        | VisualClipTransitionKind::ClockWipe
        | VisualClipTransitionKind::Dissolve
        | VisualClipTransitionKind::Slide
        | VisualClipTransitionKind::Zoom => {}
    }
    if incoming
        && definition.fade_opacity
        && matches!(
            definition.kind,
            VisualClipTransitionKind::Slide
                | VisualClipTransitionKind::Push
                | VisualClipTransitionKind::Zoom
        )
    {
        spatial.opacity *= transition.progress;
    }
    spatial
}

/// Transition pairs stay drawable outside each clip's nominal edit interval.
/// Match CUDA's source handling while leaving property evaluation at timeline time.
pub fn held_item(
    item: &VideoItem,
    position: Time,
    active: bool,
) -> std::borrow::Cow<'_, VideoItem> {
    if active && (position < item.start || position >= item.end) {
        let mut held = item.clone();
        held.repeat_strategy = shrimply_project::project::RepeatStrategy::Hold;
        std::borrow::Cow::Owned(held)
    } else {
        std::borrow::Cow::Borrowed(item)
    }
}

pub fn color_layer(
    transition: ActiveClipTransition,
) -> Option<(shrimply_render_core::Color<u8>, f32)> {
    (transition.role == ClipTransitionRole::Outgoing
        && transition.definition.kind == VisualClipTransitionKind::FadeThroughColor)
        .then_some((
            transition.definition.fade_color,
            (transition.progress * 2.0).min(1.0),
        ))
}
