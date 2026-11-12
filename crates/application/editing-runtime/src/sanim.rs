//! Typed reflected Sanim settings use the same metadata and validation as the inspector.
use super::*;
use shrimply_editing_api::protocol::{GetSanimClipRequest, SanimClipResponse, SetSanimClipRequest};
use shrimply_visual_core::sanim::reflection;

async fn inspect_sanim(
    live: &Rc<RefCell<Project>>,
    address: &shrimply_editing_api::protocol::ClipAddress,
    canceled: Arc<AtomicBool>,
    inspection: reflection::Inspection,
) -> Result<(ItemAddress, VideoItemSnapshot, reflection::Snapshot), String> {
    let address = shrimply_editing_api::query::model_item_address(address)?;
    let (item, viewport) = {
        let project = live.borrow();
        let item = project
            .video_item(&address)
            .ok_or("Sanim clip is no longer available")?
            .clone();
        if !matches!(item.content, VideoItemContent::Sanim(_)) {
            return Err("addressed clip is not a Sanim clip".into());
        }
        (
            item,
            reflection::viewport(
                project.canvas_size.width,
                project.canvas_size.height,
                project.fps,
            ),
        )
    };
    let source_revision = item.file.snapshot()?.revision();
    let expected = item.clone();
    let (sender, receiver) = async_channel::bounded(1);
    thread::Builder::new()
        .name("editing-sanim-reflection".into())
        .spawn(move || {
            let result = reflection::inspect(&item, viewport, &canceled, inspection);
            let _ = sender.send_blocking(result);
        })
        .map_err(|e| e.to_string())?;
    let state = receiver.recv().await.map_err(|e| e.to_string())??;
    {
        let project = live.borrow();
        let current = project
            .video_item(&address)
            .ok_or("Sanim clip changed during inspection; retry")?;
        let (VideoItemContent::Sanim(a), VideoItemContent::Sanim(b)) =
            (&expected.content, &current.content)
        else {
            return Err("clip stopped being a Sanim clip during inspection".into());
        };
        if current.file != expected.file
            || current.file.snapshot()?.revision() != source_revision
            || a.parameters != b.parameters
        {
            return Err("Sanim source or overrides changed during inspection; retry".into());
        }
    }
    Ok((
        address,
        VideoItemSnapshot {
            item: expected,
            source_revision,
        },
        state,
    ))
}
struct VideoItemSnapshot {
    item: shrimply_project_document::project::VideoItem,
    source_revision: u64,
}

pub(super) async fn get_sanim_clip(
    live: &Rc<RefCell<Project>>,
    request: GetSanimClipRequest,
    canceled: Arc<AtomicBool>,
) -> Result<Value, String> {
    let (_, snapshot, state) = inspect_sanim(
        live,
        &request.address,
        canceled,
        reflection::Inspection::Values,
    )
    .await?;
    let mut error = state.diagnostic.as_ref().map(ToString::to_string);
    let mut parameters = Vec::new();
    if let Some(prepared) = &state.prepared {
        let VideoItemContent::Sanim(settings) = &snapshot.item.content else {
            unreachable!()
        };
        let inputs = match reflection::effective_inputs(prepared, &settings.parameters) {
            Ok(inputs) => inputs,
            Err(message) => {
                error = Some(message);
                prepared.reflected.iter().cloned().collect()
            }
        };
        parameters = inputs
            .iter()
            .map(reflection::parameter_json)
            .collect::<Result<Vec<_>, _>>()?;
    }
    serde_json::to_value(SanimClipResponse {
        address: request.address,
        source: snapshot.item.file.path().to_string_lossy().into_owned(),
        source_revision: snapshot.source_revision,
        parameters_ready: state.prepared.is_some(),
        preparing: state.preparing,
        parameters,
        error,
    })
    .map_err(|e| e.to_string())
}
pub(super) async fn set_sanim_clip(
    live: &Rc<RefCell<Project>>,
    player: &SharedPlayerState,
    request: SetSanimClipRequest,
    canceled: Arc<AtomicBool>,
) -> Result<Value, String> {
    if request.parameters.is_empty() && request.reset_keys.is_empty() {
        return Err("sanim set requires parameter values or reset_keys".into());
    }
    let original = content_revision(&live.borrow())?;
    let (address, snapshot, state) = inspect_sanim(
        live,
        &request.address,
        canceled.clone(),
        reflection::Inspection::Schema,
    )
    .await?;
    if canceled.load(Ordering::Acquire) {
        return Err("Sanim edit canceled".into());
    }
    let prepared = state.prepared.ok_or_else(|| {
        state
            .diagnostic
            .map_or("Sanim reflection is not ready".into(), |e| e.to_string())
    })?;
    let mut project = live.borrow().clone();
    if content_revision(&project)? != original {
        return Err("project changed while the Sanim edit was being prepared; retry".into());
    }
    let item = project
        .video_item_mut(&address)
        .ok_or("Sanim clip is no longer available")?;
    if item.file.snapshot()?.revision() != snapshot.source_revision {
        return Err("Sanim source changed while editing; retry".into());
    }
    if !reflection::patch(item, &prepared, &request.parameters, &request.reset_keys)? {
        return Err("requested Sanim values are already set".into());
    }
    shrimply_project_document::project::commit_edit_saved(
        &project,
        "editing API set Sanim inputs",
    )?;
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
        &HashSet::from([address.item_id()]),
    )?;
    serde_json::to_value(EditResponse {
        operations: vec![EditOperationResult {
            details: None,
            index: 0,
            operation: "set_sanim_clip".into(),
            changed_addresses: presentations.iter().map(|p| p.address.clone()).collect(),
            deleted_addresses: vec![],
            changed_tracks: vec![],
            presentations,
        }],
        duration: shrimply_editing_api::query::frame_time_from_time(
            project.duration(),
            project.fps,
            true,
        ),
        revision: player_state::snapshot(player).revision,
    })
    .map_err(|e| e.to_string())
}
