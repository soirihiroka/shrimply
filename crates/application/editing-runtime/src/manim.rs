use super::*;

pub(super) async fn get_manim_clip(
    live: &Rc<RefCell<Project>>,
    request: GetManimClipRequest,
    canceled: Arc<AtomicBool>,
) -> Result<Value, String> {
    let address = shrimply_editing_api::query::model_item_address(&request.address)?;
    let (source, source_revision, scene, input_parameters, parameters, error) = {
        let project = live.borrow();
        let item = project
            .video_item(&address)
            .ok_or_else(|| "get_manim_clip requires a video clip address".to_string())?;
        let VideoItemContent::Manim(manim) = &item.content else {
            return Err("addressed clip is not a Manim clip".to_string());
        };
        let source_revision = item.file.snapshot()?.revision();
        (
            item.file.clone(),
            source_revision,
            manim.scene.clone(),
            manim.parameters.clone(),
            shrimply_editor_state::manim_status::parameters(
                item.id,
                source_revision,
                &manim.scene,
                &manim.parameters,
            ),
            shrimply_editor_state::manim_status::error(
                item.id,
                source_revision,
                &manim.scene,
                &manim.parameters,
            ),
        )
    };
    let scenes = discover_manim_scenes(source.clone(), canceled.clone()).await?;
    if canceled.load(Ordering::Acquire) {
        return Err("editing API client canceled Manim inspection".to_string());
    }
    {
        let project = live.borrow();
        let item = project
            .video_item(&address)
            .ok_or_else(|| "Manim clip changed while it was being inspected".to_string())?;
        let VideoItemContent::Manim(manim) = &item.content else {
            return Err("clip stopped being a Manim clip while it was being inspected".to_string());
        };
        if item.file.snapshot()?.revision() != source_revision
            || manim.scene != scene
            || manim.parameters != input_parameters
        {
            return Err("Manim clip changed while it was being inspected; retry".to_string());
        }
    }
    let parameters_ready = parameters.is_some();
    let parameters = parameters
        .unwrap_or_default()
        .into_iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("could not serialize reflected Manim parameter: {error}"))?;
    serde_json::to_value(ManimClipResponse {
        address: request.address,
        source: source.path().to_string_lossy().into_owned(),
        source_revision,
        scene,
        scenes,
        parameters_ready,
        parameters,
        error,
    })
    .map_err(|error| format!("could not serialize Manim clip metadata: {error}"))
}

pub(super) async fn set_manim_clip(
    live: &Rc<RefCell<Project>>,
    player: &SharedPlayerState,
    request: SetManimClipRequest,
    canceled: Arc<AtomicBool>,
) -> Result<Value, String> {
    if request.scene.is_none() && request.parameters.is_empty() {
        return Err("set_manim_clip requires a scene or parameter change".to_string());
    }
    let address = shrimply_editing_api::query::model_item_address(&request.address)?;
    let (original, source, source_revision, current_scene, definitions) = {
        let project = live.borrow();
        let item = project
            .video_item(&address)
            .ok_or_else(|| "set_manim_clip requires a video clip address".to_string())?;
        let VideoItemContent::Manim(manim) = &item.content else {
            return Err("addressed clip is not a Manim clip".to_string());
        };
        let source_revision = item.file.snapshot()?.revision();
        (
            content_revision(&project)?,
            item.file.clone(),
            source_revision,
            manim.scene.clone(),
            shrimply_editor_state::manim_status::parameters(
                item.id,
                source_revision,
                &manim.scene,
                &manim.parameters,
            ),
        )
    };
    if let Some(scene) = &request.scene {
        let scenes = discover_manim_scenes(source, canceled.clone()).await?;
        if !scenes.contains(scene) {
            return Err(format!("Manim scene {scene:?} was not found in the source"));
        }
        if scene != &current_scene && !request.parameters.is_empty() {
            return Err(
                "set the Manim scene first, render it, then set its reflected parameters"
                    .to_string(),
            );
        }
    }
    if canceled.load(Ordering::Acquire) {
        return Err("editing API client canceled the Manim edit".to_string());
    }

    let parameter_values = if request.parameters.is_empty() {
        Vec::new()
    } else {
        let definitions = definitions.ok_or_else(|| {
            "Manim parameters are not ready; render the current scene with view_frame, then retry"
                .to_string()
        })?;
        request
            .parameters
            .iter()
            .map(|(key, value)| {
                let definition = definitions
                    .iter()
                    .find(|parameter| &parameter.key == key)
                    .ok_or_else(|| format!("Manim parameter {key:?} was not reflected"))?;
                value
                    .as_ref()
                    .map(|value| manim_parameter_value(definition, value))
                    .transpose()
                    .map(|value| (key.clone(), value))
            })
            .collect::<Result<Vec<_>, String>>()?
    };

    let mut project = live.borrow().clone();
    if content_revision(&project)? != original {
        return Err("project changed while the Manim edit was being prepared; retry".to_string());
    }
    let item = project
        .video_item_mut(&address)
        .expect("validated Manim clip must still exist in an unchanged project");
    if item.file.snapshot()?.revision() != source_revision {
        return Err("Manim source changed while the edit was being prepared; retry".to_string());
    }
    let VideoItemContent::Manim(manim) = &mut item.content else {
        unreachable!("validated Manim clip must keep its content kind");
    };
    let mut changed = false;
    if let Some(scene) = request.scene
        && manim.scene != scene
    {
        manim.scene = scene;
        manim.parameters.clear();
        changed = true;
    }
    for (key, value) in parameter_values {
        match value {
            Some(value) if manim.parameters.get(&key) != Some(&value) => {
                manim.parameters.insert(key, value);
                changed = true;
            }
            None if manim.parameters.remove(&key).is_some() => changed = true,
            Some(_) | None => {}
        }
    }
    if !changed {
        return Err("requested Manim values are already set".to_string());
    }

    let item_id = address.item_id();
    shrimply_project_document::project::commit_edit_saved(&project, "editing API set Manim clip")?;
    *live.borrow_mut() = project;
    player_state::refresh_project(
        player,
        ProjectChange {
            video: true,
            inspector: true,
            ..Default::default()
        },
    );
    let project = live.borrow();
    let presentations = shrimply_editing_api::query::presentations_affected_by_items(
        &project,
        &HashSet::from([item_id]),
    )?;
    let response = EditResponse {
        operations: vec![EditOperationResult {
            details: None,
            index: 0,
            operation: "set_manim_clip".to_string(),
            changed_addresses: presentations
                .iter()
                .map(|clip| clip.address.clone())
                .collect(),
            deleted_addresses: Vec::new(),
            changed_tracks: Vec::new(),
            presentations,
        }],
        duration: shrimply_editing_api::query::frame_time_from_time(
            project.duration(),
            project.fps,
            true,
        ),
        revision: player_state::snapshot(player).revision,
    };
    serde_json::to_value(response)
        .map_err(|error| format!("could not serialize Manim edit result: {error}"))
}

pub(super) fn reload_manim_source(
    live: &Rc<RefCell<Project>>,
    request: ReloadManimSourceRequest,
) -> Result<Value, String> {
    let address = shrimply_editing_api::query::model_item_address(&request.address)?;
    let source = {
        let project = live.borrow();
        let item = project
            .video_item(&address)
            .ok_or_else(|| "reload_manim_source requires a video clip address".to_string())?;
        if !matches!(item.content, VideoItemContent::Manim(_)) {
            return Err("addressed clip is not a Manim clip".to_string());
        }
        item.file.clone()
    };
    shrimply_manim_bridge::invalidate_ir_cache(&source)?;
    source.mark_dirty()?;
    serde_json::to_value(ReloadManimSourceResponse {
        address: request.address,
        source_revision: source.snapshot()?.revision(),
    })
    .map_err(|error| format!("could not serialize Manim reload result: {error}"))
}

pub(super) async fn discover_manim_scenes(
    source: Asset,
    canceled: Arc<AtomicBool>,
) -> Result<Vec<String>, String> {
    let (sender, receiver) = async_channel::bounded(1);
    thread::Builder::new()
        .name("shrimply-editing-manim-scenes".to_string())
        .spawn(move || {
            let result = if canceled.load(Ordering::Acquire) {
                Err("editing API client canceled Manim scene discovery".to_string())
            } else {
                shrimply_manim_bridge::discover_scenes(&source)
            };
            let _ = sender.send_blocking(result);
        })
        .map_err(|error| format!("could not start Manim scene discovery: {error}"))?;
    receiver
        .recv()
        .await
        .map_err(|_| "Manim scene discovery stopped without a result".to_string())?
}

pub(super) fn manim_parameter_value(
    parameter: &ManimParameter,
    value: &ProtocolManimParameterValue,
) -> Result<ManimParameterValue, String> {
    let invalid = || {
        format!(
            "value for Manim parameter {:?} does not match its reflected control",
            parameter.key
        )
    };
    match (&parameter.control, value) {
        (ManimParameterControl::AntiAliasing, ProtocolManimParameterValue::Integer(value))
            if MANIM_ANTI_ALIASING_SAMPLES.contains(value) =>
        {
            Ok(ManimParameterValue::Integer(*value))
        }
        (
            ManimParameterControl::Integer {
                minimum, maximum, ..
            },
            ProtocolManimParameterValue::Integer(value),
        ) if minimum.is_none_or(|minimum| *value >= minimum)
            && maximum.is_none_or(|maximum| *value <= maximum) =>
        {
            Ok(ManimParameterValue::Integer(*value))
        }
        (
            ManimParameterControl::Float {
                minimum, maximum, ..
            },
            ProtocolManimParameterValue::Float(value),
        ) if value.is_finite()
            && minimum.is_none_or(|minimum| *value >= minimum)
            && maximum.is_none_or(|maximum| *value <= maximum) =>
        {
            Ok(ManimParameterValue::Float(*value))
        }
        (ManimParameterControl::Fraction, ProtocolManimParameterValue::Fraction(value))
            if value.denominator != 0 =>
        {
            let value = fraction_new(value.numerator, value.denominator);
            Ok(ManimParameterValue::Fraction {
                numerator: fraction_numerator(value),
                denominator: fraction_denominator(value),
            })
        }
        (ManimParameterControl::Color, ProtocolManimParameterValue::Color(value)) => {
            Ok(ManimParameterValue::Color(
                shrimply_project_document::project::Color::new(value.r, value.g, value.b, u8::MAX),
            ))
        }
        (ManimParameterControl::Option { options }, ProtocolManimParameterValue::Option(value))
            if options.contains(value) =>
        {
            Ok(ManimParameterValue::Option(value.clone()))
        }
        (ManimParameterControl::Boolean, ProtocolManimParameterValue::Boolean(value)) => {
            Ok(ManimParameterValue::Boolean(*value))
        }
        (ManimParameterControl::String, ProtocolManimParameterValue::String(value)) => {
            Ok(ManimParameterValue::String(value.clone()))
        }
        _ => Err(invalid()),
    }
}
