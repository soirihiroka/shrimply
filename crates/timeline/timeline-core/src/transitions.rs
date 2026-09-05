use crate::Fraction;
use crate::project::{Project, Time, TransitionSide};
use crate::{
    geometry::timeline_x,
    items::{ItemEdge, transition_durations},
    timeline_operation::{SequenceTimeline, TimelineOperationContext},
    view::{ClipTransitionDrag, TimelineViewState, TransitionDrag},
};
use shrimply_timeline_snap::SnapRepo;

pub fn update_clip_transition_drag(
    drag: &mut ClipTransitionDrag,
    project: &Project,
    view: TimelineViewState,
    x: f64,
    snap_repository: &SnapRepo,
) {
    if drag.center_resize {
        let track = drag.outgoing.track();
        let Some(timeline_cut) = project.sequence_time_to_timeline(&track, drag.cut) else {
            return;
        };
        let cut_x = timeline_x()
            + (timeline_cut.as_secs_f64() - view.scroll_seconds) / view.seconds_per_pixel;
        let target = crate::math::time_at_x(view, cut_x + x - view.drag_start_x);
        let target = snap_repository.snap(target).unwrap_or(target);
        let Some(target) = project
            .timeline_time_to_sequence(&track, target)
            .map(|target| target.snapped(project.frame_step()))
        else {
            return;
        };
        let Some((minimum, maximum)) =
            clip_transition_cut_range(drag, project, crate::geometry::frame_step(project))
        else {
            return;
        };
        drag.target_cut = target.max(minimum).min(maximum);
        return;
    }
    let Some(handle) = drag.handle else {
        return;
    };
    let target = crate::math::time_at_x(view, x);
    let target = snap_repository.snap(target).unwrap_or(target);
    let track = drag.outgoing.track();
    let Some(target) = project
        .timeline_time_to_sequence(&track, target)
        .map(|target| target.snapped(project.frame_step()))
    else {
        return;
    };
    let distance = match handle {
        ItemEdge::Start => drag.cut.saturating_sub(target),
        ItemEdge::End => target.saturating_sub(drag.cut),
    };
    let Some((left_start, left_end)) = project.item(&drag.outgoing).map(|item| item.times()) else {
        return;
    };
    let Some((right_start, right_end)) = project.item(&drag.incoming).map(|item| item.times())
    else {
        return;
    };
    let left_intro = transition_durations(project, &drag.outgoing).and_then(|(intro, _)| intro);
    let right_outro = transition_durations(project, &drag.incoming).and_then(|(_, outro)| outro);
    let maximum = crate::math::maximum_clip_transition_duration(
        left_end.saturating_sub(left_start),
        right_end.saturating_sub(right_start),
        left_intro,
        right_outro,
    );
    let duration = Time {
        seconds: distance.seconds * Fraction::from(2_u8),
    }
    .min(maximum);
    drag.target_duration = (duration > Time::ZERO).then_some(duration);
}

fn clip_transition_cut_range(
    drag: &ClipTransitionDrag,
    project: &Project,
    minimum_item_duration: Time,
) -> Option<(Time, Time)> {
    let duration = drag.original_duration?;
    let (left_start, _) = project.item(&drag.outgoing)?.times();
    let (_, right_end) = project.item(&drag.incoming)?.times();
    let left_intro = transition_durations(project, &drag.outgoing)?.0;
    let right_outro = transition_durations(project, &drag.incoming)?.1;
    let (previous_duration, next_duration) = surrounding_clip_transition_durations(drag, project)?;
    let left_minimum = crate::math::minimum_clip_transition_item_duration(duration, left_intro)
        .max(
            previous_duration
                .map(|duration| crate::math::minimum_clip_transition_item_duration(duration, None))
                .unwrap_or(minimum_item_duration),
        )
        .max(minimum_item_duration);
    let right_minimum = crate::math::minimum_clip_transition_item_duration(duration, right_outro)
        .max(
            next_duration
                .map(|duration| crate::math::minimum_clip_transition_item_duration(duration, None))
                .unwrap_or(minimum_item_duration),
        )
        .max(minimum_item_duration);
    let minimum = left_start.saturating_add(left_minimum);
    let maximum = right_end.saturating_sub(right_minimum);
    (minimum <= maximum).then_some((minimum, maximum))
}

fn surrounding_clip_transition_durations(
    drag: &ClipTransitionDrag,
    project: &Project,
) -> Option<(Option<Time>, Option<Time>)> {
    let track_address = drag.outgoing.track();
    match project.track(&track_address)? {
        crate::project::TrackRef::Video(track) => {
            let index = track
                .items
                .iter()
                .position(|item| item.id == drag.outgoing.item_id())?;
            let previous = index.checked_sub(1).and_then(|index| {
                track.items[index]
                    .transitions
                    .to_next
                    .as_ref()
                    .filter(|transition| transition.target_item_id == drag.outgoing.item_id())
                    .map(|transition| transition.duration)
            });
            let next = track
                .items
                .get(index + 1)?
                .transitions
                .to_next
                .as_ref()
                .filter(|transition| {
                    track
                        .items
                        .get(index + 2)
                        .is_some_and(|item| transition.target_item_id == item.id)
                })
                .map(|transition| transition.duration);
            Some((previous, next))
        }
        crate::project::TrackRef::Audio(track) => {
            let index = track
                .items
                .iter()
                .position(|item| item.id == drag.outgoing.item_id())?;
            let previous = index.checked_sub(1).and_then(|index| {
                track.items[index]
                    .transitions
                    .to_next
                    .as_ref()
                    .filter(|transition| transition.target_item_id == drag.outgoing.item_id())
                    .map(|transition| transition.duration)
            });
            let next = track
                .items
                .get(index + 1)?
                .transitions
                .to_next
                .as_ref()
                .filter(|transition| {
                    track
                        .items
                        .get(index + 2)
                        .is_some_and(|item| transition.target_item_id == item.id)
                })
                .map(|transition| transition.duration);
            Some((previous, next))
        }
        crate::project::TrackRef::Caption(_) => None,
    }
}

pub fn update_transition_drag(
    drag: &mut TransitionDrag,
    project: &Project,
    view: TimelineViewState,
    x: f64,
    snap_repository: &SnapRepo,
) {
    let Some(context) = SequenceTimeline::for_item(project, &drag.key) else {
        return;
    };
    let times = context.timeline_item_times(project, &drag.key);
    let Some((start, end)) = times else {
        return;
    };
    let start_x =
        timeline_x() + (start.as_secs_f64() - view.scroll_seconds) / view.seconds_per_pixel;
    let end_x = timeline_x() + (end.as_secs_f64() - view.scroll_seconds) / view.seconds_per_pixel;
    drag.remove = x <= start_x || x >= end_x;
    if drag.remove {
        return;
    }
    let target = crate::math::time_at_x(view, x);
    let target = snap_repository.snap(target).unwrap_or(target);
    let (intro, outro) = context
        .transition_durations(project, &drag.key)
        .unwrap_or_default();
    let Some((item_start, item_end)) = project.item(&drag.key).map(|item| item.times()) else {
        return;
    };
    let Some(target) = project
        .timeline_time_to_sequence(&drag.key.track(), target)
        .map(|target| target.snapped(project.frame_step()))
    else {
        return;
    };
    let item_duration = item_end.saturating_sub(item_start);
    let other = match drag.side {
        TransitionSide::Intro => outro,
        TransitionSide::Outro => intro,
    }
    .unwrap_or(Time::ZERO);
    let available = item_duration.saturating_sub(other);
    let duration = match drag.side {
        TransitionSide::Intro => target.saturating_sub(item_start),
        TransitionSide::Outro => item_end.saturating_sub(target),
    };
    drag.target_duration = duration.clamp(Time::ZERO, available);
    drag.target_timeline_duration = context
        .timeline_transition_duration(project, &drag.key, drag.side, drag.target_duration)
        .unwrap_or(Time::ZERO);
}

#[derive(Default)]
pub struct Gesture {
    pub clip: Option<ClipTransitionDrag>,
    pub item: Option<TransitionDrag>,
}

impl Gesture {
    pub fn begin(
        project: &Project,
        selection: &crate::selection_state::SharedSelectionState,
        view: TimelineViewState,
        point: glam::DVec2,
    ) -> Option<Self> {
        use crate::items::{ClipTransitionHitAction, TransitionHitAction};
        if let Some(hit) = crate::items::hit_clip_transition_at(project, view, point.x, point.y) {
            let context = SequenceTimeline::for_item(project, &hit.outgoing)?;
            if !crate::selection::select_item_in_context(
                &context,
                project,
                selection,
                hit.outgoing.clone(),
                false,
                false,
            ) {
                return Some(Self::default());
            }
            if hit.duration.is_some() {
                crate::selection_state::set_focused_transition(selection, TransitionSide::Outro);
            }
            return Some(Self {
                clip: (!matches!(hit.action, ClipTransitionHitAction::Body)).then_some(
                    ClipTransitionDrag {
                        outgoing: hit.outgoing,
                        incoming: hit.incoming,
                        cut: hit.cut,
                        target_cut: hit.cut,
                        original_duration: hit.duration,
                        target_duration: hit.duration,
                        center_resize: matches!(hit.action, ClipTransitionHitAction::CenterHandle),
                        handle: match hit.action {
                            ClipTransitionHitAction::StartHandle => Some(ItemEdge::Start),
                            ClipTransitionHitAction::EndHandle => Some(ItemEdge::End),
                            _ => None,
                        },
                    },
                ),
                item: None,
            });
        }
        let hit = crate::items::hit_transition_at(project, view, point.x, point.y)?;
        let context = SequenceTimeline::for_item(project, &hit.key)?;
        if !crate::selection::select_item_in_context(
            &context,
            project,
            selection,
            hit.key.clone(),
            false,
            false,
        ) {
            return Some(Self::default());
        }
        crate::selection_state::set_focused_transition(selection, hit.side);
        let (intro, outro) = context
            .transition_durations(project, &hit.key)
            .unwrap_or_default();
        let (timeline_intro, timeline_outro) =
            transition_durations(project, &hit.key).unwrap_or_default();
        Some(Self {
            clip: None,
            item: (!matches!(hit.action, TransitionHitAction::Body)).then_some(TransitionDrag {
                key: hit.key,
                side: hit.side,
                target_duration: match hit.side {
                    TransitionSide::Intro => intro,
                    TransitionSide::Outro => outro,
                }
                .unwrap_or(Time::ZERO),
                target_timeline_duration: match hit.side {
                    TransitionSide::Intro => timeline_intro,
                    TransitionSide::Outro => timeline_outro,
                }
                .unwrap_or(Time::ZERO),
                remove: false,
            }),
        })
    }

    pub fn update(
        &mut self,
        project: &Project,
        selection: &crate::selection_state::SharedSelectionState,
        view: TimelineViewState,
        x: f64,
        snaps: &SnapRepo,
    ) {
        if let Some(mut drag) = self.clip.take() {
            if drag.original_duration.is_none() && drag.handle.is_none() && view.drag_moved {
                let side = if x < view.drag_start_x {
                    TransitionSide::Outro
                } else {
                    TransitionSide::Intro
                };
                let key = if side == TransitionSide::Outro {
                    drag.outgoing
                } else {
                    drag.incoming
                };
                let context = SequenceTimeline::for_item(project, &key).expect("transition scope");
                crate::selection::select_item_in_context(
                    &context,
                    project,
                    selection,
                    key.clone(),
                    false,
                    false,
                );
                crate::selection_state::set_focused_transition(selection, side);
                self.item = Some(TransitionDrag {
                    key,
                    side,
                    target_duration: Time::ZERO,
                    target_timeline_duration: Time::ZERO,
                    remove: false,
                });
            } else {
                update_clip_transition_drag(&mut drag, project, view, x, snaps);
                self.clip = Some(drag);
            }
        }
        if let Some(drag) = &mut self.item {
            update_transition_drag(drag, project, view, x, snaps);
        }
    }
}

pub struct Applied {
    pub message: &'static str,
    pub focus: Option<TransitionSide>,
    pub kind: crate::project::ItemKind,
    pub rolling: bool,
}

impl Gesture {
    /// Apply the final GTK transition state to a candidate project; the host commits it.
    pub fn finish(self, project: &mut Project, moved: bool) -> Option<Applied> {
        if let Some(drag) = self.clip {
            let context = SequenceTimeline::for_item(project, &drag.outgoing)?;
            if drag.center_resize && drag.target_cut != drag.cut {
                return context
                    .apply_clip_transition_cut(
                        project,
                        &drag.outgoing,
                        &drag.incoming,
                        drag.target_cut,
                    )
                    .then_some(Applied {
                        message: "roll-clip-transition",
                        focus: Some(TransitionSide::Outro),
                        kind: drag.outgoing.kind(),
                        rolling: true,
                    });
            }
            let duration = if drag.original_duration.is_none() && !moved {
                let (left_start, left_end) = project.item(&drag.outgoing)?.times();
                let (right_start, right_end) = project.item(&drag.incoming)?.times();
                let left_intro =
                    transition_durations(project, &drag.outgoing).and_then(|(intro, _)| intro);
                let right_outro =
                    transition_durations(project, &drag.incoming).and_then(|(_, outro)| outro);
                let left_duration = left_end.saturating_sub(left_start);
                let right_duration = right_end.saturating_sub(right_start);
                let duration =
                    crate::math::default_clip_transition_duration(left_duration, right_duration)
                        .min(crate::math::maximum_clip_transition_duration(
                            left_duration,
                            right_duration,
                            left_intro,
                            right_outro,
                        ));
                (duration > Time::ZERO).then_some(duration)
            } else if drag.handle.is_some() && moved {
                drag.target_duration
            } else {
                drag.original_duration
            };
            return (duration != drag.original_duration
                && context.apply_clip_transition(
                    project,
                    &drag.outgoing,
                    &drag.incoming,
                    duration,
                ))
            .then_some(Applied {
                message: "edit-clip-transition",
                focus: duration.map(|_| TransitionSide::Outro),
                kind: drag.outgoing.kind(),
                rolling: false,
            });
        }
        let drag = self
            .item
            .filter(|drag| moved && (drag.remove || drag.target_duration > Time::ZERO))?;
        let context = SequenceTimeline::for_item(project, &drag.key)?;
        let duration = (!drag.remove).then_some(drag.target_duration);
        context
            .apply_transition(project, &drag.key, drag.side, duration)
            .then_some(Applied {
                message: "edit-item-transition",
                focus: duration.map(|_| drag.side),
                kind: drag.key.kind(),
                rolling: false,
            })
    }
}
