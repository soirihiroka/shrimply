use super::*;
use crate::timeline_operation::{SequenceTimeline, TimelineOperationContext};

pub(crate) fn item_group_id(project: &Project, key: ItemKey) -> Option<u64> {
    let address = crate::selection_state::item_address(project, key)?;
    item_address_group_id(project, &address)
}

pub(crate) fn item_address_group_id(project: &Project, address: &ItemAddress) -> Option<u64> {
    match project.item(address)? {
        ItemRef::Caption(item) => item.group_id,
        ItemRef::Video(item) => item.group_id,
        ItemRef::Audio(item) => item.group_id,
    }
}

pub(crate) fn set_item_address_group_id(
    project: &mut Project,
    address: &ItemAddress,
    group_id: Option<u64>,
) -> bool {
    match project.item_mut(address) {
        Some(ItemMut::Caption(item)) => item.group_id = group_id,
        Some(ItemMut::Video(item)) => item.group_id = group_id,
        Some(ItemMut::Audio(item)) => item.group_id = group_id,
        None => return false,
    }
    true
}

pub(crate) fn group_item_addresses(
    context: &impl TimelineOperationContext,
    project: &mut Project,
    selected_items: &[ItemAddress],
) -> Option<Vec<ItemAddress>> {
    assert!(
        selected_items
            .iter()
            .all(|address| context.contains_item(project, address)),
        "grouped items must belong to their operation context"
    );
    if selected_items.len() < 2 {
        return None;
    }
    let mut seen = HashSet::new();
    let addresses = selected_items
        .iter()
        .filter(|address| project.item(address).is_some())
        .filter(|address| seen.insert((*address).clone()))
        .cloned()
        .collect::<Vec<_>>();
    if addresses.len() < 2 {
        return None;
    }

    let group_id = Some(next_group_id(project));
    for address in &addresses {
        set_item_address_group_id(project, address, group_id);
    }
    Some(addresses)
}

pub(crate) fn ungroup_item_addresses(
    context: &impl TimelineOperationContext,
    project: &mut Project,
    selected_items: &[ItemAddress],
) -> Option<Vec<ItemAddress>> {
    let addresses = expand_grouped_item_addresses(context, project, selected_items);
    let mut changed = false;
    for address in &addresses {
        if item_address_group_id(project, address).is_some() {
            changed |= set_item_address_group_id(project, address, None);
        }
    }
    changed.then_some(addresses)
}

pub(crate) fn expand_grouped_item_addresses(
    context: &dyn TimelineOperationContext,
    project: &Project,
    selected_items: &[ItemAddress],
) -> Vec<ItemAddress> {
    assert!(
        selected_items
            .iter()
            .all(|address| context.contains_item(project, address)),
        "grouped items must belong to their operation context"
    );

    let scope = context.items(project);
    let mut expanded = Vec::new();
    let mut seen = HashSet::new();
    for address in selected_items {
        if let Some(group_id) = item_address_group_id(project, address) {
            for member in &scope {
                if item_address_group_id(project, member) == Some(group_id)
                    && seen.insert(member.clone())
                {
                    expanded.push(member.clone());
                }
            }
        } else if project.item(address).is_some() && seen.insert(address.clone()) {
            expanded.push(address.clone());
        }
    }
    expanded
}

pub(crate) fn fold_items(
    project: &mut Project,
    selected_items: &[ItemKey],
) -> Option<Vec<ItemKey>> {
    let mut keys = selected_items
        .iter()
        .copied()
        .filter(|key| matches!(key.kind, TrackKind::Video | TrackKind::Audio))
        .filter(|key| item_times(project, *key).is_some())
        .collect::<Vec<_>>();
    keys.sort_by_key(item_key_sort_key);
    keys.dedup();
    if keys.len() < 2 || keys.len() != selected_items.len() {
        return None;
    }
    let origin = keys
        .iter()
        .filter_map(|key| item_times(project, *key).map(|times| times.0))
        .min()?;
    let end = keys
        .iter()
        .filter_map(|key| item_times(project, *key).map(|times| times.1))
        .max()?;
    let duration = end.saturating_sub(origin);
    if duration <= Time::ZERO {
        return None;
    }

    let sequence_id = Uuid::new_v4();
    let instance_id = Uuid::new_v4();
    let reference = SequenceReference {
        sequence_id,
        instance_id,
    };
    let group_id = (keys.iter().any(|key| key.kind == TrackKind::Video)
        && keys.iter().any(|key| key.kind == TrackKind::Audio))
    .then(|| next_group_id(project));

    let video_source_tracks = keys
        .iter()
        .filter(|key| key.kind == TrackKind::Video)
        .map(|key| key.track_index)
        .collect::<HashSet<_>>();
    let audio_source_tracks = keys
        .iter()
        .filter(|key| key.kind == TrackKind::Audio)
        .map(|key| key.track_index)
        .collect::<HashSet<_>>();
    let mut sequence_video_tracks = Vec::new();
    let mut sequence_audio_tracks = Vec::new();
    let mut visual_proxy = None;
    let mut audio_proxy = None;

    for track_index in 0..project.video_tracks.len() {
        if !video_source_tracks.contains(&track_index) {
            continue;
        }
        let indices = keys
            .iter()
            .filter(|key| key.kind == TrackKind::Video && key.track_index == track_index)
            .map(|key| key.item_index)
            .collect::<HashSet<_>>();
        let mut nested = project.video_tracks[track_index].clone();
        nested.id = Uuid::new_v4();
        nested.items = nested
            .items
            .into_iter()
            .enumerate()
            .filter_map(|(index, mut item)| {
                indices.contains(&index).then(|| {
                    item.start = item.start.saturating_sub(origin);
                    item.end = item.end.saturating_sub(origin);
                    item
                })
            })
            .collect();
        if visual_proxy.is_none() {
            visual_proxy = nested.items.first().cloned();
        }
        sequence_video_tracks.push(nested);
    }
    for track_index in 0..project.audio_tracks.len() {
        if !audio_source_tracks.contains(&track_index) {
            continue;
        }
        let indices = keys
            .iter()
            .filter(|key| key.kind == TrackKind::Audio && key.track_index == track_index)
            .map(|key| key.item_index)
            .collect::<HashSet<_>>();
        let mut nested = project.audio_tracks[track_index].clone();
        nested.id = Uuid::new_v4();
        nested.items = nested
            .items
            .into_iter()
            .enumerate()
            .filter_map(|(index, mut item)| {
                indices.contains(&index).then(|| {
                    item.start = item.start.saturating_sub(origin);
                    item.end = item.end.saturating_sub(origin);
                    item
                })
            })
            .collect();
        if audio_proxy.is_none() {
            audio_proxy = nested.items.first().cloned();
        }
        sequence_audio_tracks.push(nested);
    }

    for track_index in (0..project.video_tracks.len()).rev() {
        project.video_tracks[track_index].items.retain(|item| {
            !sequence_video_tracks
                .iter()
                .any(|track| track.items.iter().any(|nested| nested.id == item.id))
        });
    }
    for track_index in (0..project.audio_tracks.len()).rev() {
        project.audio_tracks[track_index].items.retain(|item| {
            !sequence_audio_tracks
                .iter()
                .any(|track| track.items.iter().any(|nested| nested.id == item.id))
        });
    }

    let video_requires_enabled_track = sequence_video_tracks.iter().any(|track| track.enabled);
    let audio_requires_enabled_track = sequence_audio_tracks.iter().any(|track| track.enabled);
    project.folded_sequences.push(FoldedSequence {
        id: sequence_id,
        video_tracks: sequence_video_tracks,
        audio_tracks: sequence_audio_tracks,
    });

    let mut selection = Vec::new();
    if let Some(mut item) = visual_proxy {
        item.id = Uuid::new_v4();
        item.start = origin;
        item.end = end;
        item.time_offset = Time::ZERO;
        item.source_duration = duration;
        item.playback_speed = default_playback_speed();
        item.playback_fps = crate::project::native_playback_fps();
        item.repeat_strategy = RepeatStrategy::Empty;
        item.stabilize_video = false;
        item.animation_time_offset = Time::ZERO;
        item.transform = Transform::fill(project.canvas_size);
        item.modifiers.clear();
        item.sample_method = Default::default();
        item.skia_drawing_strategy = Default::default();
        item.compositing = Default::default();
        item.visibility = shrimply_core::timeline_value::TimelineValue::new_const(
            shrimply_core::timeline_value::TimelineBool::True,
        );
        item.alpha_mask_video = None;
        item.transitions = Default::default();
        item.svg_color_overrides.clear();
        item.source_width = project.canvas_size.width;
        item.source_height = project.canvas_size.height;
        item.default_transform = None;
        item.render_canvas_size = None;
        item.group_id = group_id;
        item.content = VideoItemContent::FoldedSequence(reference);
        item.file = Default::default();
        item.track_id = 0;
        let preferred_index = video_source_tracks.iter().copied().min().unwrap_or(0);
        let mut indices = video_source_tracks.iter().copied().collect::<Vec<_>>();
        indices.sort_unstable();
        let target = indices.into_iter().find(|index| {
            project.video_tracks.get(*index).is_some_and(|track| {
                (!video_requires_enabled_track || track.enabled)
                    && !timeline_search::collides(&track.items, origin, end)
            })
        });
        let (index, item_index) = if let Some(index) = target {
            let item_index = insert_sorted(&mut project.video_tracks[index].items, item);
            (index, item_index)
        } else {
            project.video_tracks.insert(
                preferred_index,
                VisualTrack {
                    id: Uuid::new_v4(),
                    enabled: true,
                    items: vec![item],
                },
            );
            (preferred_index, 0)
        };
        selection.push(ItemKey {
            kind: TrackKind::Video,
            track_index: index,
            item_index,
        });
    }
    if let Some(mut item) = audio_proxy {
        item.id = Uuid::new_v4();
        item.start = origin;
        item.end = end;
        item.time_offset = Time::ZERO;
        item.source_duration = duration;
        item.playback_speed = default_playback_speed();
        item.repeat_strategy = RepeatStrategy::Empty;
        item.speed_method = Default::default();
        item.modifiers.clear();
        item.transitions = Default::default();
        item.beat_detection = false;
        item.group_id = group_id;
        item.source = AudioSource::FoldedSequence(reference);
        item.file = Default::default();
        item.track_id = 0;
        let preferred_index = audio_source_tracks.iter().copied().min().unwrap_or(0);
        let mut indices = audio_source_tracks.iter().copied().collect::<Vec<_>>();
        indices.sort_unstable();
        let target = indices.into_iter().find(|index| {
            project.audio_tracks.get(*index).is_some_and(|track| {
                (!audio_requires_enabled_track || track.enabled)
                    && track.gain_db == 0.0
                    && !timeline_search::collides(&track.items, origin, end)
            })
        });
        let (index, item_index) = if let Some(index) = target {
            let item_index = insert_sorted(&mut project.audio_tracks[index].items, item);
            (index, item_index)
        } else {
            project.audio_tracks.insert(
                preferred_index,
                AudioTrack {
                    id: Uuid::new_v4(),
                    enabled: true,
                    gain_db: 0.0,
                    items: vec![item],
                },
            );
            (preferred_index, 0)
        };
        selection.push(ItemKey {
            kind: TrackKind::Audio,
            track_index: index,
            item_index,
        });
    }
    Some(selection)
}

pub(crate) fn expand_grouped_selection(project: &Project, selection: &[ItemKey]) -> Vec<ItemKey> {
    let addresses = selection
        .iter()
        .filter_map(|key| crate::selection_state::item_address(project, *key))
        .collect::<Vec<_>>();
    expand_grouped_item_addresses(&SequenceTimeline::root(), project, &addresses)
        .iter()
        .filter_map(|address| crate::selection_state::item_key(project, address))
        .collect()
}

pub(crate) fn split_item_addresses(
    context: &impl TimelineOperationContext,
    project: &mut Project,
    addresses: &[ItemAddress],
    cut: Time,
) -> (Vec<ItemAddress>, Vec<ItemAddress>) {
    let mut addresses = expand_grouped_item_addresses(context, project, addresses);
    addresses.sort_by_key(|address| {
        (
            match address.kind() {
                ItemKind::Caption => 0,
                ItemKind::Video => 1,
                ItemKind::Audio => 2,
            },
            address.track_id(),
            address.item_id(),
        )
    });
    addresses.dedup();

    let mut groups: Vec<(Option<u64>, Vec<ItemAddress>)> = Vec::new();
    for address in addresses {
        let group_id = item_address_group_id(project, &address);
        if let Some(group_id) = group_id
            && let Some((_, members)) = groups
                .iter_mut()
                .find(|(existing, _)| *existing == Some(group_id))
        {
            members.push(address);
        } else {
            groups.push((group_id, vec![address]));
        }
    }

    let mut left = Vec::new();
    let mut right = Vec::new();
    for (group_id, members) in groups {
        let split_group_ids = group_id.map(|_| {
            let left = next_group_id(project);
            (left, left.saturating_add(1))
        });
        for address in members {
            let Some((start, end)) = context.timeline_item_times(project, &address) else {
                continue;
            };
            if start < cut && cut < end {
                let Some((left_address, right_address)) =
                    crate::items::split_item_address(project, &address, cut)
                else {
                    continue;
                };
                if let Some((left_group_id, right_group_id)) = split_group_ids {
                    set_item_address_group_id(project, &left_address, Some(left_group_id));
                    set_item_address_group_id(project, &right_address, Some(right_group_id));
                }
                left.push(left_address);
                right.push(right_address);
            } else if end <= cut {
                if let Some((left_group_id, _)) = split_group_ids {
                    set_item_address_group_id(project, &address, Some(left_group_id));
                }
                left.push(address);
            } else {
                if let Some((_, right_group_id)) = split_group_ids {
                    set_item_address_group_id(project, &address, Some(right_group_id));
                }
                right.push(address);
            }
        }
    }
    (left, right)
}
