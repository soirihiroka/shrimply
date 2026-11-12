use std::collections::HashSet;

use shrimply_math_core::{Fraction, Time, fraction_new, time_from_frame};
use shrimply_project_document::project::{
    AudioTransition, CaptionItem, ItemAddress, ItemKind, ItemMut, Project, ProjectItem,
    RepeatStrategy, SequenceScopeId, Time as ProjectTime, TrackAddress as ModelTrackAddress,
    TrackRef, Transform, TransitionSide, VideoItemContent, VisualTransition, caption_languages,
};
use shrimply_property_model::timeline_value::{TimelineBase, TimelineValue};
use shrimply_timeline_edit::edit::{self, CollisionBehavior as ModelCollision};
use shrimply_visual_modifiers::{
    ModifierEffect, RasterModifierEffect, VisualKind,
    crop::{CropEdges as ModelCropEdges, CropModifier},
};
use uuid::Uuid;

use crate::protocol::{
    ClipAddress, ClipSummary, ClipTransitionInput, CollisionBehavior, CropEdges, CropMode,
    EditOperation, ExactFraction, InsertCaptionsRequest, InsertTextOperation,
    ReplaceClipSourceRequest, SetClipPropertiesRequest, SetClipTransitionsRequest,
    SetVideoTransformRequest, TrackAddress, VideoFitMode,
};
use crate::query::{model_item_address, model_kind, model_track_address};

#[derive(Default)]
pub struct MutationResult {
    pub details: Option<serde_json::Value>,
    pub changed_item_ids: Vec<Uuid>,
    pub deleted_addresses: Vec<ClipAddress>,
    pub deleted_presentations: Vec<ClipSummary>,
    pub changed_tracks: Vec<TrackAddress>,
}

pub fn source_replacement_target(
    project: &Project,
    request: &ReplaceClipSourceRequest,
) -> Result<ItemAddress, String> {
    if request.address.kind != crate::protocol::ClipKind::Video {
        return Err("replace_clip_source requires a video clip address".into());
    }
    let address = model_item_address(&request.address)?;
    if project.video_item(&address).is_none() {
        return Err("replacement target video clip was not found".into());
    }
    if request.link && request.copy_root.is_some() {
        return Err("link mode forbids copy_root".into());
    }
    if !std::path::Path::new(&request.source).is_file() {
        return Err(format!("source file does not exist: {}", request.source));
    }
    Ok(address)
}

pub fn apply_non_import(
    project: &mut Project,
    operation: &EditOperation,
    anchor: u64,
    scope: &SequenceScopeId,
) -> Result<MutationResult, String> {
    match operation {
        EditOperation::SetProjectSettings(request) => crate::extra::settings(project, request),
        EditOperation::SplitClip(request) => crate::extra::split(project, request),
        EditOperation::DuplicateClip(request) => crate::extra::duplicate(project, request),
        EditOperation::ReorderTrack(request) => crate::extra::reorder(project, request),
        EditOperation::ReplaceText(request) => crate::text::apply(project, request),
        EditOperation::InsertFiles(_) => {
            Err("insert_files must be handled by the native importer".to_string())
        }
        EditOperation::ReplaceClipSource(_) => {
            Err("replace_clip_source must be handled by the native importer".into())
        }
        EditOperation::InsertTts(_) => {
            Err("insert_tts must be handled by the native editor".to_string())
        }
        EditOperation::InsertText(request) => insert_text(project, request, anchor, scope),
        EditOperation::InsertCaptions(request) => insert_captions(project, request),
        EditOperation::CreateTrack(request) => {
            let id = edit::create_track(
                project,
                scope,
                model_kind(request.kind),
                request.enabled.unwrap_or(true),
            )?;
            Ok(MutationResult {
                changed_tracks: crate::query::addresses_for_tracks(project, &HashSet::from([id]))?,
                ..Default::default()
            })
        }
        EditOperation::MoveClip(request) => {
            let address = model_item_address(&request.address)?;
            let destination = request
                .destination
                .as_ref()
                .map(model_track_address)
                .transpose()?;
            let projected = frame(
                project,
                resolve_frame(
                    request.start_frame,
                    request.offset_frames,
                    anchor,
                    "move start",
                )?,
            )?;
            let path = destination
                .as_ref()
                .map(|track| track.sequence_path())
                .unwrap_or_else(|| address.sequence_path());
            let local = project
                .timeline_time_to_sequence_path(model_kind(request.address.kind), path, projected)
                .ok_or_else(|| "destination scope does not resolve in the project".to_string())?
                .snapped(project.frame_step());
            let destination = destination.unwrap_or_else(|| address.track());
            let duration = project
                .item(&address)
                .map(|item| {
                    let (start, end) = item.times();
                    end.saturating_sub(start)
                })
                .ok_or_else(|| "clip was not found".to_string())?;
            let deleted = overwritten_presentations(
                project,
                &destination,
                local,
                local.saturating_add(duration),
                address.item_id(),
                request.collision,
            )?;
            let result = edit::move_item(
                project,
                &address,
                Some(&destination),
                local,
                collision(request.collision),
            )?;
            changed_after_insert(project, &result, &destination, deleted)
        }
        EditOperation::TrimClip(request) => {
            let address = model_item_address(&request.address)?;
            let start = optional_frame(
                project,
                request.start_frame,
                request.start_offset_frames,
                anchor,
                "trim start",
            )?
            .map(|time| {
                project
                    .timeline_time_to_sequence_path(
                        model_kind(request.address.kind),
                        address.sequence_path(),
                        time,
                    )
                    .map(|time| time.snapped(project.frame_step()))
                    .ok_or_else(|| "clip scope does not resolve in the project".to_string())
            })
            .transpose()?;
            let end = optional_frame(
                project,
                request.end_frame,
                request.end_offset_frames,
                anchor,
                "trim end",
            )?
            .map(|time| {
                project
                    .timeline_time_to_sequence_path(
                        model_kind(request.address.kind),
                        address.sequence_path(),
                        time,
                    )
                    .map(|time| time.snapped(project.frame_step()))
                    .ok_or_else(|| "clip scope does not resolve in the project".to_string())
            })
            .transpose()?;
            if start.is_none() && end.is_none() {
                return Err("trim_clip requires a start or end frame".to_string());
            }
            let (old_start, old_end) = project
                .item(&address)
                .map(|item| item.times())
                .ok_or_else(|| "clip was not found".to_string())?;
            let deleted = overwritten_presentations(
                project,
                &address.track(),
                start.unwrap_or(old_start),
                end.unwrap_or(old_end),
                address.item_id(),
                request.collision,
            )?;
            let result =
                edit::trim_item(project, &address, start, end, collision(request.collision))?;
            changed_after_insert(project, &result, &address.track(), deleted)
        }
        EditOperation::DeleteClips(request) => {
            if request.addresses.is_empty() {
                return Err("delete_clips requires at least one address".to_string());
            }
            let mut addresses = request
                .addresses
                .iter()
                .map(model_item_address)
                .collect::<Result<Vec<_>, _>>()?;
            for address in &addresses {
                if project.item(address).is_none() {
                    return Err(format!("clip {} was not found", address.item_id()));
                }
            }
            addresses.sort_by_key(|address| std::cmp::Reverse(address.sequence_path().len()));
            let mut logical_items = std::collections::HashSet::new();
            addresses.retain(|address| {
                logical_items.insert((address.kind(), address.track_id(), address.item_id()))
            });
            let item_ids = addresses.iter().map(|address| address.item_id()).collect();
            let deleted_presentations =
                crate::query::presentations_affected_by_items(project, &item_ids)?;
            edit::delete_items(project, &addresses)?;
            Ok(MutationResult {
                deleted_addresses: deleted_presentations
                    .iter()
                    .map(|clip| clip.address.clone())
                    .collect(),
                deleted_presentations,
                ..Default::default()
            })
        }
        EditOperation::SetClipProperties(request) => set_properties(project, request),
        EditOperation::SetVideoTransform(request) => set_video_transform(project, request),
        EditOperation::UpsertKeyframes(request) => Ok(changed(crate::property::upsert_keyframes(
            project, request,
        )?)),
        EditOperation::DeleteKeyframes(request) => Ok(changed(crate::property::delete_keyframes(
            project, request,
        )?)),
        EditOperation::UpsertPropertyExpression(request) => Ok(changed(
            crate::property::upsert_expression(project, request)?,
        )),
        EditOperation::DeletePropertyExpression(request) => Ok(changed(
            crate::property::delete_expression(project, request)?,
        )),
        EditOperation::SetClipTransitions(request) => set_clip_transitions(project, request),
        EditOperation::SetExpression(request) => {
            Ok(changed(crate::expression::set(project, request)?))
        }
        EditOperation::SetTrackEnabled(request) => {
            let address = model_track_address(&request.address)?;
            edit::set_track_enabled(project, &address, request.enabled)?;
            let track_ids = [address.track_id()].into_iter().collect();
            let presentations = crate::query::presentations_for_tracks(project, &track_ids)?;
            Ok(MutationResult {
                changed_item_ids: presentations
                    .iter()
                    .filter_map(|clip| Uuid::parse_str(&clip.address.item_id).ok())
                    .collect(),
                changed_tracks: crate::query::addresses_for_tracks(project, &track_ids)?,
                ..Default::default()
            })
        }
        EditOperation::SetCaptionTrackLanguage(request) => {
            if let Some(language) = &request.language
                && !caption_languages().contains(language)
            {
                return Err(format!("{language} is not a supported caption language"));
            }
            let address = model_track_address(&request.address)?;
            edit::set_caption_track_language(project, &address, request.language.clone())?;
            let track_ids = [address.track_id()].into_iter().collect();
            let presentations = crate::query::presentations_for_tracks(project, &track_ids)?;
            Ok(MutationResult {
                changed_item_ids: presentations
                    .iter()
                    .filter_map(|clip| Uuid::parse_str(&clip.address.item_id).ok())
                    .collect(),
                changed_tracks: crate::query::addresses_for_tracks(project, &track_ids)?,
                ..Default::default()
            })
        }
        EditOperation::DeleteTrack(request) => {
            let address = model_track_address(&request.address)?;
            let item_ids = match project
                .track(&address)
                .ok_or_else(|| "track was not found".to_string())?
            {
                TrackRef::Comment(track) => track.items.iter().map(|item| item.id).collect(),
                TrackRef::Caption(track) => track.items.iter().map(|item| item.id).collect(),
                TrackRef::Video(track) => track.items.iter().map(|item| item.id).collect(),
                TrackRef::Audio(track) => track.items.iter().map(|item| item.id).collect(),
            };
            let deleted_presentations =
                crate::query::presentations_affected_by_items(project, &item_ids)?;
            edit::delete_track(project, &address)?;
            Ok(MutationResult {
                deleted_addresses: deleted_presentations
                    .iter()
                    .map(|clip| clip.address.clone())
                    .collect(),
                deleted_presentations,
                changed_tracks: vec![request.address.clone()],
                ..Default::default()
            })
        }
    }
}

fn insert_captions(
    project: &mut Project,
    request: &InsertCaptionsRequest,
) -> Result<MutationResult, String> {
    if request.captions.is_empty() {
        return Err("insert_captions requires at least one caption".to_string());
    }
    if let Some(language) = &request.language
        && !caption_languages().contains(language)
    {
        return Err(format!("{language} is not a supported caption language"));
    }

    let mut captions = request
        .captions
        .iter()
        .map(|cue| {
            let start = frame(project, cue.start_frame)?;
            let end = frame(project, cue.end_frame)?;
            if end <= start {
                return Err(format!(
                    "caption end_frame {} must be after start_frame {}",
                    cue.end_frame, cue.start_frame
                ));
            }
            let mut caption = if let Some(source) = &cue.copy_style_from {
                let source = model_item_address(source)?;
                project
                    .caption_item(&source)
                    .cloned()
                    .ok_or_else(|| "copy_style_from must address a caption clip".to_string())?
            } else {
                CaptionItem::new(start, end, cue.text.clone())
            };
            caption.id = Uuid::new_v4();
            caption.start = start;
            caption.end = end;
            caption.text = cue.text.clone();
            caption.group_id = None;
            Ok(caption)
        })
        .collect::<Result<Vec<_>, String>>()?;
    captions.sort_by_key(|caption| (caption.start, caption.end));
    if captions.windows(2).any(|pair| pair[0].end > pair[1].start) {
        return Err("inserted captions cannot overlap each other".to_string());
    }

    let requested_track = request
        .track
        .as_ref()
        .map(model_track_address)
        .transpose()?;
    if requested_track
        .as_ref()
        .is_some_and(|track| track.kind() != ItemKind::Caption)
    {
        return Err("insert_captions requires a caption track".to_string());
    }
    let mut target = if let Some(track) = requested_track {
        if project.track(&track).is_none() {
            return Err("caption track was not found".to_string());
        }
        track
    } else {
        create_caption_track(project, request.enabled.unwrap_or(true))?
    };

    let mut collisions = Vec::new();
    for caption in &captions {
        collisions.extend(edit::collision_addresses(
            project,
            &target,
            caption.start,
            caption.end,
        )?);
    }
    collisions.sort_by_key(|address| address.item_id());
    collisions.dedup_by_key(|address| address.item_id());
    let deleted_presentations = match request.collision {
        CollisionBehavior::Reject if !collisions.is_empty() => {
            return Err("caption insertion collides with an existing clip".to_string());
        }
        CollisionBehavior::NewTrack if !collisions.is_empty() => {
            let (enabled, language) = match project
                .track(&target)
                .expect("validated caption track must exist")
            {
                TrackRef::Comment(_) => unreachable!("validated caption track must be a caption"),
                TrackRef::Caption(track) => (track.enabled, track.language.clone()),
                TrackRef::Video(_) | TrackRef::Audio(_) => unreachable!(),
            };
            target = create_caption_track(project, request.enabled.unwrap_or(enabled))?;
            edit::set_caption_track_language(
                project,
                &target,
                request.language.clone().or(language),
            )?;
            Vec::new()
        }
        CollisionBehavior::Overwrite => {
            let item_ids = collisions.iter().map(|address| address.item_id()).collect();
            let presentations = crate::query::presentations_affected_by_items(project, &item_ids)?;
            edit::delete_items(project, &collisions)?;
            presentations
        }
        CollisionBehavior::Reject | CollisionBehavior::NewTrack => Vec::new(),
    };

    if let Some(enabled) = request.enabled {
        edit::set_track_enabled(project, &target, enabled)?;
    }
    if let Some(language) = &request.language {
        edit::set_caption_track_language(project, &target, Some(language.clone()))?;
    }
    let mut changed_item_ids = Vec::with_capacity(captions.len());
    for caption in captions {
        changed_item_ids.push(caption.id);
        project
            .insert_item(&target, ProjectItem::Caption(caption))
            .expect("validated caption track must accept caption items");
    }
    let track_ids = HashSet::from([target.track_id()]);
    Ok(MutationResult {
        details: None,
        changed_item_ids,
        deleted_addresses: deleted_presentations
            .iter()
            .map(|caption| caption.address.clone())
            .collect(),
        deleted_presentations,
        changed_tracks: crate::query::addresses_for_tracks(project, &track_ids)?,
    })
}

fn insert_text(
    project: &mut Project,
    request: &InsertTextOperation,
    anchor: u64,
    scope: &SequenceScopeId,
) -> Result<MutationResult, String> {
    if request.text.is_empty() {
        return Err("insert_text requires nonempty text".to_string());
    }
    if request.duration_frames == 0 {
        return Err("insert_text duration_frames must be positive".to_string());
    }
    let end_frame = anchor
        .checked_add(request.duration_frames)
        .ok_or_else(|| "insert_text end frame overflow".to_string())?;
    let projected_start = frame(project, anchor)?;
    let projected_end = frame(project, end_frame)?;
    let requested_track = request
        .track
        .as_ref()
        .map(model_track_address)
        .transpose()?;
    if requested_track
        .as_ref()
        .is_some_and(|track| track.kind() != ItemKind::Video)
    {
        return Err("insert_text requires a video track".to_string());
    }
    if requested_track
        .as_ref()
        .is_some_and(|track| project.track(track).is_none())
    {
        return Err("video track was not found".to_string());
    }
    let path = requested_track
        .as_ref()
        .map(|track| track.sequence_path().to_vec())
        .or_else(|| project.sequence_path_for_scope(ItemKind::Video, scope))
        .ok_or_else(|| "text insertion scope does not have one concrete video path".to_string())?;
    let start = project
        .timeline_time_to_sequence_path(ItemKind::Video, &path, projected_start)
        .ok_or_else(|| "text insertion scope does not resolve in the project".to_string())?
        .snapped(project.frame_step());
    let end = project
        .timeline_time_to_sequence_path(ItemKind::Video, &path, projected_end)
        .ok_or_else(|| "text insertion scope does not resolve in the project".to_string())?
        .snapped(project.frame_step());
    if end <= start {
        return Err(
            "insert_text must have a positive duration in its destination scope".to_string(),
        );
    }
    let mut item =
        shrimply_project_document::project::VideoItem::text_item(project.canvas_size, start, end);
    let VideoItemContent::Text(text) = &mut item.content else {
        unreachable!("text item constructor returned another item type");
    };
    text.text = TimelineValue::new_const(request.text.clone());
    let item_id = item.id;

    let target = if let Some(track) = requested_track {
        Some(track)
    } else {
        project
            .video_tracks_for_scope(scope)
            .ok_or_else(|| "text insertion scope was not found".to_string())?
            .iter()
            .map(|track| ModelTrackAddress::Video {
                sequence_path: path.clone(),
                track_id: track.id,
            })
            .find(|track| {
                edit::collision_addresses(project, track, start, end)
                    .is_ok_and(|collisions| collisions.is_empty())
            })
    };
    let mut deleted_presentations = Vec::new();
    let inserted = if let Some(target) = target {
        let collisions = edit::collision_addresses(project, &target, start, end)?;
        match request.collision {
            CollisionBehavior::Reject if !collisions.is_empty() => {
                return Err("text insertion collides with an existing clip".to_string());
            }
            CollisionBehavior::Overwrite if !collisions.is_empty() => {
                let item_ids = collisions.iter().map(|address| address.item_id()).collect();
                deleted_presentations =
                    crate::query::presentations_affected_by_items(project, &item_ids)?;
                edit::delete_items(project, &collisions)?;
                project
                    .insert_item(&target, ProjectItem::Video(Box::new(item)))
                    .expect("validated video track must accept a text item")
            }
            CollisionBehavior::NewTrack if !collisions.is_empty() => project
                .insert_item_on_new_track(&path, ProjectItem::Video(Box::new(item)))
                .ok_or_else(|| "could not create a video track for the text clip".to_string())?,
            CollisionBehavior::Reject
            | CollisionBehavior::NewTrack
            | CollisionBehavior::Overwrite => project
                .insert_item(&target, ProjectItem::Video(Box::new(item)))
                .expect("validated video track must accept a text item"),
        }
    } else {
        project
            .insert_item_on_new_track(&path, ProjectItem::Video(Box::new(item)))
            .ok_or_else(|| "could not create a video track for the text clip".to_string())?
    };
    let mut changed_tracks = Vec::new();
    if request
        .track
        .as_ref()
        .is_none_or(|track| track.track_id != inserted.track_id().to_string())
    {
        changed_tracks =
            crate::query::addresses_for_tracks(project, &HashSet::from([inserted.track_id()]))?;
    }
    Ok(MutationResult {
        details: None,
        changed_item_ids: vec![item_id],
        deleted_addresses: deleted_presentations
            .iter()
            .map(|clip| clip.address.clone())
            .collect(),
        deleted_presentations,
        changed_tracks,
    })
}

fn create_caption_track(project: &mut Project, enabled: bool) -> Result<ModelTrackAddress, String> {
    Ok(ModelTrackAddress::Caption {
        track_id: edit::create_track(
            project,
            &SequenceScopeId::root(),
            ItemKind::Caption,
            enabled,
        )?,
    })
}

fn set_properties(
    project: &mut Project,
    request: &SetClipPropertiesRequest,
) -> Result<MutationResult, String> {
    if request.text.is_none()
        && request.enabled.is_none()
        && request.gain_db.is_none()
        && request.playback_speed.is_none()
        && request.repeat_strategy.is_none()
    {
        return Err("set_clip_properties requires at least one property".to_string());
    }
    let address = model_item_address(&request.address)?;
    let has_playback = request.playback_speed.is_some() || request.repeat_strategy.is_some();
    edit::validate_properties_target(
        project,
        &address,
        request.text.is_some(),
        request.enabled.is_some(),
        request.gain_db.is_some(),
        has_playback,
    )?;
    if let Some(text) = &request.text {
        edit::set_item_text(project, &address, text.clone())?;
    }
    if let Some(enabled) = request.enabled {
        edit::set_audio_enabled(project, &address, enabled)?;
    }
    if let Some(gain_db) = request.gain_db {
        edit::set_audio_gain(project, &address, gain_db)?;
    }
    let speed = request
        .playback_speed
        .as_ref()
        .map(positive_fraction)
        .transpose()?;
    let repeat = request
        .repeat_strategy
        .as_deref()
        .map(parse_repeat)
        .transpose()?;
    if has_playback {
        edit::set_playback(project, &address, speed, repeat)?;
    }
    Ok(changed(address.item_id()))
}

fn set_video_transform(
    project: &mut Project,
    request: &SetVideoTransformRequest,
) -> Result<MutationResult, String> {
    if request.address.kind != crate::protocol::ClipKind::Video {
        return Err("set_video_transform requires a video clip address".to_string());
    }
    if request.fit_mode.is_none()
        && request.position.is_none()
        && request.scale.is_none()
        && request.rotation_degrees.is_none()
        && request.anchor.is_none()
        && request.shear.is_none()
        && request.crop.is_none()
    {
        return Err("set_video_transform requires at least one property".to_string());
    }
    for (name, value) in [
        ("position", request.position),
        ("scale", request.scale),
        ("anchor", request.anchor),
        ("shear", request.shear),
    ] {
        if value.is_some_and(|value| !value.x.is_finite() || !value.y.is_finite()) {
            return Err(format!("{name} values must be finite"));
        }
    }
    if request
        .scale
        .is_some_and(|scale| scale.x < 0.0 || scale.y < 0.0)
    {
        return Err("scale values must be nonnegative".to_string());
    }
    if request
        .rotation_degrees
        .is_some_and(|rotation| !rotation.is_finite())
    {
        return Err("rotation_degrees must be finite".to_string());
    }
    if let Some(crop) = request.crop {
        validate_crop(crop)?;
    }

    let address = model_item_address(&request.address)?;
    let canvas = project.canvas_size;
    let item = project
        .video_item_mut(&address)
        .ok_or_else(|| "video clip was not found".to_string())?;
    if let Some(mode) = request.fit_mode {
        item.transform = match mode {
            VideoFitMode::Natural => item.natural_transform(canvas),
            VideoFitMode::Contain => {
                require_source_size(item)?;
                Transform::contain(canvas, item.source_width, item.source_height)
            }
            VideoFitMode::Cover => {
                require_source_size(item)?;
                Transform::cover(canvas, item.source_width, item.source_height)
            }
            VideoFitMode::Stretch => {
                require_source_size(item)?;
                Transform::stretch(canvas, item.source_width, item.source_height)
            }
        };
    }
    if let Some(position) = request.position {
        item.transform.position.base = TimelineBase::Const([position.x, position.y].into());
    }
    if let Some(scale) = request.scale {
        item.transform.scale.base = TimelineBase::Const([scale.x, scale.y].into());
    }
    if let Some(rotation) = request.rotation_degrees {
        item.transform.rotation_degrees.base = TimelineBase::Const(rotation);
    }
    if let Some(anchor) = request.anchor {
        item.transform.anchor.base = TimelineBase::Const([anchor.x, anchor.y].into());
    }
    if let Some(shear) = request.shear {
        item.transform.shear.base = TimelineBase::Const([shear.x, shear.y].into());
    }
    if let Some(crop) = request.crop {
        set_crop(item, crop)?;
    }
    Ok(changed(address.item_id()))
}

fn require_source_size(item: &shrimply_project_document::project::VideoItem) -> Result<(), String> {
    if item.source_width == 0 || item.source_height == 0 {
        Err("fit_mode requires known nonzero source dimensions".to_string())
    } else {
        Ok(())
    }
}

fn validate_crop(crop: CropEdges) -> Result<(), String> {
    let values = [crop.top, crop.right, crop.bottom, crop.left];
    if values
        .iter()
        .any(|value| !value.is_finite() || *value < 0.0)
    {
        return Err("crop edges must be finite and nonnegative".to_string());
    }
    if matches!(crop.mode, CropMode::Percentage)
        && (values.iter().any(|value| *value > 100.0)
            || crop.top + crop.bottom >= 100.0
            || crop.left + crop.right >= 100.0)
    {
        return Err("percentage crop edges must leave a nonempty image".to_string());
    }
    Ok(())
}

fn set_crop(
    item: &mut shrimply_project_document::project::VideoItem,
    crop: CropEdges,
) -> Result<(), String> {
    let edges = ModelCropEdges {
        top: TimelineValue::new_const(crop.top),
        right: TimelineValue::new_const(crop.right),
        bottom: TimelineValue::new_const(crop.bottom),
        left: TimelineValue::new_const(crop.left),
    };
    let crop = match crop.mode {
        CropMode::Percentage => CropModifier::Percentage(edges),
        CropMode::Pixels => CropModifier::Pixels(edges),
    };
    if let Some(existing) =
        item.modifiers
            .iter_mut()
            .find_map(|modifier| match &mut modifier.effect {
                ModifierEffect::Raster(effect) => match &mut **effect {
                    RasterModifierEffect::Crop(crop) => Some(crop),
                    _ => None,
                },
                _ => None,
            })
    {
        *existing = crop;
        return Ok(());
    }
    if item.modifier_output_state()?.kind != VisualKind::Raster {
        return Err("crop requires a raster clip or an existing Rasterize modifier".to_string());
    }
    item.modifiers
        .push(shrimply_project_document::project::VisualModifier::new(
            ModifierEffect::raster(RasterModifierEffect::Crop(crop)),
        ));
    Ok(())
}

fn set_clip_transitions(
    project: &mut Project,
    request: &SetClipTransitionsRequest,
) -> Result<MutationResult, String> {
    if request.updates.is_empty() {
        return Err("set_clip_transitions requires at least one update".to_string());
    }
    let mut sides = HashSet::new();
    let updates = request
        .updates
        .iter()
        .map(|update| {
            if !sides.insert(update.side) {
                return Err(format!(
                    "set_clip_transitions contains more than one {:?} update",
                    update.side
                ));
            }
            Ok((
                update.side,
                update
                    .transition
                    .map(|input| transition_input(project, input))
                    .transpose()?,
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let address = model_item_address(&request.address)?;
    let canvas = project.canvas_size;
    match project
        .item_mut(&address)
        .ok_or_else(|| "clip was not found".to_string())?
    {
        ItemMut::Comment(_) => {
            return Err("comment clips do not support intro/outro transitions".to_string());
        }
        ItemMut::Caption(_) => {
            return Err("caption clips do not support intro/outro transitions".to_string());
        }
        ItemMut::Video(item) => {
            if matches!(
                item.content,
                VideoItemContent::Obj(_) | VideoItemContent::Gaussian(_)
            ) {
                return Err(
                    "2D intro/outro transitions are not supported for 3D scene clips".to_string(),
                );
            }
            for (side, input) in updates {
                let slot = match side {
                    TransitionSide::Intro => &mut item.transitions.intro,
                    TransitionSide::Outro => &mut item.transitions.outro,
                };
                apply_transition_update(
                    slot,
                    input,
                    |duration| VisualTransition::new(side, duration, canvas),
                    |transition, duration, input| {
                        transition.duration = duration;
                        if let Some(kind) = input.kind {
                            transition.set_kind(side, kind);
                        }
                        if let Some(interpolation) = input.interpolation {
                            transition.interpolation = interpolation;
                        }
                        Ok(())
                    },
                )?;
            }
        }
        ItemMut::Audio(item) => {
            for (side, input) in updates {
                let slot = match side {
                    TransitionSide::Intro => &mut item.transitions.intro,
                    TransitionSide::Outro => &mut item.transitions.outro,
                };
                apply_transition_update(
                    slot,
                    input,
                    |duration| AudioTransition::new(side, duration),
                    |transition, duration, input| {
                        if input.kind.is_some_and(|kind| {
                            kind != shrimply_project_document::project::VisualTransitionKind::Fade
                        }) {
                            return Err(
                                "audio clips only support fade intro/outro transitions".to_string()
                            );
                        }
                        transition.duration = duration;
                        if let Some(interpolation) = input.interpolation {
                            transition.interpolation = interpolation;
                        }
                        Ok(())
                    },
                )?;
            }
        }
    }
    Ok(changed(address.item_id()))
}

fn transition_input(
    project: &Project,
    input: ClipTransitionInput,
) -> Result<(Time, ClipTransitionInput), String> {
    if input.duration_frames == 0 {
        return Err("transition duration_frames must be positive".to_string());
    }
    Ok((frame(project, input.duration_frames)?, input))
}

fn apply_transition_update<T>(
    slot: &mut Option<T>,
    input: Option<(Time, ClipTransitionInput)>,
    create: impl FnOnce(Time) -> T,
    update: impl FnOnce(&mut T, Time, ClipTransitionInput) -> Result<(), String>,
) -> Result<(), String> {
    let Some((duration, input)) = input else {
        *slot = None;
        return Ok(());
    };
    update(
        slot.get_or_insert_with(|| create(duration)),
        duration,
        input,
    )
}

pub fn positive_fraction(value: &ExactFraction) -> Result<Fraction, String> {
    if value.numerator <= 0 || value.denominator <= 0 {
        return Err("playback_speed must be a positive exact fraction".to_string());
    }
    Ok(fraction_new(value.numerator, value.denominator))
}

fn parse_repeat(value: &str) -> Result<RepeatStrategy, String> {
    match value {
        "repeat" => Ok(RepeatStrategy::Repeat),
        "ping_pong" => Ok(RepeatStrategy::PingPong),
        "hold" => Ok(RepeatStrategy::Hold),
        "empty" => Ok(RepeatStrategy::Empty),
        _ => Err("repeat_strategy must be repeat, ping_pong, hold, or empty".to_string()),
    }
}

fn frame(project: &Project, value: u64) -> Result<Time, String> {
    time_from_frame(value, project.fps)
        .ok_or_else(|| "frame exceeds the supported exact fraction range".to_string())
}

fn optional_frame(
    project: &Project,
    absolute: Option<u64>,
    offset: Option<i64>,
    anchor: u64,
    name: &str,
) -> Result<Option<Time>, String> {
    match (absolute, offset) {
        (None, None) => Ok(None),
        _ => resolve_frame(absolute, offset, anchor, name)
            .and_then(|value| frame(project, value))
            .map(Some),
    }
}

fn resolve_frame(
    absolute: Option<u64>,
    offset: Option<i64>,
    anchor: u64,
    name: &str,
) -> Result<u64, String> {
    match (absolute, offset) {
        (Some(frame), None) => Ok(frame),
        (None, Some(offset)) => {
            frame_with_offset(anchor, offset).map_err(|error| format!("{name}: {error}"))
        }
        (Some(_), Some(_)) => Err(format!(
            "provide exactly one of {name}_frame and {name}_offset_frames"
        )),
        (None, None) => Err(format!("{name} frame is required")),
    }
}

pub fn frame_with_offset(anchor: u64, offset: i64) -> Result<u64, String> {
    if offset >= 0 {
        anchor
            .checked_add(offset as u64)
            .ok_or_else(|| "frame overflow".to_string())
    } else {
        anchor
            .checked_sub(offset.unsigned_abs())
            .ok_or_else(|| "offset places the frame before zero".to_string())
    }
}

fn changed(item_id: Uuid) -> MutationResult {
    MutationResult {
        changed_item_ids: vec![item_id],
        ..Default::default()
    }
}

fn changed_with_deleted(item_id: Uuid, deleted: Vec<ClipSummary>) -> MutationResult {
    MutationResult {
        changed_item_ids: vec![item_id],
        deleted_addresses: deleted.iter().map(|clip| clip.address.clone()).collect(),
        deleted_presentations: deleted,
        ..Default::default()
    }
}

fn changed_after_insert(
    project: &Project,
    result: &shrimply_project_document::project::ItemAddress,
    requested_track: &shrimply_project_document::project::TrackAddress,
    deleted: Vec<ClipSummary>,
) -> Result<MutationResult, String> {
    let mut mutation = changed_with_deleted(result.item_id(), deleted);
    if result.track_id() != requested_track.track_id() {
        mutation.changed_tracks =
            crate::query::addresses_for_tracks(project, &HashSet::from([result.track_id()]))?;
    }
    Ok(mutation)
}

fn overwritten_presentations(
    project: &Project,
    track: &shrimply_project_document::project::TrackAddress,
    start: ProjectTime,
    end: ProjectTime,
    source: Uuid,
    collision: CollisionBehavior,
) -> Result<Vec<ClipSummary>, String> {
    if collision != CollisionBehavior::Overwrite {
        return Ok(Vec::new());
    }
    let item_ids = edit::collision_addresses(project, track, start, end)?
        .into_iter()
        .map(|address| address.item_id())
        .filter(|item_id| *item_id != source)
        .collect();
    crate::query::presentations_affected_by_items(project, &item_ids)
}

fn collision(value: CollisionBehavior) -> ModelCollision {
    match value {
        CollisionBehavior::Reject => ModelCollision::Reject,
        CollisionBehavior::NewTrack => ModelCollision::NewTrack,
        CollisionBehavior::Overwrite => ModelCollision::Overwrite,
    }
}
