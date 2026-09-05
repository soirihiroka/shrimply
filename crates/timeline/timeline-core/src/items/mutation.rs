use super::*;

/// Waveform inputs exclude placement and grouping; moving a clip preserves its cache.
pub fn audio_waveform_cache_signature(project: &Project) -> Vec<(Uuid, String)> {
    let mut signature = project
        .audio_tracks
        .iter()
        .flat_map(|track| &track.items)
        .chain(
            project
                .folded_sequences
                .iter()
                .flat_map(|sequence| &sequence.audio_tracks)
                .flat_map(|track| &track.items),
        )
        .map(|item| {
            (
                item.id,
                serde_json::to_string(&(
                    item.end.saturating_sub(item.start),
                    item.time_offset,
                    item.source_duration,
                    item.playback_speed.to_string(),
                    item.repeat_strategy,
                    item.speed_method,
                    &item.source,
                    &item.file,
                    item.track_id,
                    &item.gain,
                    &item.modifiers,
                ))
                .expect("waveform input signature must serialize"),
            )
        })
        .collect::<Vec<_>>();
    signature.sort_unstable_by_key(|(id, _)| *id);
    signature
}

pub trait OverwriteItem: Clone + TimeSlice {
    fn trim_start(&mut self, start: Time);
    fn set_end(&mut self, end: Time);
    fn reset_id(&mut self);
    fn group_id(&self) -> Option<u64>;
    fn set_group_id(&mut self, group_id: Option<u64>);
}

impl OverwriteItem for CaptionItem {
    fn trim_start(&mut self, start: Time) {
        self.start = start;
    }

    fn set_end(&mut self, end: Time) {
        self.end = end;
    }

    fn reset_id(&mut self) {
        self.id = Uuid::new_v4();
    }

    fn group_id(&self) -> Option<u64> {
        self.group_id
    }

    fn set_group_id(&mut self, group_id: Option<u64>) {
        self.group_id = group_id;
    }
}

impl OverwriteItem for VideoItem {
    fn trim_start(&mut self, start: Time) {
        self.transitions.intro = None;
        if self.repeats_keyframes() {
            self.start = start;
            fit_visual_transitions(self);
            return;
        }
        let animation_delta = start.signed_sub(self.start);
        self.time_offset = shifted_media_source_offset(
            self.time_offset,
            self.start,
            start,
            self.playback_speed,
            self.repeat_strategy,
            self.source_duration,
        );
        self.animation_time_offset = Time {
            seconds: self.animation_time_offset.seconds + animation_delta.seconds,
        };
        self.start = start;
        fit_visual_transitions(self);
    }

    fn set_end(&mut self, end: Time) {
        self.end = end;
        self.transitions.outro = None;
        self.transitions.to_next = None;
        fit_visual_transitions(self);
    }

    fn reset_id(&mut self) {
        self.id = Uuid::new_v4();
        Project::regenerate_video_property_ids(self);
    }

    fn group_id(&self) -> Option<u64> {
        self.group_id
    }

    fn set_group_id(&mut self, group_id: Option<u64>) {
        self.group_id = group_id;
    }
}

impl OverwriteItem for AudioItem {
    fn trim_start(&mut self, start: Time) {
        self.transitions.intro = None;
        self.time_offset = shifted_media_source_offset(
            self.time_offset,
            self.start,
            start,
            self.playback_speed,
            self.repeat_strategy,
            self.source_duration,
        );
        self.start = start;
        fit_audio_transitions(self);
    }

    fn set_end(&mut self, end: Time) {
        self.end = end;
        self.transitions.outro = None;
        self.transitions.to_next = None;
        fit_audio_transitions(self);
    }

    fn reset_id(&mut self) {
        self.id = Uuid::new_v4();
        Project::regenerate_audio_property_ids(self);
    }

    fn group_id(&self) -> Option<u64> {
        self.group_id
    }

    fn set_group_id(&mut self, group_id: Option<u64>) {
        self.group_id = group_id;
    }
}

pub fn overwrite_caption_items(
    project: &mut Project,
    placements: &[ItemPlacement],
    new_track_indices: &[usize],
) -> Option<()> {
    for placement in placements
        .iter()
        .copied()
        .filter(|placement| placement.key.kind == TrackKind::Caption)
    {
        let Some(track_index) = target_existing_track_index(new_track_indices, placement) else {
            continue;
        };
        let items = &mut project.caption_tracks.get_mut(track_index)?.items;
        overwrite_items(items, placement.start, placement.end);
    }
    Some(())
}

pub fn overwrite_video_items(
    project: &mut Project,
    placements: &[ItemPlacement],
    new_track_indices: &[usize],
) -> Option<()> {
    for placement in placements
        .iter()
        .copied()
        .filter(|placement| placement.key.kind == TrackKind::Video)
    {
        let Some(track_index) = target_existing_track_index(new_track_indices, placement) else {
            continue;
        };
        let items = &mut project.video_tracks.get_mut(track_index)?.items;
        overwrite_items(items, placement.start, placement.end);
    }
    Some(())
}

pub fn overwrite_audio_items(
    project: &mut Project,
    placements: &[ItemPlacement],
    new_track_indices: &[usize],
) -> Option<()> {
    for placement in placements
        .iter()
        .copied()
        .filter(|placement| placement.key.kind == TrackKind::Audio)
    {
        let Some(track_index) = target_existing_track_index(new_track_indices, placement) else {
            continue;
        };
        let items = &mut project.audio_tracks.get_mut(track_index)?.items;
        overwrite_items(items, placement.start, placement.end);
    }
    Some(())
}

pub fn overwrite_items<T: OverwriteItem>(items: &mut Vec<T>, start: Time, end: Time) {
    let mut index = 0;
    while index < items.len() {
        let item_start = items[index].start();
        let item_end = items[index].end();
        if !time_ranges_collide(start, end, item_start, item_end) {
            index += 1;
            continue;
        }

        if start <= item_start && end >= item_end {
            items.remove(index);
        } else if item_start < start && end < item_end {
            let mut right = items[index].clone();
            right.reset_id();
            right.trim_start(end);
            items[index].set_end(start);
            items.insert(index + 1, right);
            index += 2;
        } else if item_start < start {
            items[index].set_end(start);
            index += 1;
        } else {
            items[index].trim_start(end);
            index += 1;
        }
    }
}

pub fn remove_caption_items(project: &mut Project, placements: &[ItemPlacement]) -> Option<()> {
    let mut keys: Vec<_> = placements
        .iter()
        .map(|placement| placement.key)
        .filter(|key| key.kind == TrackKind::Caption)
        .collect();
    keys.sort_by_key(|key| (key.track_index, key.item_index));
    for key in keys.into_iter().rev() {
        project
            .caption_tracks
            .get_mut(key.track_index)?
            .items
            .get(key.item_index)?;
        project
            .caption_tracks
            .get_mut(key.track_index)?
            .items
            .remove(key.item_index);
    }
    Some(())
}

pub fn remove_video_items(project: &mut Project, placements: &[ItemPlacement]) -> Option<()> {
    let mut keys: Vec<_> = placements
        .iter()
        .map(|placement| placement.key)
        .filter(|key| key.kind == TrackKind::Video)
        .collect();
    keys.sort_by_key(|key| (key.track_index, key.item_index));
    for key in keys.into_iter().rev() {
        project
            .video_tracks
            .get_mut(key.track_index)?
            .items
            .get(key.item_index)?;
        project
            .video_tracks
            .get_mut(key.track_index)?
            .items
            .remove(key.item_index);
    }
    Some(())
}

pub fn remove_audio_items(project: &mut Project, placements: &[ItemPlacement]) -> Option<()> {
    let mut keys: Vec<_> = placements
        .iter()
        .map(|placement| placement.key)
        .filter(|key| key.kind == TrackKind::Audio)
        .collect();
    keys.sort_by_key(|key| (key.track_index, key.item_index));
    for key in keys.into_iter().rev() {
        project
            .audio_tracks
            .get_mut(key.track_index)?
            .items
            .get(key.item_index)?;
        project
            .audio_tracks
            .get_mut(key.track_index)?
            .items
            .remove(key.item_index);
    }
    Some(())
}
