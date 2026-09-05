use crate::items::{fit_audio_transitions, fit_visual_transitions, shifted_media_source_offset};
use crate::project::{
    AudioClipTransition, AudioClipTransitionCurve, AudioTransition, ItemAddress, ItemKind, ItemMut,
    ItemRef, Project, SequenceScopeId, Time, TrackAddress, TrackMut, TransitionSide,
    VideoItemContent, VisualClipTransition, VisualTransition,
};

pub trait TimelineOperationContext {
    fn scope(&self) -> &SequenceScopeId;

    fn contains_track(&self, project: &Project, address: &TrackAddress) -> bool {
        project
            .track_scope(address)
            .is_some_and(|scope| &scope == self.scope())
    }

    fn contains_item(&self, project: &Project, address: &ItemAddress) -> bool {
        project
            .item_scope(address)
            .is_some_and(|scope| &scope == self.scope())
    }

    fn tracks(&self, project: &Project) -> Vec<TrackAddress> {
        let mut addresses = Vec::new();
        if self.scope().is_root() {
            addresses.extend(
                project
                    .caption_tracks
                    .iter()
                    .map(|track| TrackAddress::Caption { track_id: track.id }),
            );
        }
        if let (Some(path), Some(tracks)) = (
            project.sequence_path_for_scope(ItemKind::Video, self.scope()),
            project.video_tracks_for_scope(self.scope()),
        ) {
            addresses.extend(tracks.iter().map(|track| TrackAddress::Video {
                sequence_path: path.clone(),
                track_id: track.id,
            }));
        }
        if let (Some(path), Some(tracks)) = (
            project.sequence_path_for_scope(ItemKind::Audio, self.scope()),
            project.audio_tracks_for_scope(self.scope()),
        ) {
            addresses.extend(tracks.iter().map(|track| TrackAddress::Audio {
                sequence_path: path.clone(),
                track_id: track.id,
            }));
        }
        addresses
    }

    fn items(&self, project: &Project) -> Vec<ItemAddress> {
        self.tracks(project)
            .into_iter()
            .flat_map(|track| {
                let item_ids = match project.track(&track) {
                    Some(crate::project::TrackRef::Caption(track)) => {
                        track.items.iter().map(|item| item.id).collect()
                    }
                    Some(crate::project::TrackRef::Video(track)) => {
                        track.items.iter().map(|item| item.id).collect()
                    }
                    Some(crate::project::TrackRef::Audio(track)) => {
                        track.items.iter().map(|item| item.id).collect()
                    }
                    None => Vec::new(),
                };
                item_ids.into_iter().map(move |item_id| track.item(item_id))
            })
            .collect()
    }

    fn timeline_item_times(
        &self,
        project: &Project,
        address: &ItemAddress,
    ) -> Option<(Time, Time)> {
        self.contains_item(project, address)
            .then(|| project.timeline_item_times(address))?
    }

    fn transition_durations(
        &self,
        project: &Project,
        address: &ItemAddress,
    ) -> Option<(Option<Time>, Option<Time>)> {
        if !self.contains_item(project, address) {
            return None;
        }
        match project.item(address)? {
            ItemRef::Caption(_) => None,
            ItemRef::Video(item) => {
                if matches!(
                    item.content,
                    VideoItemContent::Obj(_) | VideoItemContent::Gaussian(_)
                ) {
                    return None;
                }
                Some((
                    item.transitions.intro.as_ref().map(|value| value.duration),
                    item.transitions.outro.as_ref().map(|value| value.duration),
                ))
            }
            ItemRef::Audio(item) => Some((
                item.transitions.intro.as_ref().map(|value| value.duration),
                item.transitions.outro.as_ref().map(|value| value.duration),
            )),
        }
    }

    fn timeline_transition_durations(
        &self,
        project: &Project,
        address: &ItemAddress,
    ) -> Option<(Option<Time>, Option<Time>)> {
        let (intro, outro) = self.transition_durations(project, address)?;
        Some((
            intro.and_then(|duration| {
                self.timeline_transition_duration(project, address, TransitionSide::Intro, duration)
            }),
            outro.and_then(|duration| {
                self.timeline_transition_duration(project, address, TransitionSide::Outro, duration)
            }),
        ))
    }

    fn timeline_transition_duration(
        &self,
        project: &Project,
        address: &ItemAddress,
        side: TransitionSide,
        duration: Time,
    ) -> Option<Time> {
        if !self.contains_item(project, address) {
            return None;
        }
        let (start, end) = project.item(address)?.times();
        let (first, second) = match side {
            TransitionSide::Intro => (start, start.saturating_add(duration)),
            TransitionSide::Outro => (end.saturating_sub(duration), end),
        };
        let track = address.track();
        let first = project.sequence_time_to_timeline(&track, first)?;
        let second = project.sequence_time_to_timeline(&track, second)?;
        Some(first.max(second).saturating_sub(first.min(second)))
    }

    fn timeline_clip_transition_duration(
        &self,
        project: &Project,
        address: &ItemAddress,
        duration: Time,
    ) -> Option<Time> {
        if !self.contains_item(project, address) {
            return None;
        }
        let cut = project.item(address)?.times().1;
        let half = crate::math::clip_transition_half_duration(duration);
        let track = address.track();
        let first = project.sequence_time_to_timeline(&track, cut.saturating_sub(half))?;
        let second = project.sequence_time_to_timeline(&track, cut.saturating_add(half))?;
        Some(first.max(second).saturating_sub(first.min(second)))
    }

    fn apply_transition(
        &self,
        project: &mut Project,
        address: &ItemAddress,
        side: TransitionSide,
        duration: Option<Time>,
    ) -> bool {
        if !self.contains_item(project, address) {
            return false;
        }
        let canvas_size = project.canvas_size;
        match project.item_mut(address) {
            Some(ItemMut::Video(item)) => {
                let slot = match side {
                    TransitionSide::Intro => &mut item.transitions.intro,
                    TransitionSide::Outro => &mut item.transitions.outro,
                };
                match duration {
                    None => *slot = None,
                    Some(duration) if slot.is_some() => {
                        slot.as_mut().expect("checked transition slot").duration = duration;
                    }
                    Some(duration) => {
                        *slot = Some(VisualTransition::new(side, duration, canvas_size));
                    }
                }
                true
            }
            Some(ItemMut::Audio(item)) => {
                let slot = match side {
                    TransitionSide::Intro => &mut item.transitions.intro,
                    TransitionSide::Outro => &mut item.transitions.outro,
                };
                match duration {
                    None => *slot = None,
                    Some(duration) if slot.is_some() => {
                        slot.as_mut().expect("checked transition slot").duration = duration;
                    }
                    Some(duration) => *slot = Some(AudioTransition::new(side, duration)),
                }
                true
            }
            Some(ItemMut::Caption(_)) | None => false,
        }
    }

    fn apply_clip_transition(
        &self,
        project: &mut Project,
        outgoing: &ItemAddress,
        incoming: &ItemAddress,
        duration: Option<Time>,
    ) -> bool {
        if !self.contains_item(project, outgoing)
            || !self.contains_item(project, incoming)
            || outgoing.track() != incoming.track()
        {
            return false;
        }
        let track = outgoing.track();
        let outgoing_id = outgoing.item_id();
        let incoming_id = incoming.item_id();
        match project.track_mut(&track) {
            Some(TrackMut::Video(track)) => {
                let Some(index) = track.items.iter().position(|item| item.id == outgoing_id) else {
                    return false;
                };
                let Some(incoming_item) = track.items.get(index + 1) else {
                    return false;
                };
                if incoming_item.id != incoming_id || track.items[index].end != incoming_item.start
                {
                    return false;
                }
                match duration {
                    None => track.items[index].transitions.to_next = None,
                    Some(duration) => {
                        if let Some(transition) = track.items[index].transitions.to_next.as_mut() {
                            transition.target_item_id = incoming_id;
                            transition.duration = duration;
                        } else {
                            track.items[index].transitions.to_next =
                                Some(VisualClipTransition::new(incoming_id, duration));
                        }
                    }
                }
                true
            }
            Some(TrackMut::Audio(track)) => {
                let Some(index) = track.items.iter().position(|item| item.id == outgoing_id) else {
                    return false;
                };
                let Some(incoming_item) = track.items.get(index + 1) else {
                    return false;
                };
                if incoming_item.id != incoming_id || track.items[index].end != incoming_item.start
                {
                    return false;
                }
                match duration {
                    None => track.items[index].transitions.to_next = None,
                    Some(duration) => {
                        let curve = track.items[index]
                            .transitions
                            .to_next
                            .as_ref()
                            .map_or(AudioClipTransitionCurve::EqualPower, |value| value.curve);
                        track.items[index].transitions.to_next =
                            Some(Box::new(AudioClipTransition {
                                target_item_id: incoming_id,
                                duration,
                                curve,
                            }));
                    }
                }
                true
            }
            Some(TrackMut::Caption(_)) | None => false,
        }
    }

    fn apply_clip_transition_cut(
        &self,
        project: &mut Project,
        outgoing: &ItemAddress,
        incoming: &ItemAddress,
        cut: Time,
    ) -> bool {
        if !self.contains_item(project, outgoing)
            || !self.contains_item(project, incoming)
            || outgoing.track() != incoming.track()
        {
            return false;
        }
        let track_address = outgoing.track();
        let outgoing_id = outgoing.item_id();
        let incoming_id = incoming.item_id();
        match project.track_mut(&track_address) {
            Some(TrackMut::Video(track)) => {
                let Some(index) = track.items.iter().position(|item| item.id == outgoing_id) else {
                    return false;
                };
                let (left, right) = track.items.split_at_mut(index + 1);
                let outgoing = &mut left[index];
                let Some(incoming) = right.first_mut() else {
                    return false;
                };
                if incoming.id != incoming_id
                    || outgoing.end != incoming.start
                    || outgoing
                        .transitions
                        .to_next
                        .as_ref()
                        .is_none_or(|transition| transition.target_item_id != incoming_id)
                    || cut <= outgoing.start
                    || cut >= incoming.end
                {
                    return false;
                }
                let animation_time_offset = Time {
                    seconds: incoming.animation_time_offset.seconds + cut.seconds
                        - incoming.start.seconds,
                };
                incoming.time_offset = shifted_media_source_offset(
                    incoming.time_offset,
                    incoming.start,
                    cut,
                    incoming.playback_speed,
                    incoming.repeat_strategy,
                    incoming.source_duration,
                );
                incoming.animation_time_offset = animation_time_offset;
                outgoing.end = cut;
                incoming.start = cut;
                fit_visual_transitions(outgoing);
                fit_visual_transitions(incoming);
                true
            }
            Some(TrackMut::Audio(track)) => {
                let Some(index) = track.items.iter().position(|item| item.id == outgoing_id) else {
                    return false;
                };
                let (left, right) = track.items.split_at_mut(index + 1);
                let outgoing = &mut left[index];
                let Some(incoming) = right.first_mut() else {
                    return false;
                };
                if incoming.id != incoming_id
                    || outgoing.end != incoming.start
                    || outgoing
                        .transitions
                        .to_next
                        .as_ref()
                        .is_none_or(|transition| transition.target_item_id != incoming_id)
                    || cut <= outgoing.start
                    || cut >= incoming.end
                {
                    return false;
                }
                incoming.time_offset = shifted_media_source_offset(
                    incoming.time_offset,
                    incoming.start,
                    cut,
                    incoming.playback_speed,
                    incoming.repeat_strategy,
                    incoming.source_duration,
                );
                outgoing.end = cut;
                incoming.start = cut;
                fit_audio_transitions(outgoing);
                fit_audio_transitions(incoming);
                true
            }
            Some(TrackMut::Caption(_)) | None => false,
        }
    }

    fn allows_track_drop(
        &self,
        project: &Project,
        source: &ItemAddress,
        target: &TrackAddress,
    ) -> bool {
        self.contains_item(project, source)
            && source.kind() == target.kind()
            && project.track(target).is_some()
            && project.can_move_item_to_sequence_path(source, target.sequence_path())
    }

    fn allows_new_track_drop(
        &self,
        project: &Project,
        source: &ItemAddress,
        sequence_path: &[uuid::Uuid],
    ) -> bool {
        self.contains_item(project, source)
            && project
                .sequence_scope_for_path(source.kind(), sequence_path)
                .is_some()
            && !sequence_path.contains(&source.item_id())
            && project.can_move_item_to_sequence_path(source, sequence_path)
    }

    fn move_item_out(&self, project: &mut Project, address: &ItemAddress) -> Option<ItemAddress> {
        if !self.contains_item(project, address) {
            return None;
        }
        let parent_scope = self.scope().parent()?;
        let parent_path = project.sequence_path_for_scope(address.kind(), &parent_scope)?;
        let (timeline_start, timeline_end) = project.timeline_item_times(address)?;
        let first = project
            .timeline_time_to_sequence_path(address.kind(), &parent_path, timeline_start)?
            .snapped(project.frame_step());
        let second = project
            .timeline_time_to_sequence_path(address.kind(), &parent_path, timeline_end)?
            .snapped(project.frame_step());
        project.move_item_to_new_top_track(
            address,
            &parent_path,
            first.min(second),
            first.max(second),
        )
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SequenceTimeline {
    scope: SequenceScopeId,
}

impl SequenceTimeline {
    pub fn root() -> Self {
        Self {
            scope: SequenceScopeId::root(),
        }
    }

    pub fn new(scope: SequenceScopeId) -> Self {
        Self { scope }
    }

    pub fn for_item(project: &Project, address: &ItemAddress) -> Option<Self> {
        project.item_scope(address).map(Self::new)
    }

    pub fn for_track(project: &Project, address: &TrackAddress) -> Option<Self> {
        project.track_scope(address).map(Self::new)
    }
}

impl TimelineOperationContext for SequenceTimeline {
    fn scope(&self) -> &SequenceScopeId {
        &self.scope
    }
}
