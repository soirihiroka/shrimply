use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use shrimply_asset::{Asset, AssetSnapshot};
use shrimply_math_core::{Time, time_from_frame};
use shrimply_mcp::edit::MutationResult;
use shrimply_mcp::protocol::{
    ClipKind, CollisionBehavior, EditOperation, EditOperationResult, EditRequest,
    GenerateTtsRequest, ImportEntry, InitialClipProperties, InsertTtsRequest, ScopeRef,
    SetClipPropertiesRequest,
};
use shrimply_project::project::{
    AudioItem, AudioSource, ItemKind, Project, ProjectItem, SequenceScopeId,
    TrackAddress as ModelTrackAddress,
};
use shrimply_timeline::TrackKind;
use shrimply_timeline::edit as timeline_edit;
use uuid::Uuid;

use crate::timeline::import as native;

pub struct PreparedEdit {
    pub project: Project,
    mutations: Vec<MutationResult>,
    operations: Vec<&'static str>,
    staged: Vec<StagedDirectory>,
    linked_sources: Vec<AssetSnapshot>,
}

#[derive(Clone)]
struct ResolvedScope {
    logical: SequenceScopeId,
    concrete_path: Option<Vec<Uuid>>,
}

#[derive(Clone, Copy)]
struct ImportStarts {
    video: Option<Time>,
    audio: Option<Time>,
}

impl PreparedEdit {
    pub fn promote(&mut self) -> Result<(), String> {
        for index in 0..self.staged.len() {
            if let Err(error) = self.staged[index].promote() {
                for staged in self.staged[..index].iter_mut().rev() {
                    staged.rollback();
                }
                return Err(error);
            }
        }
        Ok(())
    }

    pub fn discard_promoted(&mut self) {
        for staged in self.staged.iter_mut().rev() {
            if staged.promoted {
                staged.rollback();
            }
        }
    }

    pub fn ensure_linked_sources_current(&self) -> Result<(), String> {
        for source in &self.linked_sources {
            source.verify_current()?;
        }
        Ok(())
    }

    pub fn results(&self) -> Result<Vec<EditOperationResult>, String> {
        self.mutations
            .iter()
            .zip(&self.operations)
            .enumerate()
            .map(|(index, (mutation, operation))| {
                let changed_presentations = shrimply_mcp::query::presentations_affected_by_items(
                    &self.project,
                    &mutation.changed_item_ids.iter().copied().collect(),
                )?;
                let mut changed_addresses = changed_presentations
                    .iter()
                    .map(|clip| clip.address.clone())
                    .collect::<Vec<_>>();
                changed_addresses.sort_by_key(|address| {
                    (
                        address.kind as u8,
                        address.sequence_path.clone(),
                        address.track_id.clone(),
                        address.item_id.clone(),
                    )
                });
                changed_addresses.dedup();
                let mut presentations = changed_presentations;
                presentations.extend(mutation.deleted_presentations.clone());
                Ok(EditOperationResult {
                    index,
                    operation: (*operation).to_string(),
                    changed_addresses,
                    deleted_addresses: mutation.deleted_addresses.clone(),
                    changed_tracks: mutation.changed_tracks.clone(),
                    presentations,
                })
            })
            .collect()
    }
}

pub fn prepare(
    project: Project,
    request: &EditRequest,
    playhead_frame: u64,
    active_scope: SequenceScopeId,
    default_visual_duration: Time,
    project_path: &Path,
) -> Result<PreparedEdit, String> {
    if request.operations.is_empty() {
        return Err("edit request requires at least one operation".to_string());
    }
    let anchor = request.frame.unwrap_or(playhead_frame);
    let script_scope = request
        .scope
        .as_ref()
        .map(|scope| resolve_scope(&project, scope))
        .transpose()?
        .unwrap_or_else(|| ResolvedScope {
            concrete_path: active_scope.is_root().then(Vec::new),
            logical: active_scope,
        });
    let mut prepared = PreparedEdit {
        project,
        mutations: Vec::new(),
        operations: Vec::new(),
        staged: Vec::new(),
        linked_sources: Vec::new(),
    };
    for (index, operation) in request.operations.iter().enumerate() {
        let result = match operation {
            EditOperation::InsertFiles(import) => apply_import(
                &mut prepared,
                import,
                anchor,
                &script_scope,
                default_visual_duration,
                project_path,
            ),
            EditOperation::InsertTts(request) => apply_tts(
                &mut prepared.project,
                request,
                anchor,
                &script_scope,
                default_visual_duration,
            ),
            _ => shrimply_mcp::edit::apply_non_import(
                &mut prepared.project,
                operation,
                anchor,
                &script_scope.logical,
            ),
        };
        prepared.mutations.push(result.map_err(|error| {
            format!(
                "operation {index} ({}) failed: {error}",
                operation_name(operation)
            )
        })?);
        prepared.operations.push(operation_name(operation));
    }
    prepared
        .project
        .validate()
        .map_err(|error| format!("edited project is invalid: {error}"))?;
    prepared.ensure_linked_sources_current()?;
    Ok(prepared)
}

fn apply_tts(
    project: &mut Project,
    request: &InsertTtsRequest,
    script_anchor: u64,
    script_scope: &ResolvedScope,
    default_duration: Time,
) -> Result<MutationResult, String> {
    let duration = request
        .duration_frames
        .map(|frames| {
            if frames == 0 {
                return Err("duration_frames must be positive".to_string());
            }
            time_from_frame(frames, project.fps)
                .ok_or_else(|| "TTS duration exceeds the supported exact range".to_string())
        })
        .transpose()?
        .unwrap_or(default_duration)
        .snapped(project.frame_step());
    if duration <= Time::ZERO {
        return Err("TTS duration must span at least one project frame".to_string());
    }
    let scope = request
        .scope
        .as_ref()
        .map(|scope| resolve_scope(project, scope))
        .transpose()?
        .unwrap_or_else(|| script_scope.clone());
    insert_tts_item(
        project,
        TtsInsertion {
            track: request.track.as_ref(),
            scope: &scope,
            frame: request.frame.unwrap_or(script_anchor),
            duration,
            source_duration: Time::ZERO,
            settings: shrimply_tts::TtsSettings::default(),
            path: PathBuf::new(),
            collision: request.collision,
        },
    )
}

pub fn insert_generated_tts(
    project: &mut Project,
    request: &GenerateTtsRequest,
    playhead_frame: u64,
    active_scope: SequenceScopeId,
    duration: Time,
    settings: shrimply_tts::TtsSettings,
    path: PathBuf,
) -> Result<MutationResult, String> {
    if duration <= Time::ZERO {
        return Err("generated speech has no valid duration".to_string());
    }
    let scope = request
        .scope
        .as_ref()
        .map(|scope| resolve_scope(project, scope))
        .transpose()?
        .unwrap_or_else(|| ResolvedScope {
            concrete_path: active_scope.is_root().then(Vec::new),
            logical: active_scope,
        });
    insert_tts_item(
        project,
        TtsInsertion {
            track: request.track.as_ref(),
            scope: &scope,
            frame: request.frame.unwrap_or(playhead_frame),
            duration,
            source_duration: duration,
            settings,
            path,
            collision: request.collision,
        },
    )
}

struct TtsInsertion<'a> {
    track: Option<&'a shrimply_mcp::protocol::TrackAddress>,
    scope: &'a ResolvedScope,
    frame: u64,
    duration: Time,
    source_duration: Time,
    settings: shrimply_tts::TtsSettings,
    path: PathBuf,
    collision: CollisionBehavior,
}

fn insert_tts_item(
    project: &mut Project,
    insertion: TtsInsertion<'_>,
) -> Result<MutationResult, String> {
    let TtsInsertion {
        track,
        scope,
        frame,
        duration,
        source_duration,
        settings,
        path,
        collision,
    } = insertion;
    let target = track
        .map(shrimply_mcp::query::model_track_address)
        .transpose()?;
    if target
        .as_ref()
        .is_some_and(|target| target.kind() != ItemKind::Audio)
    {
        return Err("TTS insertion requires an audio track".to_string());
    }
    if let Some(target) = &target {
        if project.track(target).is_none() {
            return Err("TTS target track was not found".to_string());
        }
        if project.track_scope(target).as_ref() != Some(&scope.logical) {
            return Err("TTS target track is outside the requested scope".to_string());
        }
    }
    if target.is_none() && collision == CollisionBehavior::Overwrite {
        return Err("overwrite TTS insertion requires an explicit audio track".to_string());
    }
    let projected = time_from_frame(frame, project.fps)
        .ok_or_else(|| "TTS frame exceeds the supported exact range".to_string())?;
    let path_for_time = if let Some(target) = &target {
        target.sequence_path().to_vec()
    } else if let Some(path) = &scope.concrete_path
        && project
            .sequence_scope_for_path(ItemKind::Audio, path)
            .as_ref()
            == Some(&scope.logical)
    {
        path.clone()
    } else {
        project
            .sequence_path_for_scope(ItemKind::Audio, &scope.logical)
            .ok_or_else(|| {
                "TTS scope has no unique concrete presentation; provide an audio track".to_string()
            })?
    };
    let start = project
        .timeline_time_to_sequence_path(ItemKind::Audio, &path_for_time, projected)
        .ok_or_else(|| "TTS scope cannot map the projected frame".to_string())?
        .snapped(project.frame_step());
    let end = start.saturating_add(duration).snapped(project.frame_step());
    if end <= start {
        return Err("TTS item must span at least one project frame".to_string());
    }

    let original = project.clone();
    let original_tracks = track_ids(project);
    let target_id = target.as_ref().map(ModelTrackAddress::track_id);
    let (item_id, overwritten) = with_scope_tracks(project, &scope.logical, |project| {
        let mut index = target_id
            .map(|id| {
                project
                    .audio_tracks
                    .iter()
                    .position(|track| track.id == id)
                    .ok_or_else(|| "TTS target track was not found in its scope".to_string())
            })
            .transpose()?;
        if let Some(current) = index
            && collision == CollisionBehavior::NewTrack
            && timeline_edit::track_collides(
                project,
                &root_track(project, ClipKind::Audio, current),
                start,
                end,
            )?
        {
            index = Some(tracks_with_room(project, ClipKind::Audio, 1, start, end, true)?[0]);
        }
        let index = match index {
            Some(index) => index,
            None => tracks_with_room(
                project,
                ClipKind::Audio,
                1,
                start,
                end,
                collision == CollisionBehavior::NewTrack,
            )?[0],
        };
        let track = root_track(project, ClipKind::Audio, index);
        let collisions = timeline_edit::collision_addresses(project, &track, start, end)?;
        if collision == CollisionBehavior::Reject && !collisions.is_empty() {
            return Err("TTS insertion collides with an existing clip".to_string());
        }
        let overwritten = collisions
            .iter()
            .map(shrimply_project::project::ItemAddress::item_id)
            .collect::<HashSet<_>>();
        if collision == CollisionBehavior::Overwrite {
            timeline_edit::overwrite_interval(project, &track, start, end)?;
        }
        let item = AudioItem::builder(start, end)
            .source_duration(source_duration)
            .source(AudioSource::Tts(Box::new(settings)))
            .file(path)
            .build();
        let item_id = item.id;
        project
            .insert_item(&track, ProjectItem::Audio(Box::new(item)))
            .expect("validated audio track must accept a TTS item");
        Ok((item_id, overwritten))
    })?;
    let deleted_presentations =
        shrimply_mcp::query::presentations_affected_by_items(&original, &overwritten)?;
    let created_track_ids = track_ids(project)
        .difference(&original_tracks)
        .copied()
        .collect();
    Ok(MutationResult {
        changed_item_ids: vec![item_id],
        deleted_addresses: deleted_presentations
            .iter()
            .map(|clip| clip.address.clone())
            .collect(),
        deleted_presentations,
        changed_tracks: shrimply_mcp::query::addresses_for_tracks(project, &created_track_ids)?,
    })
}

fn apply_import(
    prepared: &mut PreparedEdit,
    request: &shrimply_mcp::protocol::InsertFilesRequest,
    script_anchor: u64,
    script_scope: &ResolvedScope,
    default_visual_duration: Time,
    project_path: &Path,
) -> Result<MutationResult, String> {
    let PreparedEdit {
        project,
        staged,
        linked_sources,
        ..
    } = prepared;
    if request.files.is_empty() {
        return Err("insert_files requires at least one source file".to_string());
    }
    if request.link && request.copy_root.is_some() {
        return Err("link mode forbids copy_root".to_string());
    }
    let scope = request
        .scope
        .as_ref()
        .map(|scope| resolve_scope(project, scope))
        .transpose()?
        .unwrap_or_else(|| script_scope.clone());
    validate_targets(project, &request.files, &scope.logical)?;
    let anchor = request.frame.unwrap_or(script_anchor);
    let (sources, staging) = stage_sources(
        &request.files,
        request.link,
        request.copy_root.as_deref(),
        project_path,
    )?;
    if let Some(staging) = staging {
        staged.push(staging);
    }
    if !scope.logical.is_root()
        && sources
            .iter()
            .any(|source| native::file_kind(&source.inspect) == Some(native::FileKind::Vtt))
    {
        return Err("VTT imports are only supported in the root scope".to_string());
    }
    let original_project = project.clone();
    let original_track_ids = track_ids(project);
    let mut changed_item_ids = Vec::new();
    let mut overwritten_item_ids = HashSet::new();
    with_scope_tracks(project, &scope.logical, |project| {
        for (entry, mut source) in request.files.iter().zip(sources) {
            if request.link {
                linked_sources.push(Asset::new(source.inspect.clone()).snapshot()?);
            }
            let kind = native::file_kind(&source.inspect)
                .ok_or_else(|| format!("unsupported file type: {}", source.inspect.display()))?;
            if matches!(kind, native::FileKind::Mkv | native::FileKind::WebM) {
                if request.link {
                    return Err(
                        "MKV/WebM requires an MP4 derivative and cannot be linked".to_string()
                    );
                }
                source.inspect = native::remux_mkv_to_mp4(&source.inspect)?;
                let name = source
                    .inspect
                    .file_name()
                    .ok_or_else(|| "remux output has no file name".to_string())?;
                source.stored.set_file_name(name);
            }
            let frame = shrimply_mcp::edit::frame_with_offset(anchor, entry.offset_frames)
                .map_err(|error| format!("import {error}"))?;
            let projected_start = time_from_frame(frame, project.fps)
                .ok_or_else(|| "import frame exceeds the supported exact range".to_string())?;
            let before = item_ids(project);
            if kind == native::FileKind::Vtt {
                overwritten_item_ids.extend(apply_vtt(
                    project,
                    &source.inspect,
                    entry,
                    projected_start,
                    request.collision,
                )?);
            } else {
                let mut info =
                    native::inspect(source.inspect, project.canvas_size, default_visual_duration)?;
                info.source = Asset::new(source.stored);
                let starts = ImportStarts {
                    video: import_start(
                        &original_project,
                        &scope,
                        entry,
                        ClipKind::Video,
                        projected_start,
                        info.video_streams > 0,
                    )?,
                    audio: import_start(
                        &original_project,
                        &scope,
                        entry,
                        ClipKind::Audio,
                        projected_start,
                        info.audio_streams > 0,
                    )?,
                };
                overwritten_item_ids.extend(apply_media(
                    project,
                    &info,
                    entry,
                    starts,
                    request.collision,
                )?);
            }
            let new_ids = item_ids(project)
                .difference(&before)
                .copied()
                .collect::<HashSet<_>>();
            apply_initial_properties(project, &new_ids, &entry.properties)?;
            changed_item_ids.extend(new_ids);
        }
        Ok(())
    })?;
    let deleted_presentations = shrimply_mcp::query::presentations_affected_by_items(
        &original_project,
        &overwritten_item_ids,
    )?;
    let created_track_ids = track_ids(project)
        .difference(&original_track_ids)
        .copied()
        .collect();
    Ok(MutationResult {
        changed_item_ids,
        deleted_addresses: deleted_presentations
            .iter()
            .map(|clip| clip.address.clone())
            .collect(),
        deleted_presentations,
        changed_tracks: shrimply_mcp::query::addresses_for_tracks(project, &created_track_ids)?,
    })
}

fn apply_media(
    project: &mut Project,
    info: &native::MediaInfo,
    entry: &ImportEntry,
    starts: ImportStarts,
    collision: CollisionBehavior,
) -> Result<HashSet<Uuid>, String> {
    if entry
        .targets
        .iter()
        .any(|target| target.kind == ClipKind::Caption)
    {
        return Err("only VTT files can target caption tracks".to_string());
    }
    if entry.targets.is_empty() {
        match collision {
            CollisionBehavior::Reject => apply_media_automatically(project, info, starts, false)?,
            CollisionBehavior::NewTrack => apply_media_automatically(project, info, starts, true)?,
            CollisionBehavior::Overwrite => {
                return Err("overwrite imports require explicit target tracks".to_string());
            }
        }
        return Ok(HashSet::new());
    }
    let mut overwritten = HashSet::new();
    for kind in [ClipKind::Video, ClipKind::Audio] {
        let mut indices = target_indices(project, entry, kind)?;
        if indices.is_empty() {
            continue;
        }
        let track_kind = track_kind(kind)?;
        let start = match kind {
            ClipKind::Video => starts.video,
            ClipKind::Audio => starts.audio,
            ClipKind::Caption => None,
        }
        .ok_or_else(|| "import has no concrete presentation path for its target".to_string())?;
        let end = start.saturating_add(info.duration.max(Time::from_nanos(1)));
        let collides = indices
            .iter()
            .copied()
            .map(|index| root_track(project, kind, index))
            .map(|track| timeline_edit::track_collides(project, &track, start, end))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .any(|value| value);
        match collision {
            CollisionBehavior::Reject => {}
            CollisionBehavior::NewTrack if collides => {
                indices = tracks_with_room(project, kind, indices.len(), start, end, true)?;
            }
            CollisionBehavior::Overwrite => {
                for index in &indices {
                    let track = root_track(project, kind, *index);
                    overwritten.extend(
                        timeline_edit::collision_addresses(project, &track, start, end)?
                            .into_iter()
                            .map(|address| address.item_id()),
                    );
                    timeline_edit::overwrite_interval(project, &track, start, end)?;
                }
            }
            CollisionBehavior::NewTrack => {}
        }
        native::apply_media_to_tracks(project, info, track_kind, &indices, start)?;
    }
    Ok(overwritten)
}

fn apply_media_automatically(
    project: &mut Project,
    info: &native::MediaInfo,
    starts: ImportStarts,
    create: bool,
) -> Result<(), String> {
    if info.video_streams > 0 {
        let start = starts.video.expect("video import start was resolved");
        let end = start.saturating_add(info.duration.max(Time::from_nanos(1)));
        let indices = tracks_with_room(
            project,
            ClipKind::Video,
            info.video_streams,
            start,
            end,
            create,
        )?;
        native::apply_media_to_tracks(project, info, TrackKind::Video, &indices, start)?;
    }
    if info.audio_streams > 0 {
        let start = starts.audio.expect("audio import start was resolved");
        let end = start.saturating_add(info.duration.max(Time::from_nanos(1)));
        let indices = tracks_with_room(
            project,
            ClipKind::Audio,
            info.audio_streams,
            start,
            end,
            create,
        )?;
        native::apply_media_to_tracks(project, info, TrackKind::Audio, &indices, start)?;
    }
    Ok(())
}

fn apply_vtt(
    project: &mut Project,
    path: &Path,
    entry: &ImportEntry,
    start: Time,
    collision: CollisionBehavior,
) -> Result<HashSet<Uuid>, String> {
    if entry
        .targets
        .iter()
        .any(|target| target.kind != ClipKind::Caption)
    {
        return Err("VTT files can target only caption tracks".to_string());
    }
    let ranges = native::vtt_ranges(path)?;
    let mut indices = if entry.targets.is_empty() {
        match collision {
            CollisionBehavior::Reject => {
                caption_tracks_with_room(project, &ranges, start, 1, false)?
            }
            CollisionBehavior::NewTrack => {
                caption_tracks_with_room(project, &ranges, start, 1, true)?
            }
            CollisionBehavior::Overwrite => {
                return Err("overwrite imports require explicit target tracks".to_string());
            }
        }
    } else {
        target_indices(project, entry, ClipKind::Caption)?
    };
    let mut collides = false;
    for index in &indices {
        for (cue_start, cue_end) in &ranges {
            collides |= timeline_edit::track_collides(
                project,
                &root_track(project, ClipKind::Caption, *index),
                start.saturating_add(*cue_start),
                start.saturating_add(*cue_end),
            )?;
        }
    }
    let mut overwritten = HashSet::new();
    match collision {
        CollisionBehavior::Reject => {}
        CollisionBehavior::NewTrack if collides => {
            indices = caption_tracks_with_room(project, &ranges, start, indices.len(), true)?;
        }
        CollisionBehavior::Overwrite => {
            for index in &indices {
                for (cue_start, cue_end) in &ranges {
                    let track = root_track(project, ClipKind::Caption, *index);
                    let start = start.saturating_add(*cue_start);
                    let end = start.saturating_add(cue_end.saturating_sub(*cue_start));
                    overwritten.extend(
                        timeline_edit::collision_addresses(project, &track, start, end)?
                            .into_iter()
                            .map(|address| address.item_id()),
                    );
                    timeline_edit::overwrite_interval(project, &track, start, end)?;
                }
            }
        }
        CollisionBehavior::NewTrack => {}
    }
    native::apply_vtt_to_tracks(project, path, &indices, start)?;
    Ok(overwritten)
}

fn target_indices(
    project: &Project,
    entry: &ImportEntry,
    kind: ClipKind,
) -> Result<Vec<usize>, String> {
    entry
        .targets
        .iter()
        .filter(|target| target.kind == kind)
        .map(|target| {
            let id = Uuid::parse_str(&target.track_id)
                .map_err(|error| format!("invalid target track ID: {error}"))?;
            let index = match kind {
                ClipKind::Caption => project
                    .caption_tracks
                    .iter()
                    .position(|track| track.id == id),
                ClipKind::Video => project.video_tracks.iter().position(|track| track.id == id),
                ClipKind::Audio => project.audio_tracks.iter().position(|track| track.id == id),
            };
            index.ok_or_else(|| format!("target track {} was not found", target.track_id))
        })
        .collect()
}

fn append_tracks(project: &mut Project, kind: ClipKind, count: usize) -> Vec<usize> {
    let start = match kind {
        ClipKind::Caption => {
            let start = project.caption_tracks.len();
            project
                .caption_tracks
                .resize_with(start + count, Default::default);
            start
        }
        ClipKind::Video => {
            let start = project.video_tracks.len();
            project
                .video_tracks
                .resize_with(start + count, Default::default);
            start
        }
        ClipKind::Audio => {
            let start = project.audio_tracks.len();
            project
                .audio_tracks
                .resize_with(start + count, Default::default);
            start
        }
    };
    (start..start + count).collect()
}

fn tracks_with_room(
    project: &mut Project,
    kind: ClipKind,
    count: usize,
    start: Time,
    end: Time,
    create: bool,
) -> Result<Vec<usize>, String> {
    let track_count = match kind {
        ClipKind::Caption => project.caption_tracks.len(),
        ClipKind::Video => project.video_tracks.len(),
        ClipKind::Audio => project.audio_tracks.len(),
    };
    let mut tracks = Vec::new();
    for index in 0..track_count {
        if !timeline_edit::track_collides(project, &root_track(project, kind, index), start, end)? {
            tracks.push(index);
            if tracks.len() == count {
                return Ok(tracks);
            }
        }
    }
    if create {
        tracks.extend(append_tracks(project, kind, count - tracks.len()));
        Ok(tracks)
    } else {
        Err(format!(
            "no {kind:?} track has room for this insertion; choose a target, create a track, or use collision=new_track"
        ))
    }
}

fn caption_tracks_with_room(
    project: &mut Project,
    ranges: &[(Time, Time)],
    start: Time,
    count: usize,
    create: bool,
) -> Result<Vec<usize>, String> {
    let mut tracks = Vec::new();
    for index in 0..project.caption_tracks.len() {
        let track = root_track(project, ClipKind::Caption, index);
        let mut collides = false;
        for (cue_start, cue_end) in ranges {
            collides |= timeline_edit::track_collides(
                project,
                &track,
                start.saturating_add(*cue_start),
                start.saturating_add(*cue_end),
            )?;
        }
        if !collides {
            tracks.push(index);
            if tracks.len() == count {
                return Ok(tracks);
            }
        }
    }
    if create {
        tracks.extend(append_tracks(
            project,
            ClipKind::Caption,
            count - tracks.len(),
        ));
        Ok(tracks)
    } else {
        Err("no Caption track has room for this import; choose a target, create a track, or use collision=new_track".to_string())
    }
}

fn root_track(project: &Project, kind: ClipKind, index: usize) -> ModelTrackAddress {
    match kind {
        ClipKind::Caption => ModelTrackAddress::Caption {
            track_id: project.caption_tracks[index].id,
        },
        ClipKind::Video => ModelTrackAddress::Video {
            sequence_path: Vec::new(),
            track_id: project.video_tracks[index].id,
        },
        ClipKind::Audio => ModelTrackAddress::Audio {
            sequence_path: Vec::new(),
            track_id: project.audio_tracks[index].id,
        },
    }
}

fn track_kind(kind: ClipKind) -> Result<TrackKind, String> {
    match kind {
        ClipKind::Caption => Err("only VTT files can target caption tracks".to_string()),
        ClipKind::Video => Ok(TrackKind::Video),
        ClipKind::Audio => Ok(TrackKind::Audio),
    }
}

fn apply_initial_properties(
    project: &mut Project,
    item_ids: &HashSet<Uuid>,
    properties: &InitialClipProperties,
) -> Result<(), String> {
    let addresses = root_item_addresses(project, item_ids);
    let has_caption = addresses
        .iter()
        .any(|address| address.kind() == ItemKind::Caption);
    let has_audio = addresses
        .iter()
        .any(|address| address.kind() == ItemKind::Audio);
    let has_playback = addresses
        .iter()
        .any(|address| address.kind() != ItemKind::Caption);
    if properties.text.is_some() && !has_caption {
        return Err("initial text requires an imported caption clip".to_string());
    }
    if (properties.enabled.is_some() || properties.gain_db.is_some()) && !has_audio {
        return Err("initial audio properties require an imported audio clip".to_string());
    }
    if (properties.playback_speed.is_some() || properties.repeat_strategy.is_some())
        && !has_playback
    {
        return Err("initial playback properties require visual or audio clips".to_string());
    }
    for address in addresses {
        let kind = address.kind();
        let request = SetClipPropertiesRequest {
            address: shrimply_mcp::query::protocol_item_address(&address),
            text: (kind == ItemKind::Caption)
                .then(|| properties.text.clone())
                .flatten(),
            enabled: (kind == ItemKind::Audio)
                .then_some(properties.enabled)
                .flatten(),
            gain_db: (kind == ItemKind::Audio)
                .then_some(properties.gain_db)
                .flatten(),
            playback_speed: (kind != ItemKind::Caption)
                .then(|| properties.playback_speed.clone())
                .flatten(),
            repeat_strategy: (kind != ItemKind::Caption)
                .then(|| properties.repeat_strategy.clone())
                .flatten(),
        };
        if request.text.is_none()
            && request.enabled.is_none()
            && request.gain_db.is_none()
            && request.playback_speed.is_none()
            && request.repeat_strategy.is_none()
        {
            continue;
        }
        let operation = EditOperation::SetClipProperties(request);
        shrimply_mcp::edit::apply_non_import(project, &operation, 0, &SequenceScopeId::root())?;
    }
    Ok(())
}

fn root_item_addresses(
    project: &Project,
    item_ids: &HashSet<Uuid>,
) -> Vec<shrimply_project::project::ItemAddress> {
    project
        .caption_tracks
        .iter()
        .flat_map(|track| {
            track
                .items
                .iter()
                .filter(|item| item_ids.contains(&item.id))
                .map(|item| ModelTrackAddress::Caption { track_id: track.id }.item(item.id))
        })
        .chain(project.video_tracks.iter().flat_map(|track| {
            track
                .items
                .iter()
                .filter(|item| item_ids.contains(&item.id))
                .map(|item| {
                    ModelTrackAddress::Video {
                        sequence_path: Vec::new(),
                        track_id: track.id,
                    }
                    .item(item.id)
                })
        }))
        .chain(project.audio_tracks.iter().flat_map(|track| {
            track
                .items
                .iter()
                .filter(|item| item_ids.contains(&item.id))
                .map(|item| {
                    ModelTrackAddress::Audio {
                        sequence_path: Vec::new(),
                        track_id: track.id,
                    }
                    .item(item.id)
                })
        }))
        .collect()
}

fn item_ids(project: &Project) -> HashSet<Uuid> {
    project
        .caption_tracks
        .iter()
        .flat_map(|track| &track.items)
        .map(|item| item.id)
        .chain(
            project
                .video_tracks
                .iter()
                .chain(
                    project
                        .folded_sequences
                        .iter()
                        .flat_map(|sequence| &sequence.video_tracks),
                )
                .flat_map(|track| &track.items)
                .map(|item| item.id),
        )
        .chain(
            project
                .audio_tracks
                .iter()
                .chain(
                    project
                        .folded_sequences
                        .iter()
                        .flat_map(|sequence| &sequence.audio_tracks),
                )
                .flat_map(|track| &track.items)
                .map(|item| item.id),
        )
        .collect()
}

fn track_ids(project: &Project) -> HashSet<Uuid> {
    project
        .caption_tracks
        .iter()
        .map(|track| track.id)
        .chain(
            project
                .video_tracks
                .iter()
                .chain(
                    project
                        .folded_sequences
                        .iter()
                        .flat_map(|sequence| &sequence.video_tracks),
                )
                .map(|track| track.id),
        )
        .chain(
            project
                .audio_tracks
                .iter()
                .chain(
                    project
                        .folded_sequences
                        .iter()
                        .flat_map(|sequence| &sequence.audio_tracks),
                )
                .map(|track| track.id),
        )
        .collect()
}

fn validate_targets(
    project: &Project,
    entries: &[ImportEntry],
    scope: &SequenceScopeId,
) -> Result<(), String> {
    for target in entries.iter().flat_map(|entry| &entry.targets) {
        let address = shrimply_mcp::query::model_track_address(target)?;
        if project.track(&address).is_none() {
            return Err(format!("target track {} was not found", target.track_id));
        }
        if project.track_scope(&address).as_ref() != Some(scope) {
            return Err(format!(
                "target track {} is outside the import scope",
                target.track_id
            ));
        }
    }
    Ok(())
}

fn import_start(
    project: &Project,
    scope: &ResolvedScope,
    entry: &ImportEntry,
    kind: ClipKind,
    projected: Time,
    source_has_kind: bool,
) -> Result<Option<Time>, String> {
    if !source_has_kind
        || (!entry.targets.is_empty() && !entry.targets.iter().any(|target| target.kind == kind))
    {
        return Ok(None);
    }
    let item_kind = match kind {
        ClipKind::Caption => ItemKind::Caption,
        ClipKind::Video => ItemKind::Video,
        ClipKind::Audio => ItemKind::Audio,
    };
    let target_paths = entry
        .targets
        .iter()
        .filter(|target| target.kind == kind)
        .map(|target| shrimply_mcp::query::parse_path(&target.sequence_path))
        .collect::<Result<HashSet<_>, _>>()?;
    if target_paths.len() > 1 {
        return Err(format!(
            "{kind:?} import targets must use one concrete sequence presentation"
        ));
    }
    let path = if let Some(path) = target_paths.into_iter().next() {
        path
    } else if let Some(path) = &scope.concrete_path
        && project.sequence_scope_for_path(item_kind, path).as_ref() == Some(&scope.logical)
    {
        path.clone()
    } else {
        project
            .sequence_path_for_scope(item_kind, &scope.logical)
            .ok_or_else(|| {
                format!(
                    "{kind:?} import scope has no unique concrete presentation; provide a concrete target track"
                )
            })?
    };
    project
        .timeline_time_to_sequence_path(item_kind, &path, projected)
        .map(|time| time.snapped(project.frame_step()))
        .map(Some)
        .ok_or_else(|| format!("{kind:?} import scope cannot map the projected anchor frame"))
}

fn resolve_scope(project: &Project, scope: &ScopeRef) -> Result<ResolvedScope, String> {
    let path = shrimply_mcp::query::parse_path(&scope.sequence_path)?;
    if path.is_empty() {
        return Ok(ResolvedScope {
            logical: SequenceScopeId::root(),
            concrete_path: Some(path),
        });
    }
    let video = project.sequence_scope_for_path(ItemKind::Video, &path);
    let audio = project.sequence_scope_for_path(ItemKind::Audio, &path);
    let logical = match (video, audio) {
        (Some(video), Some(audio)) if video != audio => {
            return Err(
                "scope path ambiguously names different video and audio sequences".to_string(),
            );
        }
        (Some(scope), _) | (_, Some(scope)) => scope,
        (None, None) => {
            return Err("scope does not resolve to a folded-sequence presentation".to_string());
        }
    };
    Ok(ResolvedScope {
        logical,
        concrete_path: Some(path),
    })
}

fn with_scope_tracks<T>(
    project: &mut Project,
    scope: &SequenceScopeId,
    apply: impl FnOnce(&mut Project) -> Result<T, String>,
) -> Result<T, String> {
    let Some(sequence_id) = project
        .sequence_id_for_scope(scope)
        .ok_or_else(|| "scope does not resolve to a sequence definition".to_string())?
    else {
        return apply(project);
    };
    let (video_tracks, audio_tracks) = {
        let sequence = project
            .folded_sequence_mut(sequence_id)
            .expect("resolved sequence must exist");
        (
            std::mem::take(&mut sequence.video_tracks),
            std::mem::take(&mut sequence.audio_tracks),
        )
    };
    let root_video = std::mem::replace(&mut project.video_tracks, video_tracks);
    let root_audio = std::mem::replace(&mut project.audio_tracks, audio_tracks);
    let result = apply(project);
    let video_tracks = std::mem::replace(&mut project.video_tracks, root_video);
    let audio_tracks = std::mem::replace(&mut project.audio_tracks, root_audio);
    let sequence = project
        .folded_sequence_mut(sequence_id)
        .expect("resolved sequence must still exist");
    sequence.video_tracks = video_tracks;
    sequence.audio_tracks = audio_tracks;
    result
}

struct SourcePaths {
    inspect: PathBuf,
    stored: PathBuf,
}

struct StagedDirectory {
    staging: PathBuf,
    final_path: PathBuf,
    promoted: bool,
}

impl StagedDirectory {
    fn promote(&mut self) -> Result<(), String> {
        fs::rename(&self.staging, &self.final_path).map_err(|error| {
            format!(
                "could not promote imported media {}: {error}",
                self.final_path.display()
            )
        })?;
        self.promoted = true;
        Ok(())
    }

    fn rollback(&mut self) {
        fs::rename(&self.final_path, &self.staging).unwrap_or_else(|error| {
            panic!(
                "could not roll back imported media {}: {error}",
                self.final_path.display()
            )
        });
        self.promoted = false;
    }
}

impl Drop for StagedDirectory {
    fn drop(&mut self) {
        if !self.promoted && self.staging.exists() {
            fs::remove_dir_all(&self.staging).unwrap_or_else(|error| {
                panic!(
                    "could not clean staged MCP import {}: {error}",
                    self.staging.display()
                )
            });
        }
    }
}

fn stage_sources(
    entries: &[ImportEntry],
    link: bool,
    copy_root: Option<&str>,
    project_path: &Path,
) -> Result<(Vec<SourcePaths>, Option<StagedDirectory>), String> {
    let sources = entries
        .iter()
        .map(|entry| {
            Path::new(&entry.source)
                .canonicalize()
                .map_err(|error| format!("could not resolve {}: {error}", entry.source))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if link {
        return Ok((
            sources
                .into_iter()
                .map(|source| SourcePaths {
                    inspect: source.clone(),
                    stored: source,
                })
                .collect(),
            None,
        ));
    }
    let imported = project_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("media/imported");
    fs::create_dir_all(&imported)
        .map_err(|error| format!("could not create {}: {error}", imported.display()))?;
    let imported = imported
        .canonicalize()
        .map_err(|error| format!("could not resolve {}: {error}", imported.display()))?;
    let id = Uuid::new_v4().to_string();
    let final_path = imported.join(&id);
    let staging = imported.join(format!(".{id}.staging"));
    if final_path.exists() || staging.exists() {
        return Err("generated import directory already exists".to_string());
    }
    if let Some(root) = copy_root {
        let root = Path::new(root)
            .canonicalize()
            .map_err(|error| format!("could not resolve copy_root: {error}"))?;
        if final_path.starts_with(&root) {
            return Err(format!(
                "copy_root {} contains the project import destination",
                root.display()
            ));
        }
        for source in &sources {
            if !source.starts_with(&root) {
                return Err(format!(
                    "{} escapes copy_root {}",
                    source.display(),
                    root.display()
                ));
            }
        }
        fs::create_dir(&staging)
            .map_err(|error| format!("could not create {}: {error}", staging.display()))?;
        let guard = StagedDirectory {
            staging: staging.clone(),
            final_path: final_path.clone(),
            promoted: false,
        };
        copy_tree(&root, &staging)?;
        let mapped = sources
            .into_iter()
            .map(|source| {
                let relative = source
                    .strip_prefix(&root)
                    .expect("source was checked under copy_root");
                SourcePaths {
                    inspect: staging.join(relative),
                    stored: final_path.join(relative),
                }
            })
            .collect();
        return Ok((mapped, Some(guard)));
    }
    fs::create_dir(&staging)
        .map_err(|error| format!("could not create {}: {error}", staging.display()))?;
    let guard = StagedDirectory {
        staging: staging.clone(),
        final_path: final_path.clone(),
        promoted: false,
    };
    let mapped = sources
        .into_iter()
        .enumerate()
        .map(|(index, source)| {
            let name = sanitized_name(
                source
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("import"),
            );
            let name = format!("{index}-{name}");
            let inspect = staging.join(&name);
            fs::copy(&source, &inspect)
                .map_err(|error| format!("could not copy {}: {error}", source.display()))?;
            Ok(SourcePaths {
                inspect,
                stored: final_path.join(name),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok((mapped, Some(guard)))
}

fn copy_tree(root: &Path, destination: &Path) -> Result<(), String> {
    for entry in
        fs::read_dir(root).map_err(|error| format!("could not read {}: {error}", root.display()))?
    {
        let entry = entry.map_err(|error| format!("could not read directory entry: {error}"))?;
        let source = entry.path();
        let metadata = fs::symlink_metadata(&source)
            .map_err(|error| format!("could not inspect {}: {error}", source.display()))?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "symlinks are not accepted in copy_root: {}",
                source.display()
            ));
        }
        let target = destination.join(entry.file_name());
        if metadata.is_dir() {
            fs::create_dir(&target)
                .map_err(|error| format!("could not create {}: {error}", target.display()))?;
            copy_tree(&source, &target)?;
        } else if metadata.is_file() {
            fs::copy(&source, &target)
                .map_err(|error| format!("could not copy {}: {error}", source.display()))?;
        } else {
            return Err(format!(
                "only regular files and directories are accepted in copy_root: {}",
                source.display()
            ));
        }
    }
    Ok(())
}

fn sanitized_name(name: &str) -> String {
    let sanitized = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    if sanitized.is_empty() {
        "import".to_string()
    } else {
        sanitized
    }
}

fn operation_name(operation: &EditOperation) -> &'static str {
    match operation {
        EditOperation::InsertFiles(_) => "insert_files",
        EditOperation::InsertCaptions(_) => "insert_captions",
        EditOperation::InsertTts(_) => "insert_tts",
        EditOperation::InsertText(_) => "insert_text",
        EditOperation::CreateTrack(_) => "create_track",
        EditOperation::MoveClip(_) => "move_clip",
        EditOperation::TrimClip(_) => "trim_clip",
        EditOperation::DeleteClips(_) => "delete_clips",
        EditOperation::SetClipProperties(_) => "set_clip_properties",
        EditOperation::SetVideoTransform(_) => "set_video_transform",
        EditOperation::UpsertKeyframes(_) => "upsert_keyframes",
        EditOperation::DeleteKeyframes(_) => "delete_keyframes",
        EditOperation::UpsertPropertyExpression(_) => "upsert_property_expression",
        EditOperation::DeletePropertyExpression(_) => "delete_property_expression",
        EditOperation::SetClipTransitions(_) => "set_clip_transitions",
        EditOperation::SetExpression(_) => "set_expression",
        EditOperation::SetTrackEnabled(_) => "set_track_enabled",
        EditOperation::SetCaptionTrackLanguage(_) => "set_caption_track_language",
        EditOperation::DeleteTrack(_) => "delete_track",
    }
}
