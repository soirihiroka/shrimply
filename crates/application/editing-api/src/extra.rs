use crate::{edit::MutationResult, protocol::*, query};
use shrimply_project_document::project::{self, ItemRef, Project, ProjectItem};
use shrimply_timeline_edit::edit;
use uuid::Uuid;

pub fn settings(
    project: &mut Project,
    request: &ProjectSettingsRequest,
) -> Result<MutationResult, String> {
    if let Some(name) = &request.name {
        if name.trim().is_empty() {
            return Err("project name must not be empty".into());
        }
        project.name = name.clone();
    }
    if let Some(width) = request.width {
        project.canvas_size.width = width;
    }
    if let Some(height) = request.height {
        project.canvas_size.height = height;
    }
    if let Some(fps) = &request.fps {
        if fps.numerator <= 0 || fps.denominator <= 0 {
            return Err("frame rate must be positive".into());
        }
        let fps = project::fraction_new(fps.numerator, fps.denominator);
        if !project::COMMON_FRAME_RATES
            .iter()
            .any(|rate| rate.value == fps)
        {
            return Err("unsupported project frame rate".into());
        }
        if fps != project.fps && project.has_timeline_items() {
            return Err("frame rate can only be changed on an empty timeline".into());
        }
        project.fps = fps;
    }
    project.validate()?;
    Ok(MutationResult::default())
}

pub fn split(project: &mut Project, request: &SplitClipRequest) -> Result<MutationResult, String> {
    let address = query::model_item_address(&request.address)?;
    let time = shrimply_math_core::time_from_frame(request.frame, project.fps)
        .ok_or("frame is out of range")?;
    let (left, right) = edit::split_item_address(project, &address, time)
        .ok_or("split frame must be inside the clip")?;
    Ok(MutationResult {
        changed_item_ids: vec![left.item_id(), right.item_id()],
        ..Default::default()
    })
}

pub fn duplicate(
    project: &mut Project,
    request: &DuplicateClipRequest,
) -> Result<MutationResult, String> {
    let address = query::model_item_address(&request.address)?;
    let mut item = match project.item(&address).ok_or("clip was not found")? {
        ItemRef::Comment(item) => ProjectItem::Comment(item.clone()),
        ItemRef::Caption(item) => ProjectItem::Caption(item.clone()),
        ItemRef::Video(item) => ProjectItem::Video(Box::new(item.clone())),
        ItemRef::Audio(item) => ProjectItem::Audio(Box::new(item.clone())),
    };
    let id = Uuid::new_v4();
    match &mut item {
        ProjectItem::Comment(item) => {
            item.id = id;
            item.group_id = None;
        }
        ProjectItem::Caption(item) => {
            item.id = id;
            item.group_id = None;
        }
        ProjectItem::Video(item) => {
            item.id = id;
            item.group_id = None;
            Project::regenerate_video_property_ids(item);
        }
        ProjectItem::Audio(item) => {
            item.id = id;
            item.group_id = None;
            Project::regenerate_audio_property_ids(item);
        }
    }
    let destination = request
        .destination
        .as_ref()
        .map(query::model_track_address)
        .transpose()?
        .unwrap_or_else(|| address.track());
    if destination.kind() != address.kind()
        || !project.can_move_item_to_sequence_path(&address, destination.sequence_path())
    {
        return Err(
            "duplicate destination is incompatible or would create a recursive sequence".into(),
        );
    }
    let start = shrimply_math_core::time_from_frame(request.start_frame, project.fps)
        .ok_or("frame is out of range")?;
    let start = project
        .timeline_time_to_sequence(&destination, start)
        .ok_or("destination scope was not found")?;
    let original = project.clone();
    let before = query::all_clips(&crate::preview::saved_snapshot(
        original.clone(),
        String::new(),
    ))?
    .clips;
    let inserted = project
        .insert_item(&address.track(), item)
        .ok_or("could not insert duplicate")?;
    let collision = match request.collision {
        CollisionBehavior::Reject => edit::CollisionBehavior::Reject,
        CollisionBehavior::NewTrack => edit::CollisionBehavior::NewTrack,
        CollisionBehavior::Overwrite => edit::CollisionBehavior::Overwrite,
    };
    let result = edit::move_item(project, &inserted, Some(&destination), start, collision);
    if let Err(error) = result {
        *project = original;
        return Err(error);
    }
    let after = query::all_clips(&crate::preview::saved_snapshot(
        project.clone(),
        String::new(),
    ))?
    .clips;
    let remaining: std::collections::HashSet<_> = after.iter().map(|clip| &clip.address).collect();
    let deleted_presentations: Vec<_> = before
        .into_iter()
        .filter(|clip| !remaining.contains(&clip.address))
        .collect();
    Ok(MutationResult {
        changed_item_ids: vec![id],
        deleted_addresses: deleted_presentations
            .iter()
            .map(|clip| clip.address.clone())
            .collect(),
        deleted_presentations,
        ..Default::default()
    })
}

pub fn reorder(
    project: &mut Project,
    request: &ReorderTrackRequest,
) -> Result<MutationResult, String> {
    let address = query::model_track_address(&request.address)?;
    let scope = project.track_scope(&address).ok_or("track was not found")?;
    let sequence = project
        .sequence_id_for_scope(&scope)
        .ok_or("scope was not found")?;
    macro_rules! move_track {
        ($tracks:expr) => {{
            let tracks = $tracks;
            if request.index >= tracks.len() {
                return Err("destination track index is out of range".into());
            }
            let current = tracks
                .iter()
                .position(|track| track.id == address.track_id())
                .ok_or("track was not found")?;
            let track = tracks.remove(current);
            tracks.insert(request.index, track);
        }};
    }
    match (request.address.kind, sequence) {
        (ClipKind::Comment, None) => move_track!(&mut project.comment_tracks),
        (ClipKind::Caption, None) => move_track!(&mut project.caption_tracks),
        (ClipKind::Video, None) => move_track!(&mut project.video_tracks),
        (ClipKind::Audio, None) => move_track!(&mut project.audio_tracks),
        (ClipKind::Video, Some(id)) => move_track!(
            &mut project
                .folded_sequence_mut(id)
                .ok_or("sequence was not found")?
                .video_tracks
        ),
        (ClipKind::Audio, Some(id)) => move_track!(
            &mut project
                .folded_sequence_mut(id)
                .ok_or("sequence was not found")?
                .audio_tracks
        ),
        _ => return Err("track kind is not supported in this scope".into()),
    }
    Ok(MutationResult {
        changed_tracks: vec![request.address.clone()],
        ..Default::default()
    })
}
