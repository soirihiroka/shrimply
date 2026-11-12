use std::collections::HashSet;
use std::path::Path;

use serde_json::{Value, json};
use shrimply_math_core::{
    Fraction, Time, fraction_denominator, fraction_numerator, time_from_frame,
};
use shrimply_project_document::project::{
    AudioSource, ItemAddress as ModelItemAddress, ItemKind, Project,
    TrackAddress as ModelTrackAddress, TrackRef, VideoItemContent, supported_caption_language,
};
use uuid::Uuid;

use crate::protocol::*;

const DEFAULT_QUERY_LIMIT: usize = 100;
const MAX_QUERY_LIMIT: usize = 500;

#[derive(Clone)]
struct Presentation {
    summary: ClipSummary,
    projected_start: Time,
    projected_end: Time,
    source_path: Option<String>,
    metadata: Value,
}

struct PresentationData {
    enabled: bool,
    source_kind: String,
    source_path: Option<String>,
    label: String,
    state: Value,
    metadata: Value,
}

pub fn model_track_address(address: &TrackAddress) -> Result<ModelTrackAddress, String> {
    let track_id = parse_uuid(&address.track_id, "track_id")?;
    let sequence_path = parse_path(&address.sequence_path)?;
    match address.kind {
        ClipKind::Comment if sequence_path.is_empty() => {
            Ok(ModelTrackAddress::Comment { track_id })
        }
        ClipKind::Caption if sequence_path.is_empty() => {
            Ok(ModelTrackAddress::Caption { track_id })
        }
        ClipKind::Comment => Err("comment tracks cannot be nested".to_string()),
        ClipKind::Caption => Err("caption tracks cannot be nested".to_string()),
        ClipKind::Video => Ok(ModelTrackAddress::Video {
            sequence_path,
            track_id,
        }),
        ClipKind::Audio => Ok(ModelTrackAddress::Audio {
            sequence_path,
            track_id,
        }),
    }
}

pub fn model_item_address(address: &ClipAddress) -> Result<ModelItemAddress, String> {
    Ok(model_track_address(&TrackAddress {
        kind: address.kind,
        sequence_path: address.sequence_path.clone(),
        track_id: address.track_id.clone(),
    })?
    .item(parse_uuid(&address.item_id, "item_id")?))
}

pub fn protocol_track_address(address: &ModelTrackAddress) -> TrackAddress {
    TrackAddress {
        kind: protocol_kind(address.kind()),
        sequence_path: address
            .sequence_path()
            .iter()
            .map(Uuid::to_string)
            .collect(),
        track_id: address.track_id().to_string(),
    }
}

pub fn protocol_item_address(address: &ModelItemAddress) -> ClipAddress {
    let track = protocol_track_address(&address.track());
    ClipAddress {
        kind: track.kind,
        sequence_path: track.sequence_path,
        track_id: track.track_id,
        item_id: address.item_id().to_string(),
    }
}

pub fn model_kind(kind: ClipKind) -> ItemKind {
    match kind {
        ClipKind::Comment => ItemKind::Comment,
        ClipKind::Caption => ItemKind::Caption,
        ClipKind::Video => ItemKind::Video,
        ClipKind::Audio => ItemKind::Audio,
    }
}

fn protocol_kind(kind: ItemKind) -> ClipKind {
    match kind {
        ItemKind::Comment => ClipKind::Comment,
        ItemKind::Caption => ClipKind::Caption,
        ItemKind::Video => ClipKind::Video,
        ItemKind::Audio => ClipKind::Audio,
    }
}

pub fn parse_path(path: &[String]) -> Result<Vec<Uuid>, String> {
    path.iter()
        .map(|id| parse_uuid(id, "sequence_path item ID"))
        .collect()
}

fn parse_uuid(value: &str, field: &str) -> Result<Uuid, String> {
    Uuid::parse_str(value).map_err(|error| format!("invalid {field} {value:?}: {error}"))
}

pub fn frame_time(frame: u64, fps: Fraction) -> Result<FrameTime, String> {
    let time = time_from_frame(frame, fps)
        .ok_or_else(|| "frame time exceeds the supported exact fraction range".to_string())?;
    Ok(FrameTime {
        frame,
        seconds: exact(time.seconds),
    })
}

pub fn frame_time_from_time(time: Time, fps: Fraction, ceil: bool) -> FrameTime {
    FrameTime {
        frame: if ceil {
            time.as_frame_ceil(fps)
        } else {
            time.as_frame(fps)
        },
        seconds: exact(time.seconds),
    }
}

pub fn editor_state(snapshot: &LiveSnapshot) -> Result<EditorState, String> {
    let project = &snapshot.project;
    let active = &snapshot.active_scope;
    let mut tracks = Vec::new();
    for scope in active.video_paths.iter().chain(&active.audio_paths) {
        tracks.extend(scope_tracks(project, scope)?);
    }
    tracks.sort_by_key(|track| {
        (
            track.address.kind as u8,
            track.address.sequence_path.clone(),
            track.address.track_id.clone(),
        )
    });
    tracks.dedup_by(|left, right| left.address == right.address);
    Ok(EditorState {
        project_path: snapshot.project_path.clone(),
        project_name: project.name.clone(),
        fps: exact(project.fps),
        canvas: CanvasSummary {
            width: project.canvas_size.width,
            height: project.canvas_size.height,
        },
        duration: frame_time_from_time(snapshot.player.duration, project.fps, true),
        playhead: frame_time(snapshot.player.position.as_frame(project.fps), project.fps)?,
        playing: snapshot.player.playing,
        revision: snapshot.player.revision,
        active_scope: ActiveScopeSummary {
            instance_path: active.instance_path.clone(),
            video_presentations: active.video_paths.clone(),
            audio_presentations: active.audio_paths.clone(),
        },
        focused_item: snapshot.focused_item.clone(),
        selected_items: snapshot.selected_items.clone(),
        focused_track: snapshot.focused_track.clone(),
        selected_tracks: snapshot.selected_tracks.clone(),
        tracks,
    })
}

pub fn list_scopes(snapshot: &LiveSnapshot) -> Result<ListScopesResponse, String> {
    let project = &snapshot.project;
    let mut paths = concrete_scope_paths(project);
    paths.sort();
    paths.dedup();
    let scopes = paths
        .into_iter()
        .map(|sequence_path| {
            let scope = ScopeRef { sequence_path };
            Ok(ScopeSummary {
                tracks: scope_tracks(project, &scope)?,
                scope,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(ListScopesResponse { scopes })
}

pub fn query_clips(
    snapshot: &LiveSnapshot,
    request: &QueryClipsRequest,
) -> Result<QueryClipsResponse, String> {
    if let Some(range) = &request.range
        && range.end_frame <= range.start_frame
    {
        return Err("range must be half-open with end_frame > start_frame".to_string());
    }
    let project = &snapshot.project;
    if let Some(scope) = &request.scope {
        scope_tracks(project, scope)?;
    }
    let offset = request.offset.unwrap_or(0);
    if request.all && (request.offset.is_some() || request.limit.is_some()) {
        return Err("all conflicts with pagination offset/limit".into());
    }
    let limit = if request.all {
        usize::MAX
    } else {
        request
            .limit
            .unwrap_or(DEFAULT_QUERY_LIMIT)
            .min(MAX_QUERY_LIMIT)
    };
    let mut clips = presentations(project)?
        .into_iter()
        .filter(|clip| in_requested_scope(&clip.summary.address, request))
        .filter(|clip| {
            request
                .kind
                .is_none_or(|kind| clip.summary.address.kind == kind)
        })
        .filter(|clip| {
            request
                .source_kind
                .as_ref()
                .is_none_or(|kind| &clip.summary.source_kind == kind)
        })
        .filter(|clip| {
            request
                .track_id
                .as_ref()
                .is_none_or(|id| &clip.summary.address.track_id == id)
        })
        .filter(|clip| {
            request
                .item_id
                .as_ref()
                .is_none_or(|id| &clip.summary.address.item_id == id)
        })
        .filter(|clip| {
            request
                .enabled
                .is_none_or(|enabled| clip.summary.enabled == enabled)
        })
        .filter(|clip| {
            request.caption_text.as_ref().is_none_or(|needle| {
                clip.summary.address.kind == ClipKind::Caption
                    && clip.summary.label.contains(needle)
            })
        })
        .filter(|clip| {
            request.source_filename.as_ref().is_none_or(|needle| {
                clip.source_path.as_ref().is_some_and(|path| {
                    Path::new(path)
                        .file_name()
                        .is_some_and(|name| name.to_string_lossy().contains(needle))
                })
            })
        })
        .filter(|clip| range_matches(project, clip, request))
        .collect::<Vec<_>>();
    clips.sort_by_key(|clip| {
        (
            clip.projected_start.as_frame(project.fps),
            clip.summary.address.kind as u8,
            clip.summary.address.track_id.clone(),
            clip.summary.address.item_id.clone(),
        )
    });
    let total = clips.len();
    let clips = clips
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|clip| clip.summary)
        .collect();
    Ok(QueryClipsResponse {
        clips,
        offset,
        limit: if request.all { total } else { limit },
        total,
    })
}

pub fn query_expressions(
    snapshot: &LiveSnapshot,
    request: &QueryExpressionsRequest,
) -> Result<QueryExpressionsResponse, String> {
    if let Some(address) = &request.address {
        let address = model_item_address(address)?;
        if snapshot.project.item(&address).is_none() {
            return Err("clip was not found in the live project".to_string());
        }
    }
    let offset = request.offset.unwrap_or(0);
    let limit = request
        .limit
        .unwrap_or(DEFAULT_QUERY_LIMIT)
        .min(MAX_QUERY_LIMIT);
    let mut seen = HashSet::new();
    let mut expressions = presentations(&snapshot.project)?
        .into_iter()
        .filter(|clip| {
            request
                .address
                .as_ref()
                .is_none_or(|address| clip.summary.address == *address)
        })
        .flat_map(|clip| crate::expression::summaries(&clip.metadata, &clip.summary.address))
        .filter(|expression| seen.insert(expression.expression_id.clone()))
        .filter(|expression| {
            request
                .source_contains
                .as_ref()
                .is_none_or(|needle| expression.source.contains(needle))
        })
        .collect::<Vec<_>>();
    expressions.sort_by_key(|expression| {
        (
            expression.address.kind as u8,
            expression.address.sequence_path.clone(),
            expression.address.track_id.clone(),
            expression.address.item_id.clone(),
            expression.property_path.clone(),
        )
    });
    let total = expressions.len();
    let expressions = expressions.into_iter().skip(offset).take(limit).collect();
    Ok(QueryExpressionsResponse {
        expressions,
        offset,
        limit,
        total,
    })
}

pub fn all_clips(snapshot: &LiveSnapshot) -> Result<ProjectClipsResource, String> {
    let mut clips = presentations(&snapshot.project)?
        .into_iter()
        .map(|clip| clip.summary)
        .collect::<Vec<_>>();
    clips.sort_by_key(|clip| {
        (
            clip.projected.start.frame,
            clip.address.kind as u8,
            clip.address.track_id.clone(),
            clip.address.item_id.clone(),
        )
    });
    let total = clips.len();
    Ok(ProjectClipsResource { clips, total })
}

pub fn get_clip(
    snapshot: &LiveSnapshot,
    address: Option<&ClipAddress>,
    item_id: Option<&str>,
) -> Result<ClipMetadata, String> {
    if address.is_none() == item_id.is_none() {
        return Err("provide exactly one of address or item_id".to_string());
    }
    let project = &snapshot.project;
    let all = presentations(project)?;
    let resolved_item_id = if let Some(address) = address {
        all.iter()
            .find(|clip| clip.summary.address == *address)
            .map(|clip| clip.summary.address.item_id.clone())
            .ok_or_else(|| "clip was not found in the live project".to_string())?
    } else {
        item_id.expect("one selector was checked").to_string()
    };
    let mut matches = all
        .into_iter()
        .filter(|clip| clip.summary.address.item_id == resolved_item_id)
        .collect::<Vec<_>>();
    if let Some(address) = address {
        let position = matches
            .iter()
            .position(|clip| clip.summary.address == *address)
            .expect("exact address was resolved before filtering by item ID");
        matches.swap(0, position);
    }
    let first = matches
        .first()
        .ok_or_else(|| "clip was not found in the live project".to_string())?;
    let owning_track = track_summary(project, &clip_track(&first.summary.address))?;
    Ok(ClipMetadata {
        metadata: first.metadata.clone(),
        owning_track,
        asset: asset_metadata(
            first.source_path.as_deref(),
            first
                .source_path
                .as_ref()
                .and_then(|path| snapshot.asset_revisions.get(path))
                .copied(),
            Path::new(&snapshot.project_path),
        ),
        presentations: matches.into_iter().map(|clip| clip.summary).collect(),
    })
}

pub fn get_clip_info(
    snapshot: &LiveSnapshot,
    address: Option<&ClipAddress>,
    item_id: Option<&str>,
) -> Result<ClipInfo, String> {
    let clip = get_clip(snapshot, address, item_id)?;
    let Some(asset) = &clip.asset else {
        return Ok(ClipInfo {
            clip,
            source: None,
            selected_stream_index: None,
            source_error: None,
        });
    };
    let revision = asset.asset_revision;
    let source = match shrimply_media_info::inspect(Path::new(&asset.path), revision) {
        Ok(source) => source,
        Err(error) => {
            return Ok(ClipInfo {
                clip,
                source: None,
                selected_stream_index: None,
                source_error: Some(error),
            });
        }
    };
    let selected_stream_index = selected_stream_index(&clip, &source);
    Ok(ClipInfo {
        clip,
        source: Some((*source).clone()),
        selected_stream_index,
        source_error: None,
    })
}

fn selected_stream_index(
    clip: &ClipMetadata,
    source: &shrimply_media_info::FileInfo,
) -> Option<usize> {
    let presentation = clip.presentations.first()?;
    let kind = match presentation.address.kind {
        ClipKind::Video => "video",
        ClipKind::Audio => "audio",
        ClipKind::Comment => return None,
        ClipKind::Caption => return None,
    };
    let ordinal = clip.metadata.get("track_id")?.as_u64()? as usize;
    source
        .streams
        .iter()
        .filter(|stream| stream.kind == kind)
        .nth(ordinal)
        .map(|stream| stream.index)
}

pub fn get_track(
    snapshot: &LiveSnapshot,
    request: &GetTrackRequest,
) -> Result<TrackMetadata, String> {
    let track = track_summary(&snapshot.project, &request.address)?;
    let offset = request.offset.unwrap_or(0);
    let limit = request
        .limit
        .unwrap_or(MAX_QUERY_LIMIT)
        .min(MAX_QUERY_LIMIT);
    let mut clips = presentations(&snapshot.project)?
        .into_iter()
        .filter(|clip| clip.summary.address.kind == request.address.kind)
        .filter(|clip| clip.summary.address.sequence_path == request.address.sequence_path)
        .filter(|clip| clip.summary.address.track_id == request.address.track_id)
        .collect::<Vec<_>>();
    clips.sort_by_key(|clip| {
        (
            clip.projected_start.as_frame(snapshot.project.fps),
            clip.summary.address.item_id.clone(),
        )
    });
    let total = clips.len();
    let clips = clips
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|clip| clip.summary)
        .collect();
    Ok(TrackMetadata {
        track,
        clips,
        offset,
        limit,
        total,
    })
}

pub fn presentations_affected_by_items(
    project: &Project,
    item_ids: &HashSet<Uuid>,
) -> Result<Vec<ClipSummary>, String> {
    Ok(presentations(project)?
        .into_iter()
        .filter(|clip| {
            Uuid::parse_str(&clip.summary.address.item_id).is_ok_and(|id| item_ids.contains(&id))
                || clip
                    .summary
                    .address
                    .sequence_path
                    .iter()
                    .any(|id| Uuid::parse_str(id).is_ok_and(|id| item_ids.contains(&id)))
        })
        .map(|clip| clip.summary)
        .collect())
}

pub fn presentations_for_tracks(
    project: &Project,
    track_ids: &HashSet<Uuid>,
) -> Result<Vec<ClipSummary>, String> {
    Ok(presentations(project)?
        .into_iter()
        .filter(|clip| {
            Uuid::parse_str(&clip.summary.address.track_id).is_ok_and(|id| track_ids.contains(&id))
        })
        .map(|clip| clip.summary)
        .collect())
}

pub fn addresses_for_tracks(
    project: &Project,
    track_ids: &HashSet<Uuid>,
) -> Result<Vec<TrackAddress>, String> {
    let mut addresses = Vec::new();
    for sequence_path in concrete_scope_paths(project) {
        addresses.extend(
            scope_tracks(project, &ScopeRef { sequence_path })?
                .into_iter()
                .map(|track| track.address)
                .filter(|address| {
                    Uuid::parse_str(&address.track_id).is_ok_and(|id| track_ids.contains(&id))
                }),
        );
    }
    addresses.sort_by_key(|address| {
        (
            address.kind as u8,
            address.sequence_path.clone(),
            address.track_id.clone(),
        )
    });
    addresses.dedup();
    Ok(addresses)
}

fn in_requested_scope(address: &ClipAddress, request: &QueryClipsRequest) -> bool {
    let root = ScopeRef::default();
    let scope = request.scope.as_ref().unwrap_or(&root);
    in_scope(
        &address.sequence_path,
        &scope.sequence_path,
        request.recursive,
    )
}

fn range_matches(project: &Project, clip: &Presentation, request: &QueryClipsRequest) -> bool {
    let Some(range) = &request.range else {
        return true;
    };
    let start = clip.projected_start.as_frame(project.fps);
    let end = clip.projected_end.as_frame_ceil(project.fps);
    match request.range_match {
        RangeMatch::Overlaps => start < range.end_frame && end > range.start_frame,
        RangeMatch::Contained => start >= range.start_frame && end <= range.end_frame,
        RangeMatch::StartsIn => start >= range.start_frame && start < range.end_frame,
    }
}

fn in_scope(path: &[String], scope: &[String], recursive: bool) -> bool {
    if recursive {
        path.starts_with(scope)
    } else {
        path == scope
    }
}

fn exact(value: Fraction) -> ExactFraction {
    ExactFraction {
        numerator: fraction_numerator(value),
        denominator: fraction_denominator(value),
    }
}

fn time_span(project: &Project, start: Time, end: Time) -> TimeSpan {
    TimeSpan {
        start: frame_time_from_time(start, project.fps, false),
        end: frame_time_from_time(end, project.fps, true),
    }
}

fn asset_metadata(
    path: Option<&str>,
    revision: Option<u64>,
    project_path: &Path,
) -> Option<AssetMetadata> {
    let path = path.filter(|path| !path.is_empty())?;
    let path = Path::new(path);
    let canonical = path.canonicalize().ok();
    let metadata = path.metadata().ok();
    let project_media = project_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("media");
    let canonical_media = project_media.canonicalize().unwrap_or(project_media);
    Some(AssetMetadata {
        path: path.to_string_lossy().into_owned(),
        canonical_path: canonical
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned()),
        exists: metadata.is_some(),
        size: metadata.as_ref().map(std::fs::Metadata::len),
        modified_unix_seconds: metadata
            .and_then(|metadata| metadata.modified().ok())
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs()),
        asset_revision: revision,
        inside_project_media: canonical
            .as_ref()
            .is_some_and(|path| path.starts_with(canonical_media)),
    })
}

fn scope_tracks(project: &Project, scope: &ScopeRef) -> Result<Vec<TrackSummary>, String> {
    let path = parse_path(&scope.sequence_path)?;
    let mut tracks = Vec::new();
    if path.is_empty() {
        tracks.extend(project.comment_tracks.iter().map(|track| TrackSummary {
            address: TrackAddress {
                kind: ClipKind::Comment,
                sequence_path: Vec::new(),
                track_id: track.id.to_string(),
            },
            enabled: track.enabled,
            language: None,
            clip_count: track.items.len(),
        }));
        tracks.extend(project.caption_tracks.iter().map(|track| TrackSummary {
            address: TrackAddress {
                kind: ClipKind::Caption,
                sequence_path: Vec::new(),
                track_id: track.id.to_string(),
            },
            enabled: track.enabled,
            language: supported_caption_language(&track.language),
            clip_count: track.items.len(),
        }));
    }
    let video_tracks = project.video_tracks_for_path(&path);
    if let Some(video_tracks) = video_tracks {
        tracks.extend(video_tracks.iter().map(|track| TrackSummary {
            address: TrackAddress {
                kind: ClipKind::Video,
                sequence_path: scope.sequence_path.clone(),
                track_id: track.id.to_string(),
            },
            enabled: track.enabled,
            language: None,
            clip_count: track.items.len(),
        }));
    }
    let audio_tracks = project.audio_tracks_for_path(&path);
    if let Some(audio_tracks) = audio_tracks {
        tracks.extend(audio_tracks.iter().map(|track| TrackSummary {
            address: TrackAddress {
                kind: ClipKind::Audio,
                sequence_path: scope.sequence_path.clone(),
                track_id: track.id.to_string(),
            },
            enabled: track.enabled,
            language: None,
            clip_count: track.items.len(),
        }));
    }
    if !path.is_empty() && video_tracks.is_none() && audio_tracks.is_none() {
        return Err("scope does not resolve to a concrete sequence presentation".to_string());
    }
    Ok(tracks)
}

fn track_summary(project: &Project, address: &TrackAddress) -> Result<TrackSummary, String> {
    let model = model_track_address(address)?;
    match project
        .track(&model)
        .ok_or_else(|| "track was not found".to_string())?
    {
        TrackRef::Comment(track) => Ok(TrackSummary {
            address: address.clone(),
            enabled: track.enabled,
            language: None,
            clip_count: track.items.len(),
        }),
        TrackRef::Caption(track) => Ok(TrackSummary {
            address: address.clone(),
            enabled: track.enabled,
            language: supported_caption_language(&track.language),
            clip_count: track.items.len(),
        }),
        TrackRef::Video(track) => Ok(TrackSummary {
            address: address.clone(),
            enabled: track.enabled,
            language: None,
            clip_count: track.items.len(),
        }),
        TrackRef::Audio(track) => Ok(TrackSummary {
            address: address.clone(),
            enabled: track.enabled,
            language: None,
            clip_count: track.items.len(),
        }),
    }
}

fn concrete_scope_paths(project: &Project) -> Vec<Vec<String>> {
    fn video(
        project: &Project,
        tracks: &[shrimply_project_document::project::VisualTrack],
        path: &[Uuid],
        paths: &mut Vec<Vec<String>>,
    ) {
        for item in tracks.iter().flat_map(|track| &track.items) {
            let VideoItemContent::FoldedSequence(reference) = item.content else {
                continue;
            };
            let mut child = path.to_vec();
            child.push(item.id);
            paths.push(child.iter().map(Uuid::to_string).collect());
            if let Some(sequence) = project.folded_sequence(reference.sequence_id) {
                video(project, &sequence.video_tracks, &child, paths);
            }
        }
    }
    fn audio(
        project: &Project,
        tracks: &[shrimply_project_document::project::AudioTrack],
        path: &[Uuid],
        paths: &mut Vec<Vec<String>>,
    ) {
        for item in tracks.iter().flat_map(|track| &track.items) {
            let AudioSource::FoldedSequence(reference) = &item.source else {
                continue;
            };
            let mut child = path.to_vec();
            child.push(item.id);
            paths.push(child.iter().map(Uuid::to_string).collect());
            if let Some(sequence) = project.folded_sequence(reference.sequence_id) {
                audio(project, &sequence.audio_tracks, &child, paths);
            }
        }
    }

    let mut paths = vec![Vec::new()];
    video(project, &project.video_tracks, &[], &mut paths);
    audio(project, &project.audio_tracks, &[], &mut paths);
    paths
}

fn presentations(project: &Project) -> Result<Vec<Presentation>, String> {
    let mut output = Vec::new();
    for track in &project.comment_tracks {
        for item in &track.items {
            let address = ModelTrackAddress::Comment { track_id: track.id }.item(item.id);
            push_presentation(
                project,
                &address,
                PresentationData {
                    enabled: track.enabled,
                    source_kind: "comment".to_string(),
                    source_path: None,
                    label: item.text.clone(),
                    state: json!({ "text": item.text, "color": item.color }),
                    metadata: serde_json::to_value(item).expect("comment item must serialize"),
                },
                &mut output,
            )?;
        }
    }
    for track in &project.caption_tracks {
        for item in &track.items {
            let address = ModelTrackAddress::Caption { track_id: track.id }.item(item.id);
            push_presentation(
                project,
                &address,
                PresentationData {
                    enabled: track.enabled,
                    source_kind: "caption".to_string(),
                    source_path: None,
                    label: item.text.clone(),
                    state: json!({ "text": item.text }),
                    metadata: serde_json::to_value(item).expect("caption item must serialize"),
                },
                &mut output,
            )?;
        }
    }
    collect_video(project, &project.video_tracks, &[], true, &mut output)?;
    collect_audio(project, &project.audio_tracks, &[], true, &mut output)?;
    Ok(output)
}

fn collect_video(
    project: &Project,
    tracks: &[shrimply_project_document::project::VisualTrack],
    path: &[Uuid],
    parent_enabled: bool,
    output: &mut Vec<Presentation>,
) -> Result<(), String> {
    for track in tracks {
        let enabled = parent_enabled && track.enabled;
        for item in &track.items {
            let source_kind = video_source_kind(&item.content);
            let source_path = (!item.file.as_os_str().is_empty())
                .then(|| item.file.path().to_string_lossy().into_owned());
            let address = ModelTrackAddress::Video {
                sequence_path: path.to_vec(),
                track_id: track.id,
            }
            .item(item.id);
            push_presentation(
                project,
                &address,
                PresentationData {
                    enabled,
                    source_kind: source_kind.to_string(),
                    source_path,
                    label: item
                        .file
                        .path()
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| source_kind.to_string()),
                    state: json!({
                        "playback_speed": exact(item.playback_speed),
                        "repeat_strategy": item.repeat_strategy,
                        "content": source_kind,
                    }),
                    metadata: serde_json::to_value(item).expect("video item must serialize"),
                },
                output,
            )?;
            if let VideoItemContent::FoldedSequence(reference) = item.content
                && let Some(sequence) = project.folded_sequence(reference.sequence_id)
            {
                let mut child = path.to_vec();
                child.push(item.id);
                collect_video(project, &sequence.video_tracks, &child, enabled, output)?;
            }
        }
    }
    Ok(())
}

fn collect_audio(
    project: &Project,
    tracks: &[shrimply_project_document::project::AudioTrack],
    path: &[Uuid],
    parent_enabled: bool,
    output: &mut Vec<Presentation>,
) -> Result<(), String> {
    for track in tracks {
        let track_enabled = parent_enabled && track.enabled;
        for item in &track.items {
            let enabled = track_enabled && item.enabled;
            let source_kind = audio_source_kind(&item.source);
            let source_path = (!item.file.as_os_str().is_empty())
                .then(|| item.file.path().to_string_lossy().into_owned());
            let address = ModelTrackAddress::Audio {
                sequence_path: path.to_vec(),
                track_id: track.id,
            }
            .item(item.id);
            push_presentation(
                project,
                &address,
                PresentationData {
                    enabled,
                    source_kind: source_kind.to_string(),
                    source_path,
                    label: item
                        .file
                        .path()
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| source_kind.to_string()),
                    state: json!({
                        "enabled": item.enabled,
                        "gain": item.gain,
                        "playback_speed": exact(item.playback_speed),
                        "repeat_strategy": item.repeat_strategy,
                    }),
                    metadata: serde_json::to_value(item).expect("audio item must serialize"),
                },
                output,
            )?;
            if let AudioSource::FoldedSequence(reference) = &item.source
                && let Some(sequence) = project.folded_sequence(reference.sequence_id)
            {
                let mut child = path.to_vec();
                child.push(item.id);
                collect_audio(project, &sequence.audio_tracks, &child, enabled, output)?;
            }
        }
    }
    Ok(())
}

fn push_presentation(
    project: &Project,
    address: &ModelItemAddress,
    data: PresentationData,
    output: &mut Vec<Presentation>,
) -> Result<(), String> {
    let Some(item) = project.item(address) else {
        return Ok(());
    };
    let (local_start, local_end) = item.times();
    let Some((projected_start, projected_end)) = project.projected_item_times(address) else {
        return Ok(());
    };
    let local = time_span(project, local_start, local_end);
    let projected = time_span(project, projected_start, projected_end);
    output.push(Presentation {
        summary: ClipSummary {
            address: protocol_item_address(address),
            label: data.label,
            source_kind: data.source_kind,
            asset_path: data.source_path.clone(),
            enabled: data.enabled,
            local,
            projected,
            state: data.state,
        },
        projected_start,
        projected_end,
        source_path: data.source_path,
        metadata: data.metadata,
    });
    Ok(())
}

fn video_source_kind(content: &VideoItemContent) -> &'static str {
    match content {
        VideoItemContent::Media => "media",
        VideoItemContent::Image => "image",
        VideoItemContent::Gif => "gif",
        VideoItemContent::Svg => "svg",
        VideoItemContent::Pdf(_) => "pdf",
        VideoItemContent::Manim(_) => "manim",
        VideoItemContent::Sanim(_) => "sanim",
        VideoItemContent::Blender(_) => "blender",
        VideoItemContent::LayeredImage(_) => "layered_image",
        VideoItemContent::Text(_) => "text",
        VideoItemContent::Shape(_) => "shape",
        VideoItemContent::Paint(_) => "paint",
        VideoItemContent::Background(_) => "background",
        VideoItemContent::Obj(_) => "obj",
        VideoItemContent::Gaussian(_) => "ply",
        VideoItemContent::FoldedSequence(_) => "folded_sequence",
    }
}

fn audio_source_kind(source: &AudioSource) -> &'static str {
    match source {
        AudioSource::Media => "media",
        AudioSource::FoldedSequence(_) => "folded_sequence",
        AudioSource::Tts(_) => "tts",
        AudioSource::Generator(_) => "generator",
    }
}

fn clip_track(address: &ClipAddress) -> TrackAddress {
    TrackAddress {
        kind: address.kind,
        sequence_path: address.sequence_path.clone(),
        track_id: address.track_id.clone(),
    }
}
