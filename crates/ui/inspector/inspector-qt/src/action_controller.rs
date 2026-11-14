use super::*;

pub(super) fn current_keyframe_time(
    target: &InspectorTarget,
) -> Result<shrimply_project_document::project::Time, String> {
    with_controller(|controller| controller.current_keyframe_time(target))
}

pub(super) fn audio_modifier_expression_output(
    target: &InspectorTarget,
    modifier_id: uuid::Uuid,
    timeline_id: uuid::Uuid,
) -> Result<shrimply_inspector_core::InspectorExpressionOutput, String> {
    with_controller(|controller| {
        controller.audio_modifier_expression_output(target, modifier_id, timeline_id)
    })
}

pub(super) fn toggle_keyframe_playback() {
    with_controller(|controller| {
        controller.toggle_keyframe_playback();
        Ok(())
    })
    .expect("Qt inspector controller must be installed before graph playback");
}

pub(super) fn apply_project_settings(
    width: i32,
    height: i32,
    fps_numerator: &str,
    fps_denominator: &str,
) -> Result<(), String> {
    let dimensions = shrimply_project_document::project::CanvasSize {
        width: u32::try_from(width).map_err(|_| "project width must be positive".to_string())?,
        height: u32::try_from(height).map_err(|_| "project height must be positive".to_string())?,
    };
    let numerator = fps_numerator
        .parse::<i64>()
        .map_err(|_| format!("invalid project frame-rate numerator: {fps_numerator}"))?;
    let denominator = fps_denominator
        .parse::<i64>()
        .map_err(|_| format!("invalid project frame-rate denominator: {fps_denominator}"))?;
    if numerator <= 0 || denominator <= 0 {
        return Err("project frame rate must be positive".to_string());
    }
    let frame_rate = shrimply_math_core::fraction_new(numerator, denominator);
    with_controller(|controller| controller.apply_project_settings(dimensions, frame_rate))
}

pub(super) fn add_visual_modifier(
    target: &InspectorTarget,
    kind: &str,
) -> Result<uuid::Uuid, String> {
    with_controller(|controller| controller.add_visual_modifier(target, kind))
}

pub(super) fn paste_visual_modifiers(target: &InspectorTarget) -> Result<usize, String> {
    with_controller(|controller| {
        PROPERTY_CLIPBOARD.with_borrow(|clipboard| {
            controller.paste_visual_modifiers(
                target,
                clipboard
                    .as_ref()
                    .expect("Qt inspector paste requested before installation"),
            )
        })
    })
}

pub(super) fn add_audio_modifier(target: &InspectorTarget, kind: &str) -> Result<(), String> {
    with_controller(|controller| controller.add_audio_modifier(target, kind))
}

pub(super) fn paste_audio_modifiers(target: &InspectorTarget) -> Result<usize, String> {
    with_controller(|controller| {
        PROPERTY_CLIPBOARD.with_borrow(|clipboard| {
            controller.paste_audio_modifiers(
                target,
                clipboard
                    .as_ref()
                    .expect("Qt inspector paste requested before installation"),
            )
        })
    })
}

pub(super) fn toggle_audio_cache(target: &InspectorTarget, id: uuid::Uuid) -> Result<(), String> {
    with_controller(|controller| controller.toggle_audio_cache(target, id))
}

pub(super) fn toggle_visual_cache(target: &InspectorTarget, id: uuid::Uuid) -> Result<(), String> {
    with_controller(|controller| controller.toggle_visual_cache(target, id))
}

pub(super) fn toggle_sam2_analysis(target: &InspectorTarget, id: uuid::Uuid) -> Result<(), String> {
    let server_url = PREFERENCES.with_borrow(|preferences| {
        shrimply_editor_state::preferences::snapshot(
            preferences
                .as_ref()
                .expect("Qt inspector preferences requested before installation"),
        )
        .compute_server_url
    });
    with_controller(|controller| controller.toggle_sam2_analysis(target, id, server_url))
}

pub(super) fn transparent_fill_analysis_control(
    target: &InspectorTarget,
    id: uuid::Uuid,
) -> Result<shrimply_inspector_core::AnalysisControlPresentation, String> {
    with_controller(|controller| controller.transparent_fill_analysis_control(target, id))
}

pub(super) fn camera_analysis_control(
    target: &InspectorTarget,
) -> Result<shrimply_inspector_core::AnalysisControlPresentation, String> {
    let server_url = PREFERENCES.with_borrow(|preferences| {
        shrimply_editor_state::preferences::snapshot(
            preferences
                .as_ref()
                .expect("Qt camera inspector used before preferences were installed"),
        )
        .compute_server_url
    });
    with_controller(|controller| controller.camera_analysis_control(target, &server_url))
}

pub(super) fn toggle_camera_analysis(target: &InspectorTarget) -> Result<(), String> {
    let server_url = PREFERENCES.with_borrow(|preferences| {
        shrimply_editor_state::preferences::snapshot(
            preferences
                .as_ref()
                .expect("Qt camera inspector used before preferences were installed"),
        )
        .compute_server_url
    });
    with_controller(|controller| controller.toggle_camera_analysis(target, server_url))
}

pub(super) fn toggle_transparent_fill_analysis(
    target: &InspectorTarget,
    id: uuid::Uuid,
) -> Result<(), String> {
    with_controller(|controller| controller.toggle_transparent_fill_analysis(target, id))
}

pub(super) fn perform_action(
    target: &InspectorTarget,
    action: InspectorAction,
) -> Result<Option<String>, String> {
    let feedback = with_controller(|controller| controller.apply_basic_action(target, &action))?;
    if let InspectorAction::SetAlphaMask {
        target: mask_target,
        enabled,
    } = action
    {
        focus_alpha_mask(target, mask_target, enabled);
    }
    crate::mark_dirty();
    Ok(feedback)
}
