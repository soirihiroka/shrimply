use super::*;
use shrimply_editing_api::protocol::ReplaceClipSourceRequest;

pub(super) fn apply(
    prepared: &mut PreparedEdit,
    request: &ReplaceClipSourceRequest,
    default_duration: Time,
    project_path: &Path,
) -> Result<MutationResult, String> {
    let address =
        shrimply_editing_api::edit::source_replacement_target(&prepared.project, request)?;
    let entry = ImportEntry {
        source: request.source.clone(),
        offset_frames: 0,
        targets: Vec::new(),
        properties: InitialClipProperties::default(),
    };
    let (mut sources, staged) = stage_sources(
        &[entry],
        request.link,
        request.copy_root.as_deref(),
        project_path,
    )?;
    if let Some(staged) = staged {
        prepared.staged.push(staged);
    }
    let mut source = sources.pop().expect("one replacement source was staged");
    let kind = native::file_kind(&source.inspect).ok_or_else(|| {
        format!(
            "unsupported replacement source: {}",
            source.inspect.display()
        )
    })?;
    remux_source(&mut source, kind, request.link)?;
    let info = native::inspect(
        source.inspect,
        prepared.project.canvas_size,
        default_duration,
    )?;
    if info.video_streams != 1 {
        return Err(format!(
            "replacement requires exactly one video/visual stream; source has {}. Audio-only files and multi-stream sources cannot replace this video clip",
            info.video_streams
        ));
    }
    if info.visual_kind == Some(native::VisualMediaKind::Obj) {
        return Err("OBJ source replacement requires new scene modifiers; insert the OBJ as a new clip instead".into());
    }
    let size = info
        .video_sizes
        .first()
        .filter(|size| size.x > 0 && size.y > 0)
        .ok_or("replacement source has no valid visual dimensions")?;
    if request.link {
        prepared.linked_sources.push(info.snapshot.clone());
    }
    let item = prepared
        .project
        .video_item_mut(&address)
        .ok_or("replacement target video clip was not found")?;
    let old_source = item.file.path().to_path_buf();
    item.file = Asset::new(source.stored);
    item.content = native::video_content_for_import(&info);
    item.source_duration = info.duration;
    item.source_width = size.x;
    item.source_height = size.y;
    item.track_id = 0;
    item.svg_color_overrides.clear();
    item.video_generation = None;
    Ok(MutationResult {
        changed_item_ids: vec![item.id],
        details: Some(
            serde_json::json!({"old_source":old_source,"source":item.file.path(),"source_specific_settings":"reset","timeline_range":"preserved"}),
        ),
        ..Default::default()
    })
}

#[cfg(test)]
mod tests;
