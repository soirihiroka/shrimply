use super::*;

pub(super) fn current_keyframe_time(
    target: &InspectorTarget,
) -> Result<shrimply_project_document::project::Time, String> {
    with_controller(|controller| controller.current_keyframe_time(target))
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
    let frame_rate = format!("{fps_numerator}/{fps_denominator}")
        .parse()
        .map_err(|_| "invalid project frame rate".to_string())?;
    with_controller(|controller| {
        controller.stage_project_dimensions(dimensions.width, dimensions.height)?;
        controller.stage_project_frame_rate(frame_rate)?;
        controller.apply_staged_project_settings()
    })
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

pub(super) fn activate_control(
    target: &InspectorTarget,
    control: &shrimply_inspector_core::InspectorControl,
) -> Result<(), String> {
    let server_url = PREFERENCES.with_borrow(|preferences| {
        shrimply_editor_state::preferences::snapshot(
            preferences
                .as_ref()
                .expect("Qt inspector preferences requested before installation"),
        )
        .compute_server_url
    });
    with_controller(|controller| controller.activate_control(target, control, &server_url))
}
