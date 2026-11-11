use shrimply_math_core::fit_nonnegative_fraction_pair;
use shrimply_project_document::project::{
    AudioItem, AudioTrack, CaptionTrack, CommentTrack, ItemAddress, ItemKind, ItemMut, ItemRef,
    Project, ProjectItem, RepeatStrategy, SequenceScopeId, Time, TrackAddress, TrackMut, VideoItem,
    VisualTrack, scaled_time_delta,
};
use shrimply_property_model::timeline_value::{TimelineBase, TimelineValue};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CollisionBehavior {
    #[default]
    Reject,
    NewTrack,
    Overwrite,
}

pub fn create_track(
    project: &mut Project,
    scope: &SequenceScopeId,
    kind: ItemKind,
    enabled: bool,
) -> Result<Uuid, String> {
    if matches!(kind, ItemKind::Caption | ItemKind::Comment) && !scope.is_root() {
        return Err("caption and comment tracks can only be created in the root scope".to_string());
    }
    let sequence_id = project
        .sequence_id_for_scope(scope)
        .ok_or_else(|| "track scope does not resolve in the project".to_string())?;
    match (kind, sequence_id) {
        (ItemKind::Comment, None) => {
            let track = CommentTrack {
                enabled,
                ..Default::default()
            };
            let id = track.id;
            project.comment_tracks.push(track);
            Ok(id)
        }
        (ItemKind::Caption, None) => {
            let track = CaptionTrack {
                enabled,
                ..Default::default()
            };
            let id = track.id;
            project.caption_tracks.push(track);
            Ok(id)
        }
        (ItemKind::Video, sequence_id) => {
            let track = VisualTrack {
                enabled,
                ..Default::default()
            };
            let id = track.id;
            match sequence_id {
                Some(sequence_id) => project
                    .folded_sequence_mut(sequence_id)
                    .expect("resolved sequence must exist")
                    .video_tracks
                    .push(track),
                None => project.video_tracks.push(track),
            }
            Ok(id)
        }
        (ItemKind::Audio, sequence_id) => {
            let track = AudioTrack {
                enabled,
                ..Default::default()
            };
            let id = track.id;
            match sequence_id {
                Some(sequence_id) => project
                    .folded_sequence_mut(sequence_id)
                    .expect("resolved sequence must exist")
                    .audio_tracks
                    .push(track),
                None => project.audio_tracks.push(track),
            }
            Ok(id)
        }
        (ItemKind::Comment, Some(_)) => unreachable!(),
        (ItemKind::Caption, Some(_)) => unreachable!(),
    }
}

pub fn move_item(
    project: &mut Project,
    address: &ItemAddress,
    destination: Option<&TrackAddress>,
    start: Time,
    collision: CollisionBehavior,
) -> Result<ItemAddress, String> {
    let (old_start, old_end) = project
        .item(address)
        .map(|item| item.times())
        .ok_or_else(|| "clip was not found".to_string())?;
    let duration = old_end.saturating_sub(old_start);
    let end = start.saturating_add(duration);
    let destination = destination.cloned().unwrap_or_else(|| address.track());
    if destination.kind() != address.kind() {
        return Err("destination track kind is incompatible with the clip".to_string());
    }
    if !project.can_move_item_to_sequence_path(address, destination.sequence_path()) {
        return Err("moving this folded sequence would create a recursive sequence".to_string());
    }
    let mut item = project
        .take_item(address)
        .ok_or_else(|| "clip was not found".to_string())?;
    item.set_times(start, end);
    let inserted = insert_with_collision(project, &destination, item, collision)?;
    project.normalize_clip_transitions();
    Ok(inserted)
}

pub fn trim_item(
    project: &mut Project,
    address: &ItemAddress,
    start: Option<Time>,
    end: Option<Time>,
    collision: CollisionBehavior,
) -> Result<ItemAddress, String> {
    let (old_start, old_end) = project
        .item(address)
        .map(|item| item.times())
        .ok_or_else(|| "clip was not found".to_string())?;
    let start = start.unwrap_or(old_start);
    let end = end.unwrap_or(old_end);
    if end <= start {
        return Err("trim end must be after trim start".to_string());
    }
    let mut item = project
        .take_item(address)
        .ok_or_else(|| "clip was not found".to_string())?;
    item.set_times(start, end);
    match &mut item {
        ProjectItem::Comment(_) => {}
        ProjectItem::Caption(_) => {}
        ProjectItem::Video(item) => {
            if start != old_start {
                item.transitions.intro = None;
                if !item.repeats_keyframes() {
                    item.time_offset = shifted_media_source_offset(
                        item.time_offset,
                        old_start,
                        start,
                        item.playback_speed,
                        item.repeat_strategy,
                        item.source_duration,
                    );
                    item.animation_time_offset = Time {
                        seconds: item.animation_time_offset.seconds
                            + start.signed_sub(old_start).seconds,
                    };
                }
            }
            if end != old_end {
                item.transitions.outro = None;
                item.transitions.to_next = None;
            }
            fit_visual_transitions(item);
        }
        ProjectItem::Audio(item) => {
            if start != old_start {
                item.transitions.intro = None;
                item.time_offset = shifted_media_source_offset(
                    item.time_offset,
                    old_start,
                    start,
                    item.playback_speed,
                    item.repeat_strategy,
                    item.source_duration,
                );
            }
            if end != old_end {
                item.transitions.outro = None;
                item.transitions.to_next = None;
            }
            fit_audio_transitions(item);
        }
    }
    let inserted = insert_with_collision(project, &address.track(), item, collision)?;
    project.normalize_clip_transitions();
    Ok(inserted)
}

pub fn delete_items(project: &mut Project, addresses: &[ItemAddress]) -> Result<(), String> {
    for address in addresses {
        project
            .take_item(address)
            .ok_or_else(|| format!("clip {} was not found", address.item_id()))?;
    }
    project.normalize_clip_transitions();
    Ok(())
}

pub fn set_track_enabled(
    project: &mut Project,
    address: &TrackAddress,
    enabled: bool,
) -> Result<(), String> {
    match project
        .track_mut(address)
        .ok_or_else(|| "track was not found".to_string())?
    {
        TrackMut::Comment(track) => track.enabled = enabled,
        TrackMut::Caption(track) => track.enabled = enabled,
        TrackMut::Video(track) => track.enabled = enabled,
        TrackMut::Audio(track) => track.enabled = enabled,
    }
    Ok(())
}

pub fn set_caption_track_language(
    project: &mut Project,
    address: &TrackAddress,
    language: Option<String>,
) -> Result<(), String> {
    match project
        .track_mut(address)
        .ok_or_else(|| "track was not found".to_string())?
    {
        TrackMut::Caption(track) => {
            track.language = language;
            Ok(())
        }
        TrackMut::Comment(_) | TrackMut::Video(_) | TrackMut::Audio(_) => {
            Err("language applies only to caption tracks".to_string())
        }
    }
}

pub fn delete_track(project: &mut Project, address: &TrackAddress) -> Result<(), String> {
    if !project.remove_track(address) {
        return Err("track was not found".to_string());
    }
    project.prune_folded_sequences();
    project.normalize_clip_transitions();
    Ok(())
}

pub fn set_item_text(
    project: &mut Project,
    address: &ItemAddress,
    text: String,
) -> Result<(), String> {
    match project
        .item_mut(address)
        .ok_or_else(|| "clip was not found".to_string())?
    {
        ItemMut::Comment(item) => {
            item.text = text;
            Ok(())
        }
        ItemMut::Caption(item) => {
            item.text = text;
            Ok(())
        }
        ItemMut::Video(_) | ItemMut::Audio(_) => {
            Err("text applies only to caption and comment clips".to_string())
        }
    }
}

pub fn split_caption(
    project: &mut Project,
    address: &ItemAddress,
    cut: Time,
    text_byte: usize,
) -> Result<(ItemAddress, ItemAddress), String> {
    let track = address.track();
    let mut right = project
        .caption_item(address)
        .cloned()
        .ok_or_else(|| "selected caption was not found".to_string())?;
    let cut = cut.snapped(project.frame_step());
    if !(right.start < cut && cut < right.end) {
        return Err("playhead must be inside the selected caption".to_string());
    }
    let (left_text, right_text) =
        shrimply_project_document::caption::markup::split_at_plain_text_byte(
            &right.text,
            text_byte,
        )
        .ok_or_else(|| "caption text cannot be split at that position".to_string())?;
    let mut left = right.clone();
    left.end = cut;
    left.text = left_text;
    right.id = Uuid::new_v4();
    right.start = cut;
    right.text = right_text;

    project
        .take_item(address)
        .expect("validated caption must still exist");
    let left = project
        .insert_item(&track, ProjectItem::Caption(left))
        .expect("split caption must return to its source track");
    let right = project
        .insert_item(&track, ProjectItem::Caption(right))
        .expect("split caption must return to its source track");
    Ok((left, right))
}

pub fn set_audio_enabled(
    project: &mut Project,
    address: &ItemAddress,
    enabled: bool,
) -> Result<(), String> {
    match project
        .item_mut(address)
        .ok_or_else(|| "clip was not found".to_string())?
    {
        ItemMut::Audio(item) => {
            item.enabled = enabled;
            Ok(())
        }
        ItemMut::Comment(_) | ItemMut::Caption(_) | ItemMut::Video(_) => {
            Err("enabled applies only to audio clips".to_string())
        }
    }
}

pub fn set_audio_gain(
    project: &mut Project,
    address: &ItemAddress,
    gain_db: f32,
) -> Result<(), String> {
    if !gain_db.is_finite() {
        return Err("gain_db must be finite".to_string());
    }
    match project
        .item_mut(address)
        .ok_or_else(|| "clip was not found".to_string())?
    {
        ItemMut::Audio(item) => {
            if item
                .gain
                .decibels
                .expression
                .as_ref()
                .is_some_and(|expression| expression.enabled)
                || !matches!(item.gain.decibels.base, TimelineBase::Const(_))
            {
                return Err(
                    "gain_db cannot replace keyframed or expression-driven gain".to_string()
                );
            }
            item.gain.decibels = TimelineValue::new_const(gain_db);
            Ok(())
        }
        ItemMut::Comment(_) | ItemMut::Caption(_) | ItemMut::Video(_) => {
            Err("gain_db applies only to audio clips".to_string())
        }
    }
}

pub fn set_playback(
    project: &mut Project,
    address: &ItemAddress,
    speed: Option<shrimply_math_core::Fraction>,
    repeat: Option<RepeatStrategy>,
) -> Result<(), String> {
    match project
        .item_mut(address)
        .ok_or_else(|| "clip was not found".to_string())?
    {
        ItemMut::Comment(_) => Err("playback properties do not apply to comment clips".to_string()),
        ItemMut::Caption(_) => Err("playback properties do not apply to caption clips".to_string()),
        ItemMut::Video(item) => {
            if let Some(speed) = speed {
                item.playback_speed = speed;
            }
            if let Some(repeat) = repeat {
                item.repeat_strategy = repeat;
            }
            Ok(())
        }
        ItemMut::Audio(item) => {
            if let Some(speed) = speed {
                item.playback_speed = speed;
            }
            if let Some(repeat) = repeat {
                item.repeat_strategy = repeat;
            }
            Ok(())
        }
    }
}

fn insert_with_collision(
    project: &mut Project,
    track: &TrackAddress,
    item: ProjectItem,
    collision: CollisionBehavior,
) -> Result<ItemAddress, String> {
    let (start, end) = item.times();
    let collisions = collision_addresses(project, track, start, end)?;
    match collision {
        CollisionBehavior::Reject if !collisions.is_empty() => {
            return Err("edit collides with an existing clip".to_string());
        }
        CollisionBehavior::Overwrite => {
            for address in collisions {
                project
                    .take_item(&address)
                    .expect("collision address must exist");
            }
        }
        CollisionBehavior::NewTrack if !collisions.is_empty() => {
            return project
                .insert_item_on_new_track(track.sequence_path(), item)
                .ok_or_else(|| "could not create a compatible destination track".to_string());
        }
        CollisionBehavior::Reject | CollisionBehavior::NewTrack => {}
    }
    project
        .insert_item(track, item)
        .ok_or_else(|| "destination track was not found".to_string())
}

pub fn collision_addresses(
    project: &Project,
    track: &TrackAddress,
    start: Time,
    end: Time,
) -> Result<Vec<ItemAddress>, String> {
    let items = match project
        .track(track)
        .ok_or_else(|| "destination track was not found".to_string())?
    {
        shrimply_project_document::project::TrackRef::Comment(track) => track
            .items
            .iter()
            .map(|item| (item.id, item.start, item.end))
            .collect::<Vec<_>>(),
        shrimply_project_document::project::TrackRef::Caption(track) => track
            .items
            .iter()
            .map(|item| (item.id, item.start, item.end))
            .collect::<Vec<_>>(),
        shrimply_project_document::project::TrackRef::Video(track) => track
            .items
            .iter()
            .map(|item| (item.id, item.start, item.end))
            .collect::<Vec<_>>(),
        shrimply_project_document::project::TrackRef::Audio(track) => track
            .items
            .iter()
            .map(|item| (item.id, item.start, item.end))
            .collect::<Vec<_>>(),
    };
    Ok(items
        .into_iter()
        .filter(|(_, candidate_start, candidate_end)| {
            *candidate_start < end && *candidate_end > start
        })
        .map(|(id, _, _)| track.item(id))
        .collect())
}

pub fn track_collides(
    project: &Project,
    track: &TrackAddress,
    start: Time,
    end: Time,
) -> Result<bool, String> {
    Ok(!collision_addresses(project, track, start, end)?.is_empty())
}

pub fn overwrite_interval(
    project: &mut Project,
    track: &TrackAddress,
    start: Time,
    end: Time,
) -> Result<(), String> {
    for address in collision_addresses(project, track, start, end)? {
        project
            .take_item(&address)
            .expect("collision address must exist");
    }
    project.normalize_clip_transitions();
    Ok(())
}

pub fn validate_properties_target(
    project: &Project,
    address: &ItemAddress,
    has_text: bool,
    has_enabled: bool,
    has_gain: bool,
    has_playback: bool,
) -> Result<(), String> {
    match project
        .item(address)
        .ok_or_else(|| "clip was not found".to_string())?
    {
        ItemRef::Comment(_) | ItemRef::Caption(_) if has_enabled || has_gain || has_playback => {
            Err("one or more properties do not apply to caption or comment clips".to_string())
        }
        ItemRef::Video(_) if has_text || has_enabled || has_gain => {
            Err("one or more properties do not apply to video clips".to_string())
        }
        ItemRef::Audio(_) if has_text => Err("text does not apply to audio clips".to_string()),
        _ => Ok(()),
    }
}

pub fn shifted_media_source_offset(
    offset: Time,
    old_start: Time,
    new_start: Time,
    playback_speed: shrimply_math_core::Fraction,
    _repeat_strategy: RepeatStrategy,
    _source_duration: Time,
) -> Time {
    offset.saturating_add(scaled_time_delta(
        new_start.signed_sub(old_start),
        playback_speed,
    ))
}

pub fn advanced_media_source_offset(
    offset: Time,
    delta: Time,
    _repeat_strategy: RepeatStrategy,
    _source_duration: Time,
) -> Time {
    offset.saturating_add(delta)
}

pub fn fit_visual_transitions(item: &mut VideoItem) {
    let (intro, outro) = fitted_transition_durations(
        item.end.saturating_sub(item.start),
        item.transitions.intro.as_ref().map(|value| value.duration),
        item.transitions.outro.as_ref().map(|value| value.duration),
    );
    if let (Some(transition), Some(duration)) = (item.transitions.intro.as_mut(), intro) {
        transition.duration = duration;
    }
    if let (Some(transition), Some(duration)) = (item.transitions.outro.as_mut(), outro) {
        transition.duration = duration;
    }
}

pub fn fit_audio_transitions(item: &mut AudioItem) {
    let (intro, outro) = fitted_transition_durations(
        item.end.saturating_sub(item.start),
        item.transitions.intro.as_ref().map(|value| value.duration),
        item.transitions.outro.as_ref().map(|value| value.duration),
    );
    if let (Some(transition), Some(duration)) = (item.transitions.intro.as_mut(), intro) {
        transition.duration = duration;
    }
    if let (Some(transition), Some(duration)) = (item.transitions.outro.as_mut(), outro) {
        transition.duration = duration;
    }
}

pub fn fitted_transition_durations(
    item_duration: Time,
    intro: Option<Time>,
    outro: Option<Time>,
) -> (Option<Time>, Option<Time>) {
    let intro_seconds = intro.unwrap_or(Time::ZERO).seconds;
    let outro_seconds = outro.unwrap_or(Time::ZERO).seconds;
    if intro_seconds + outro_seconds <= item_duration.seconds {
        return (intro, outro);
    }
    let (fitted_intro, fitted_outro) =
        fit_nonnegative_fraction_pair(item_duration.seconds, intro_seconds, outro_seconds);
    (
        intro.map(|_| Time {
            seconds: fitted_intro,
        }),
        outro.map(|_| Time {
            seconds: fitted_outro,
        }),
    )
}

pub fn split_item_address(
    project: &mut Project,
    address: &shrimply_project_document::project::ItemAddress,
    timeline_cut: Time,
) -> Option<(
    shrimply_project_document::project::ItemAddress,
    shrimply_project_document::project::ItemAddress,
)> {
    use shrimply_project_document::project::{ItemRef, ProjectItem};

    let track = address.track();
    let cut = project
        .timeline_time_to_sequence(&track, timeline_cut)?
        .snapped(project.frame_step());
    let source = match project.item(address)? {
        ItemRef::Comment(item) => ProjectItem::Comment(item.clone()),
        ItemRef::Caption(item) => ProjectItem::Caption(item.clone()),
        ItemRef::Video(item) => ProjectItem::Video(Box::new(item.clone())),
        ItemRef::Audio(item) => ProjectItem::Audio(Box::new(item.clone())),
    };
    let (start, end) = source.times();
    if !(start < cut && cut < end) {
        return None;
    }

    let mut left = source.clone();
    let mut right = source;
    match (&mut left, &mut right) {
        (ProjectItem::Comment(left), ProjectItem::Comment(right)) => {
            left.end = cut;
            right.start = cut;
            right.id = uuid::Uuid::new_v4();
        }
        (ProjectItem::Caption(left), ProjectItem::Caption(right)) => {
            left.end = cut;
            right.start = cut;
            right.id = uuid::Uuid::new_v4();
        }
        (ProjectItem::Video(left), ProjectItem::Video(right)) => {
            let source_start = right.start;
            let source_offset = right.time_offset;
            left.end = cut;
            left.transitions.outro = None;
            left.transitions.to_next = None;
            right.start = cut;
            right.transitions.intro = None;
            right.id = uuid::Uuid::new_v4();
            Project::regenerate_video_property_ids(right);
            right.time_offset = advanced_media_source_offset(
                source_offset,
                scaled_time_delta(cut.saturating_sub(source_start), right.playback_speed),
                right.repeat_strategy,
                right.source_duration,
            );
            right.animation_time_offset = right
                .animation_time_offset
                .saturating_add(cut.saturating_sub(source_start));
            fit_visual_transitions(left);
            fit_visual_transitions(right);
        }
        (ProjectItem::Audio(left), ProjectItem::Audio(right)) => {
            let source_start = right.start;
            let source_offset = right.time_offset;
            left.end = cut;
            left.transitions.outro = None;
            left.transitions.to_next = None;
            right.start = cut;
            right.transitions.intro = None;
            right.id = uuid::Uuid::new_v4();
            Project::regenerate_audio_property_ids(right);
            right.time_offset = advanced_media_source_offset(
                source_offset,
                scaled_time_delta(cut.saturating_sub(source_start), right.playback_speed),
                right.repeat_strategy,
                right.source_duration,
            );
            fit_audio_transitions(left);
            fit_audio_transitions(right);
        }
        _ => unreachable!("cloned timeline item kinds must match"),
    }

    project
        .take_item(address)
        .expect("validated timeline item must still exist");
    let left = project
        .insert_item(&track, left)
        .expect("split item must return to its source track");
    let right = project
        .insert_item(&track, right)
        .expect("split item must return to its source track");
    Some((left, right))
}
