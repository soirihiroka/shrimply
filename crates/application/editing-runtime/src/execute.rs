use super::*;
use shrimply_editing_api::protocol::*;
use shrimply_editing_api::{commands, preview};

pub(super) struct Context<'a> {
    pub project: &'a Rc<RefCell<Project>>,
    pub player: &'a SharedPlayerState,
    pub selection: &'a SharedSelectionState,
    pub preferences: &'a SharedPreferences,
    pub headless: bool,
}

pub(super) async fn call(
    context: Context<'_>,
    mut call: ApiCall,
    canceled: Arc<AtomicBool>,
) -> Result<Value, String> {
    let Context {
        project,
        player,
        selection,
        preferences,
        headless,
    } = context;
    let definition = commands::get(&call.name)?;
    call.arguments = (definition.validate)(call.arguments)?;
    call.name = definition.name.into();
    if call.arguments.get("scope").is_some_and(Value::is_null) {
        call.arguments["scope"] = json!({"sequence_path":[]});
    }
    if call
        .if_revision
        .as_ref()
        .is_some_and(|expected| preview::revision(&project.borrow()).as_ref() != Ok(expected))
    {
        return Err("revision conflict: project changed; inspect it and retry".into());
    }
    if call.dry_run {
        return preview::dry_run(&call, &snapshot(project, player, selection)?);
    }
    if preview::is_snapshot_query(&call.name) {
        let snapshot = snapshot(project, player, selection)?;
        if call.name == "get_clip_info" {
            let (sender, receiver) = async_channel::bounded(1);
            thread::Builder::new()
                .name("shrimply-clip-metadata".into())
                .spawn(move || {
                    let result = preview::query_call(&call.name, call.arguments, &snapshot)
                        .and_then(|value| preview::with_state(value, &snapshot.project, "saved"));
                    let _ = sender.send_blocking(result);
                })
                .map_err(|error| error.to_string())?;
            return receiver.recv().await.map_err(|error| error.to_string())?;
        }
        return preview::query_call(&call.name, call.arguments, &snapshot);
    }
    if let Some(request) = preview::edit_request(&call.name, call.arguments.clone())? {
        return apply_edit(
            project,
            player,
            selection,
            preferences::snapshot(preferences).default_visual_duration,
            request,
            canceled,
        )
        .await;
    }
    macro_rules! decode {
        ($type:ty) => {
            serde_json::from_value::<$type>(call.arguments).map_err(|error| error.to_string())?
        };
    }
    let query_revision = definition
        .read_only
        .then(|| content_revision(&project.borrow()))
        .transpose()?;
    let result = match call.name.as_str() {
        "get_manim_clip" => get_manim_clip(project, decode!(GetManimClipRequest), canceled).await,
        "set_manim_clip" => {
            set_manim_clip(project, player, decode!(SetManimClipRequest), canceled).await
        }
        "reload_manim_source" => reload_manim_source(project, decode!(ReloadManimSourceRequest)),
        "list_tts_models" => list_tts_models(preferences, canceled).await,
        "list_stt_models" => list_stt_models(preferences, canceled).await,
        "transcribe_audio" => {
            transcribe_audio(
                project,
                player,
                selection,
                preferences,
                decode!(TranscribeAudioRequest),
                canceled,
            )
            .await
        }
        "generate_tts" => {
            generate_tts(
                project,
                player,
                selection,
                preferences,
                decode!(GenerateTtsRequest),
                canceled,
            )
            .await
        }
        "view_frame" => view_frame(project, decode!(ViewFrameRequest).frame, canceled).await,
        "analyze_transparent_fill" => {
            analyze_transparent_fill(
                project,
                player,
                decode!(AnalyzeTransparentFillRequest),
                canceled,
            )
            .await
        }
        "seek_playhead" => {
            let frame = decode!(SeekPlayheadRequest).frame;
            let fps = project.borrow().fps;
            let position =
                time_from_frame(frame, fps).ok_or("frame exceeds the supported exact range")?;
            player_state::seek_time(player, position);
            {
                let mut current = project.borrow_mut();
                current.cursor_position = Some(position);
                shrimply_project_document::project::save_view_state(&current);
            }
            shrimply_project_document::project::save()?;
            serde_json::to_value(shrimply_editing_api::query::frame_time(frame, fps)?)
                .map_err(|error| error.to_string())
        }
        "project_save" => {
            shrimply_project_document::project::save()?;
            Ok(json!({"saved":true}))
        }
        "project_close" => {
            if !headless {
                return Err("close the GUI window to release this project; project close only stops managed headless sessions".into());
            }
            shrimply_project_document::project::save()?;
            Ok(json!({"saved":true,"closing":true}))
        }
        "history_undo" | "history_redo" => {
            if canceled.load(Ordering::Acquire) {
                return Err("request was canceled before history commit".into());
            }
            let changed = shrimply_project_document::project::change_history_saved(
                &mut project.borrow_mut(),
                call.name == "history_redo",
            )?;
            if changed {
                let project = project.borrow();
                let captured = EditorSelection::capture(selection, &project).reconcile(&project);
                captured.restore(selection, &project);
                player_state::refresh_project(
                    player,
                    ProjectChange {
                        duration: Some(project.duration()),
                        frame_rate: Some(project.fps),
                        audio: true,
                        video: true,
                        captions: true,
                        inspector: true,
                        audio_beats: true,
                        audio_waveforms: true,
                        live_preview: true,
                    },
                );
            }
            Ok(
                json!({"changed":changed,"can_undo":shrimply_project_document::project::can_undo(),"can_redo":shrimply_project_document::project::can_redo()}),
            )
        }
        "get_version" => serde_json::to_value(BuildVersion::current(env!("CARGO_PKG_VERSION")))
            .map_err(|error| error.to_string()),
        _ => Err(format!("{} is a CLI lifecycle command", call.name)),
    }?;
    let mut result = result;
    if let Some(revision) = query_revision
        && let Some(object) = result.as_object_mut()
    {
        object.insert("content_revision".into(), json!(revision));
    }
    Ok(result)
}
