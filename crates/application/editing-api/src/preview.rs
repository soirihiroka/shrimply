use crate::{edit, protocol::*, query};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use shrimply_project_document::project::Project;
use std::collections::BTreeMap;

pub fn revision(project: &Project) -> Result<String, String> {
    let mut project = project.clone();
    project.cursor_position = None;
    project.timeline_zoom = None;
    project.expanded_sequence_paths.clear();
    let value = serde_json::to_value(project).map_err(|error| error.to_string())?;
    let bytes = serde_json::to_vec(&value).map_err(|error| error.to_string())?;
    Ok(Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

pub fn saved_snapshot(project: Project, project_path: String) -> LiveSnapshot {
    let player = PlayerSnapshot {
        position: project.cursor_position.unwrap_or_default(),
        duration: project.duration(),
        playing: false,
        revision: 0,
    };
    LiveSnapshot {
        project_path,
        project,
        player,
        active_scope: ActiveScopeSnapshot {
            instance_path: Vec::new(),
            video_paths: vec![ScopeRef::default()],
            audio_paths: vec![ScopeRef::default()],
        },
        focused_item: None,
        selected_items: Vec::new(),
        focused_track: None,
        selected_tracks: Vec::new(),
        asset_revisions: BTreeMap::new(),
    }
}

pub fn query_call(name: &str, arguments: Value, snapshot: &LiveSnapshot) -> Result<Value, String> {
    macro_rules! value {
        ($value:expr) => {
            serde_json::to_value($value).map_err(|error| error.to_string())
        };
    }
    macro_rules! decode {
        ($type:ty) => {
            serde_json::from_value::<$type>(arguments).map_err(|error| error.to_string())?
        };
    }
    match name {
        "get_editor_state" | "project_get" => value!(query::editor_state(snapshot)?),
        "list_scopes" => value!(query::list_scopes(snapshot)?),
        "tracks_list" => Ok(
            json!({"tracks":query::list_scopes(snapshot)?.scopes.into_iter().flat_map(|scope|scope.tracks).collect::<Vec<_>>()}),
        ),
        "player_get" => {
            let state = query::editor_state(snapshot)?;
            Ok(
                json!({"playhead":state.playhead,"duration":state.duration,"playing":state.playing,"revision":state.revision}),
            )
        }
        "query_clips" => {
            let mut request = decode!(QueryClipsRequest);
            request.scope.get_or_insert_with(ScopeRef::default);
            value!(query::query_clips(snapshot, &request)?)
        }
        "get_clip" => {
            let request = decode!(GetClipRequest);
            value!(query::get_clip(
                snapshot,
                request.address.as_ref(),
                request.item_id.as_deref()
            )?)
        }
        "get_clip_info" => {
            let request = decode!(GetClipInfoRequest);
            let mut info = query::get_clip_info(
                snapshot,
                request.address.as_ref(),
                request.item_id.as_deref(),
            )?;
            if !request.include_artwork
                && let Some(source) = &mut info.source
            {
                source.artwork.clear();
            }
            value!(info)
        }
        "get_track" => value!(query::get_track(snapshot, &decode!(GetTrackRequest))?),
        "query_expressions" => value!(query::query_expressions(
            snapshot,
            &decode!(QueryExpressionsRequest)
        )?),
        _ => Err(format!("{name} requires an editing runtime")),
    }
}

pub fn is_snapshot_query(name: &str) -> bool {
    matches!(
        name,
        "get_editor_state"
            | "project_get"
            | "list_scopes"
            | "tracks_list"
            | "player_get"
            | "query_clips"
            | "get_clip"
            | "get_clip_info"
            | "get_track"
            | "query_expressions"
    )
}

pub fn edit_request(name: &str, arguments: Value) -> Result<Option<EditRequest>, String> {
    let mut frame = arguments.get("frame").and_then(Value::as_u64);
    let scope = arguments
        .get("scope")
        .filter(|scope| !scope.is_null())
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|error| error.to_string())?
        .unwrap_or_default();
    macro_rules! decode {
        ($type:ty) => {
            serde_json::from_value::<$type>(arguments).map_err(|error| error.to_string())?
        };
    }
    let operation = match name {
        "run_edit_script" => {
            let request = decode!(RunEditScriptRequest);
            return Ok(Some(EditRequest {
                history_label: "CLI edit batch".into(),
                frame: request.frame,
                scope: Some(request.scope.unwrap_or_default()),
                operations: request.operations,
            }));
        }
        "create_track" => {
            let request = decode!(CreateTrackRequest);
            EditOperation::CreateTrack(CreateTrackOperation {
                kind: request.kind,
                enabled: request.enabled,
            })
        }
        "insert_text" => {
            let request = decode!(InsertTextRequest);
            EditOperation::InsertText(InsertTextOperation {
                text: request.text,
                track: request.track,
                duration_frames: request.duration_frames,
                collision: request.collision,
            })
        }
        "insert_files" => EditOperation::InsertFiles(decode!(InsertFilesRequest)),
        "replace_clip_source" => {
            EditOperation::ReplaceClipSource(decode!(ReplaceClipSourceRequest))
        }
        "insert_captions" => EditOperation::InsertCaptions(decode!(InsertCaptionsRequest)),
        "insert_tts" => EditOperation::InsertTts(decode!(InsertTtsRequest)),
        "move_clip" => EditOperation::MoveClip(decode!(MoveClipRequest)),
        "trim_clip" => EditOperation::TrimClip(decode!(TrimClipRequest)),
        "delete_clips" => EditOperation::DeleteClips(decode!(DeleteClipsRequest)),
        "set_clip_properties" => {
            EditOperation::SetClipProperties(decode!(SetClipPropertiesRequest))
        }
        "set_video_transform" => {
            EditOperation::SetVideoTransform(decode!(SetVideoTransformRequest))
        }
        "upsert_keyframes" => EditOperation::UpsertKeyframes(decode!(UpsertKeyframesRequest)),
        "delete_keyframes" => EditOperation::DeleteKeyframes(decode!(DeleteKeyframesRequest)),
        "upsert_property_expression" => {
            EditOperation::UpsertPropertyExpression(decode!(UpsertPropertyExpressionRequest))
        }
        "delete_property_expression" => {
            EditOperation::DeletePropertyExpression(decode!(DeletePropertyExpressionRequest))
        }
        "set_clip_transitions" => {
            EditOperation::SetClipTransitions(decode!(SetClipTransitionsRequest))
        }
        "set_expression" => EditOperation::SetExpression(decode!(SetExpressionRequest)),
        "set_track_enabled" => EditOperation::SetTrackEnabled(decode!(SetTrackEnabledRequest)),
        "set_caption_track_language" => {
            EditOperation::SetCaptionTrackLanguage(decode!(SetCaptionTrackLanguageRequest))
        }
        "delete_track" => EditOperation::DeleteTrack(decode!(DeleteTrackRequest)),
        "project_set" => EditOperation::SetProjectSettings(decode!(ProjectSettingsRequest)),
        "split_clip" => {
            frame = None;
            EditOperation::SplitClip(decode!(SplitClipRequest))
        }
        "duplicate_clip" => EditOperation::DuplicateClip(decode!(DuplicateClipRequest)),
        "reorder_track" => EditOperation::ReorderTrack(decode!(ReorderTrackRequest)),
        "replace_text" => EditOperation::ReplaceText(decode!(TextReplaceRequest)),
        _ => return Ok(None),
    };
    Ok(Some(EditRequest {
        history_label: format!("CLI {}", name.replace('_', " ")),
        frame,
        scope: Some(scope),
        operations: vec![operation],
    }))
}

pub fn dry_run(call: &ApiCall, snapshot: &LiveSnapshot) -> Result<Value, String> {
    let base_revision = revision(&snapshot.project)?;
    if call
        .if_revision
        .as_ref()
        .is_some_and(|expected| expected != &base_revision)
    {
        return Err("revision conflict: project changed; inspect it and retry".into());
    }
    if is_snapshot_query(&call.name) {
        return query_call(&call.name, call.arguments.clone(), snapshot);
    }
    if matches!(call.name.as_str(), "history_undo" | "history_redo") {
        let mut changes = Vec::new();
        if let Some(mut candidate) =
            shrimply_project_document::project::history_destination(call.name == "history_redo")
        {
            candidate.cursor_position = snapshot.project.cursor_position;
            candidate.timeline_zoom = snapshot.project.timeline_zoom;
            candidate.expanded_sequence_paths = snapshot.project.expanded_sequence_paths.clone();
            diff(
                "",
                &serde_json::to_value(&snapshot.project).map_err(|error| error.to_string())?,
                &serde_json::to_value(candidate).map_err(|error| error.to_string())?,
                &mut changes,
            );
        }
        return Ok(
            json!({"dry_run":true,"base_revision":base_revision,"validation":"complete","changed":!changes.is_empty(),"changes":changes}),
        );
    }
    let Some(request) = edit_request(&call.name, call.arguments.clone())? else {
        return Ok(
            json!({"dry_run":true,"validation":"request_only","planned":{"operation":call.name,"arguments":call.arguments},"deferred":["Execution, service calls, source code execution, cache generation, and output creation were not performed"]}),
        );
    };
    if request.operations.is_empty() {
        return Err("edit request requires at least one operation".into());
    }
    let mut project = snapshot.project.clone();
    let scope = query::parse_path(&request.scope.clone().unwrap_or_default().sequence_path)?;
    let scope = project
        .sequence_scope_for_path(shrimply_project_document::project::ItemKind::Video, &scope)
        .ok_or("scope was not found")?;
    let anchor = request
        .frame
        .unwrap_or(snapshot.player.position.as_frame(project.fps));
    let mut deferred = Vec::new();
    let mut details = Vec::new();
    for (index, operation) in request.operations.iter().enumerate() {
        if !deferred.is_empty() {
            deferred.push(json!({"index":index,"operation":operation,"reason":"depends on deferred operations"}));
            continue;
        }
        if matches!(
            operation,
            EditOperation::InsertFiles(_)
                | EditOperation::ReplaceClipSource(_)
                | EditOperation::InsertTts(_)
        ) {
            if let EditOperation::ReplaceClipSource(request) = operation {
                edit::source_replacement_target(&project, request)?;
            }
            if let EditOperation::InsertFiles(request) = operation {
                if request.files.is_empty() {
                    return Err("insert_files requires at least one source file".into());
                }
                for file in &request.files {
                    if !std::path::Path::new(&file.source).is_file() {
                        return Err(format!("source file does not exist: {}", file.source));
                    }
                    for track in &file.targets {
                        if project.track(&query::model_track_address(track)?).is_none() {
                            return Err("import target track was not found".into());
                        }
                    }
                }
            }
            deferred.push(json!({"index":index,"operation":operation,"reason":"media inspection, conversion, or generation requires execution"}));
            continue;
        }
        let result = edit::apply_non_import(&mut project, operation, anchor, &scope)?;
        if let Some(detail) = result.details {
            details.push(detail);
        }
    }
    project.validate()?;
    let mut changes = Vec::new();
    diff(
        "",
        &serde_json::to_value(&snapshot.project).map_err(|error| error.to_string())?,
        &serde_json::to_value(&project).map_err(|error| error.to_string())?,
        &mut changes,
    );
    Ok(
        json!({"dry_run":true,"base_revision":base_revision,"validation":if deferred.is_empty(){"complete"}else{"partial"},"changes":changes,"details":details,"deferred":deferred,"generated_ids":"IDs for newly created items are provisional"}),
    )
}

fn diff(path: &str, before: &Value, after: &Value, changes: &mut Vec<Value>) {
    if before == after {
        return;
    }
    match (before, after) {
        (Value::Object(before), Value::Object(after)) => {
            let keys: std::collections::BTreeSet<_> = before.keys().chain(after.keys()).collect();
            for key in keys {
                diff(
                    &format!("{path}/{}", key.replace('~', "~0").replace('/', "~1")),
                    before.get(key).unwrap_or(&Value::Null),
                    after.get(key).unwrap_or(&Value::Null),
                    changes,
                );
            }
        }
        (Value::Array(before), Value::Array(after)) => {
            for index in 0..before.len().max(after.len()) {
                diff(
                    &format!("{path}/{index}"),
                    before.get(index).unwrap_or(&Value::Null),
                    after.get(index).unwrap_or(&Value::Null),
                    changes,
                );
            }
        }
        _ => changes.push(json!({"path":path,"before":before,"after":after})),
    }
}

/// Attach provenance without replacing a revision captured by a read operation.
pub fn with_state(mut value: Value, project: &Project, mode: &str) -> Result<Value, String> {
    if let Some(object) = value.as_object_mut() {
        if !object.contains_key("content_revision") {
            object.insert("content_revision".into(), json!(revision(project)?));
        }
        object.insert("execution_mode".into(), json!(mode));
    }
    Ok(value)
}
