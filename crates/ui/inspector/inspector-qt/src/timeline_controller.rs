use super::*;

pub(super) fn transform_live_presentation(
    target: &InspectorTarget,
) -> Option<shrimply_inspector_core::transform::TransformLivePresentation> {
    CONTROLLER.with_borrow(|controller| {
        controller
            .as_ref()
            .expect("Qt inspector transform requested before installation")
            .transform_live_presentation(target)
    })
}

pub(super) fn resolved_transform(
    target: &InspectorTarget,
) -> Option<shrimply_project_document::project::ResolvedTransform> {
    CONTROLLER.with_borrow(|controller| {
        controller
            .as_ref()
            .expect("Qt inspector transform requested before installation")
            .resolved_transform(target)
    })
}

pub(super) fn vector2_expression_output(
    target: &InspectorTarget,
    path: &str,
    timeline_id: Option<uuid::Uuid>,
) -> Result<shrimply_inspector_core::InspectorExpressionOutput<glam::Vec2>, String> {
    with_controller(|controller| controller.vector2_expression_output(target, path, timeline_id))
}

pub(super) fn transform_vec2_expression_output(
    target: &InspectorTarget,
    field: shrimply_inspector_core::transform::Vec2Field,
    timeline_id: uuid::Uuid,
) -> Result<Option<shrimply_inspector_core::InspectorExpressionOutput<glam::Vec2>>, String> {
    with_controller(|controller| {
        controller.transform_vec2_expression_output(target, field, timeline_id)
    })
}

pub(super) fn vector3_expression_output(
    target: &InspectorTarget,
    path: &str,
    timeline_id: uuid::Uuid,
) -> Result<shrimply_inspector_core::InspectorExpressionOutput<glam::Vec3>, String> {
    with_controller(|controller| controller.vector3_expression_output(target, path, timeline_id))
}

pub(super) fn color_expression_output(
    target: &InspectorTarget,
    path: &str,
    timeline_id: uuid::Uuid,
) -> Result<
    shrimply_inspector_core::InspectorExpressionOutput<shrimply_property_model::Color<u8>>,
    String,
> {
    with_controller(|controller| controller.color_expression_output(target, path, timeline_id))
}

pub(super) fn color_value(
    target: &InspectorTarget,
    path: &str,
    timeline_id: uuid::Uuid,
) -> Result<shrimply_property_model::Color<u8>, String> {
    with_controller(|controller| controller.color_value(target, path, timeline_id))
}

pub(super) fn timeline_number_value(
    target: &InspectorTarget,
    audio_modifier: bool,
    target_id: Option<uuid::Uuid>,
    timeline_id: Option<uuid::Uuid>,
    path: &str,
) -> Result<f64, String> {
    with_controller(|controller| {
        if audio_modifier {
            controller.audio_modifier_number_value(
                target,
                target_id
                    .ok_or_else(|| "audio modifier target is no longer available".to_string())?,
                timeline_id.ok_or_else(|| {
                    "audio modifier timeline ID is no longer available".to_string()
                })?,
            )
        } else {
            controller.timeline_number_value(target, path, timeline_id)
        }
    })
}

pub(super) fn timeline_vector2_value(
    target: &InspectorTarget,
    timeline_id: Option<uuid::Uuid>,
    path: &str,
) -> Result<glam::Vec2, String> {
    with_controller(|controller| controller.timeline_vector2_value(target, path, timeline_id))
}

pub(super) fn timeline_vector3_value(
    target: &InspectorTarget,
    timeline_id: uuid::Uuid,
    path: &str,
) -> Result<glam::Vec3, String> {
    with_controller(|controller| controller.timeline_vector3_value(target, path, timeline_id))
}

pub(super) fn scalar_expression_output(
    target: &InspectorTarget,
    path: &str,
    timeline_id: Option<uuid::Uuid>,
) -> Result<shrimply_inspector_core::InspectorExpressionOutput, String> {
    with_controller(|controller| controller.scalar_expression_output(target, path, timeline_id))
}

pub(super) fn transform_scalar_expression_output(
    target: &InspectorTarget,
    field: shrimply_inspector_core::transform::ScalarField,
    timeline_id: uuid::Uuid,
) -> Result<Option<shrimply_inspector_core::InspectorExpressionOutput>, String> {
    with_controller(|controller| {
        controller.transform_scalar_expression_output(target, field, timeline_id)
    })
}

pub(super) fn background_integer_expression_output(
    target: &InspectorTarget,
    path: &str,
    timeline_id: uuid::Uuid,
) -> Result<shrimply_inspector_core::InspectorExpressionOutput<u32>, String> {
    with_controller(|controller| {
        controller.background_integer_expression_output(target, path, timeline_id)
    })
}

pub(super) fn bool_expression_output(
    target: &InspectorTarget,
    path: &str,
) -> Result<shrimply_inspector_core::InspectorExpressionOutput<bool>, String> {
    with_controller(|controller| controller.bool_expression_output(target, path))
}

pub(super) fn text_expression_output(
    target: &InspectorTarget,
    path: &str,
    timeline_id: uuid::Uuid,
) -> Result<shrimply_inspector_core::InspectorExpressionOutput<String>, String> {
    with_controller(|controller| controller.text_expression_output(target, path, timeline_id))
}

pub(super) fn step_expression_output(
    target: &InspectorTarget,
    path: &str,
    timeline_id: Option<uuid::Uuid>,
) -> Result<shrimply_inspector_core::InspectorExpressionOutput<String>, String> {
    with_controller(|controller| controller.step_expression_output(target, path, timeline_id))
}
