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

pub(super) fn set_fraction(
    target: &InspectorTarget,
    path: &str,
    numerator: i64,
    denominator: i64,
) -> Result<(), String> {
    if denominator <= 0 {
        return Err("inspector fraction denominator must be positive".to_string());
    }
    with_controller(|controller| {
        controller.set_fraction(
            target,
            path,
            shrimply_math_core::fraction_new(numerator, denominator),
        )
    })
}

pub(super) fn set_timeline_mode(
    target: &InspectorTarget,
    path: &str,
    keyframes: bool,
    enabled: bool,
    current: Value,
    default_expression: &str,
    commit_name: &str,
) -> Result<(), String> {
    with_controller(|controller| {
        controller.set_timeline_mode_with_commit(
            target,
            path,
            shrimply_inspector_core::TimelineModeChange {
                keyframes,
                enabled,
                current,
                default_expression,
            },
            shrimply_inspector_core::InspectorCommit::Immediate(commit_name),
        )
    })
}

pub(super) fn set_scalar_keyframes_enabled(
    target: &InspectorTarget,
    path: &str,
    enabled: bool,
    constraint: shrimply_inspector_core::NumberConstraint,
    commit_name: &str,
) -> Result<(), String> {
    with_controller(|controller| {
        controller.set_scalar_keyframes_enabled(
            target,
            path,
            enabled,
            constraint,
            shrimply_inspector_core::InspectorCommit::Immediate(commit_name),
        )
    })
}

pub(super) fn set_background_integer_keyframes_enabled(
    target: &InspectorTarget,
    path: &str,
    timeline_id: uuid::Uuid,
    enabled: bool,
) -> Result<(), String> {
    with_controller(|controller| {
        controller.set_background_integer_keyframes_enabled(target, path, timeline_id, enabled)
    })
}

pub(super) fn set_background_integer_value(
    target: &InspectorTarget,
    path: &str,
    timeline_id: uuid::Uuid,
    value: u32,
) -> Result<(), String> {
    with_controller(|controller| {
        controller.set_background_integer_value(target, path, timeline_id, value)
    })
}

pub(super) fn commit_background_integer_value(
    target: &InspectorTarget,
    path: &str,
    timeline_id: uuid::Uuid,
) -> Result<(), String> {
    with_controller(|controller| {
        controller.commit_background_integer_value(target, path, timeline_id)
    })
}

pub(super) fn set_background_integer_expression_enabled(
    target: &InspectorTarget,
    path: &str,
    timeline_id: uuid::Uuid,
    enabled: bool,
) -> Result<(), String> {
    with_controller(|controller| {
        controller.set_background_integer_expression_enabled(target, path, timeline_id, enabled)
    })
}

pub(super) fn set_background_integer_expression_source(
    target: &InspectorTarget,
    path: &str,
    timeline_id: uuid::Uuid,
    source: String,
) -> Result<(), String> {
    with_controller(|controller| {
        controller.set_background_integer_expression_source(target, path, timeline_id, source)
    })
}

pub(super) fn set_vector2_keyframes_enabled(
    target: &InspectorTarget,
    path: &str,
    enabled: bool,
    commit_name: &str,
) -> Result<(), String> {
    with_controller(|controller| {
        controller.set_vector2_keyframes_enabled(
            target,
            path,
            enabled,
            shrimply_inspector_core::InspectorCommit::Immediate(commit_name),
        )
    })
}

pub(super) fn set_vector2_expression_enabled(
    target: &InspectorTarget,
    path: &str,
    enabled: bool,
    commit_name: &str,
) -> Result<(), String> {
    with_controller(|controller| {
        controller.set_vector2_expression_enabled(
            target,
            path,
            enabled,
            shrimply_inspector_core::InspectorCommit::Immediate(commit_name),
        )
    })
}

pub(super) fn set_transform_expression_enabled(
    target: &InspectorTarget,
    field: shrimply_inspector_core::transform::TransformField,
    timeline_id: uuid::Uuid,
    enabled: bool,
) -> Result<(), String> {
    let result = with_controller(|controller| {
        controller.set_transform_expression_enabled(
            target,
            field,
            timeline_id,
            enabled,
            shrimply_inspector_core::InspectorCommit::Immediate(
                shrimply_inspector_core::transform::expressions::TOGGLE_COMMIT,
            ),
        )
    });
    if result.is_ok() {
        mark_dirty();
        EXPRESSION_DIRTY.set(true);
    }
    result
}

pub(super) fn set_vector3_keyframes_enabled(
    target: &InspectorTarget,
    path: &str,
    enabled: bool,
    commit_name: &str,
) -> Result<(), String> {
    with_controller(|controller| {
        controller.set_vector3_keyframes_enabled(
            target,
            path,
            enabled,
            shrimply_inspector_core::InspectorCommit::Immediate(commit_name),
        )
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

pub(super) fn set_color_keyframes_enabled(
    target: &InspectorTarget,
    path: &str,
    timeline_id: uuid::Uuid,
    enabled: bool,
    commit_name: &str,
) -> Result<(), String> {
    with_controller(|controller| {
        controller.set_color_keyframes_enabled(
            target,
            path,
            timeline_id,
            enabled,
            shrimply_inspector_core::InspectorCommit::Immediate(commit_name),
        )
    })
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

pub(super) fn set_bool_value(
    target: &InspectorTarget,
    path: &str,
    value: bool,
) -> Result<(), String> {
    let result = with_controller(|controller| controller.set_bool_value(target, path, value));
    if result.is_ok() {
        EXPRESSION_DIRTY.set(true);
    }
    result
}

pub(super) fn set_bool_keyframes_enabled(
    target: &InspectorTarget,
    path: &str,
    enabled: bool,
    commit_name: &str,
) -> Result<(), String> {
    with_controller(|controller| {
        controller.set_bool_keyframes_enabled(
            target,
            path,
            enabled,
            shrimply_inspector_core::InspectorCommit::Immediate(commit_name),
        )
    })
}

pub(super) fn set_text_value(
    target: &InspectorTarget,
    path: &str,
    timeline_id: uuid::Uuid,
    value: String,
    commit_name: &str,
) -> Result<(), String> {
    let result = with_controller(|controller| {
        controller.set_text_value(
            target,
            path,
            timeline_id,
            value,
            shrimply_inspector_core::InspectorCommit::Coalesced(commit_name),
        )
    });
    if result.is_ok() {
        EXPRESSION_DIRTY.set(true);
        GRAPH_DIRTY.set(true);
    }
    result
}

pub(super) fn set_text_keyframes_enabled(
    target: &InspectorTarget,
    path: &str,
    timeline_id: uuid::Uuid,
    enabled: bool,
    commit_name: &str,
) -> Result<(), String> {
    let result = with_controller(|controller| {
        controller.set_text_keyframes_enabled(
            target,
            path,
            timeline_id,
            enabled,
            shrimply_inspector_core::InspectorCommit::Immediate(commit_name),
        )
    });
    if result.is_ok() {
        EXPRESSION_DIRTY.set(true);
        GRAPH_DIRTY.set(true);
    }
    result
}

pub(super) fn set_step_keyframes_enabled(
    target: &InspectorTarget,
    path: &str,
    enabled: bool,
    commit_name: &str,
) -> Result<(), String> {
    with_controller(|controller| {
        controller.set_step_keyframes_enabled_with_commit(
            target,
            path,
            enabled,
            shrimply_inspector_core::InspectorCommit::Immediate(commit_name),
        )
    })
}

pub(super) fn set_expression_source(
    target: &InspectorTarget,
    path: &str,
    source: &str,
    commit_name: &str,
) -> Result<(), String> {
    let result = with_controller(|controller| {
        controller.set_expression_source_with_commit(
            target,
            path,
            source,
            shrimply_inspector_core::InspectorCommit::Coalesced(commit_name),
        )
    });
    if result.is_ok() {
        EXPRESSION_DIRTY.set(true);
    }
    result
}

pub(super) fn set_vector2_expression_source(
    target: &InspectorTarget,
    path: &str,
    source: String,
    commit_name: &str,
) -> Result<(), String> {
    let result = with_controller(|controller| {
        controller.set_vector2_expression_source(
            target,
            path,
            source,
            shrimply_inspector_core::InspectorCommit::Coalesced(commit_name),
        )
    });
    if result.is_ok() {
        EXPRESSION_DIRTY.set(true);
    }
    result
}

pub(super) fn set_transform_expression_source(
    target: &InspectorTarget,
    field: shrimply_inspector_core::transform::TransformField,
    timeline_id: uuid::Uuid,
    source: String,
) -> Result<(), String> {
    let result = with_controller(|controller| {
        controller.set_transform_expression_source(
            target,
            field,
            timeline_id,
            source,
            shrimply_inspector_core::InspectorCommit::Coalesced(
                shrimply_inspector_core::transform::expressions::SOURCE_COMMIT,
            ),
        )
    });
    if result.is_ok() {
        mark_dirty();
        EXPRESSION_DIRTY.set(true);
    }
    result
}

pub(super) fn set_text_expression_source(
    target: &InspectorTarget,
    path: &str,
    timeline_id: uuid::Uuid,
    source: &str,
    commit_name: &str,
) -> Result<(), String> {
    let result = with_controller(|controller| {
        controller.set_text_expression_source(
            target,
            path,
            timeline_id,
            source.to_string(),
            shrimply_inspector_core::InspectorCommit::Coalesced(commit_name),
        )
    });
    if result.is_ok() {
        EXPRESSION_DIRTY.set(true);
    }
    result
}

pub(super) fn set_text_expression_enabled(
    target: &InspectorTarget,
    path: &str,
    timeline_id: uuid::Uuid,
    enabled: bool,
    commit_name: &str,
) -> Result<(), String> {
    with_controller(|controller| {
        controller.set_text_expression_enabled(
            target,
            path,
            timeline_id,
            enabled,
            shrimply_inspector_core::InspectorCommit::Immediate(commit_name),
        )
    })
}

pub(super) fn set_timeline_base(
    target: &InspectorTarget,
    path: &str,
    value: Value,
    commit_name: &str,
    commit_immediately: bool,
) -> Result<(), String> {
    let result = with_controller(|controller| {
        let commit = if commit_immediately {
            shrimply_inspector_core::InspectorCommit::Immediate(commit_name)
        } else {
            shrimply_inspector_core::InspectorCommit::Coalesced(commit_name)
        };
        controller.set_timeline_base_with_commit(target, path, value, commit)
    });
    if result.is_ok() {
        EXPRESSION_DIRTY.set(true);
        GRAPH_DIRTY.set(true);
    }
    result
}

pub(super) fn set_audio_modifier_field(
    target: &InspectorTarget,
    id: uuid::Uuid,
    path: &str,
    value: &str,
) -> Result<(), String> {
    let result =
        with_controller(|controller| controller.set_audio_modifier_field(target, id, path, value));
    if result.is_ok() {
        EXPRESSION_DIRTY.set(true);
    }
    result
}

pub(super) fn set_audio_modifier_live_field(
    target: &InspectorTarget,
    id: uuid::Uuid,
    path: &str,
    value: &str,
) -> Result<(), String> {
    with_controller(|controller| controller.set_audio_modifier_live_field(target, id, path, value))
}

pub(super) fn set_audio_modifier_timeline_base(
    target: &InspectorTarget,
    modifier_id: uuid::Uuid,
    timeline_id: uuid::Uuid,
    value: Value,
) -> Result<(), String> {
    let result = with_controller(|controller| {
        controller.set_audio_modifier_timeline_base(
            target,
            modifier_id,
            timeline_id,
            serde_json::from_value(value)
                .map_err(|error| format!("invalid audio modifier scalar: {error}"))?,
        )
    });
    if result.is_ok() {
        EXPRESSION_DIRTY.set(true);
    }
    result
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

pub(super) fn move_scalar_keyframe(
    target: &InspectorTarget,
    path: &str,
    change: shrimply_inspector_core::AudioModifierKeyframeMove,
    constraint: shrimply_inspector_core::NumberConstraint,
    commit_name: &str,
) -> Result<(), String> {
    let result = with_controller(|controller| {
        controller.move_scalar_keyframe(
            target,
            path,
            change,
            constraint,
            shrimply_inspector_core::InspectorCommit::Coalesced(commit_name),
        )
    });
    if result.is_ok() {
        EXPRESSION_DIRTY.set(true);
        GRAPH_DIRTY.set(true);
    }
    result
}

pub(super) fn move_background_integer_keyframe(
    target: &InspectorTarget,
    path: &str,
    timeline_id: uuid::Uuid,
    change: shrimply_inspector_core::AudioModifierKeyframeMove,
) -> Result<(), String> {
    let result = with_controller(|controller| {
        controller.move_background_integer_keyframe(target, path, timeline_id, change)
    });
    if result.is_ok() {
        EXPRESSION_DIRTY.set(true);
        GRAPH_DIRTY.set(true);
    }
    result
}

pub(super) fn delete_scalar_keyframe(
    target: &InspectorTarget,
    path: &str,
    time: shrimply_project_document::project::Time,
    commit_name: &str,
) -> Result<(), String> {
    let result = with_controller(|controller| {
        controller.delete_scalar_keyframe(
            target,
            path,
            time,
            shrimply_inspector_core::InspectorCommit::Immediate(commit_name),
        )
    });
    if result.is_ok() {
        EXPRESSION_DIRTY.set(true);
        GRAPH_DIRTY.set(true);
    }
    result
}

pub(super) fn delete_background_integer_keyframe(
    target: &InspectorTarget,
    path: &str,
    timeline_id: uuid::Uuid,
    time: shrimply_project_document::project::Time,
) -> Result<(), String> {
    let result = with_controller(|controller| {
        controller.delete_background_integer_keyframe(target, path, timeline_id, time)
    });
    if result.is_ok() {
        EXPRESSION_DIRTY.set(true);
        GRAPH_DIRTY.set(true);
    }
    result
}

pub(super) fn add_scalar_keyframe(
    target: &InspectorTarget,
    path: &str,
    time: shrimply_project_document::project::Time,
    constraint: shrimply_inspector_core::NumberConstraint,
    commit_name: &str,
) -> Result<(), String> {
    let result = with_controller(|controller| {
        controller.add_scalar_keyframe(
            target,
            path,
            time,
            constraint,
            shrimply_inspector_core::InspectorCommit::Immediate(commit_name),
        )
    });
    if result.is_ok() {
        EXPRESSION_DIRTY.set(true);
        GRAPH_DIRTY.set(true);
    }
    result
}

pub(super) fn add_background_integer_keyframe(
    target: &InspectorTarget,
    path: &str,
    timeline_id: uuid::Uuid,
    time: shrimply_project_document::project::Time,
) -> Result<(), String> {
    let result = with_controller(|controller| {
        controller.add_background_integer_keyframe(target, path, timeline_id, time)
    });
    if result.is_ok() {
        EXPRESSION_DIRTY.set(true);
        GRAPH_DIRTY.set(true);
    }
    result
}

pub(super) fn set_scalar_keyframe_interpolation(
    target: &InspectorTarget,
    path: &str,
    owner_id: uuid::Uuid,
    interpolation: usize,
    commit_name: &str,
) -> Result<(), String> {
    let result = with_controller(|controller| {
        controller.set_scalar_keyframe_interpolation(
            target,
            path,
            owner_id,
            interpolation,
            shrimply_inspector_core::InspectorCommit::Immediate(commit_name),
        )
    });
    if result.is_ok() {
        EXPRESSION_DIRTY.set(true);
        GRAPH_DIRTY.set(true);
    }
    result
}

pub(super) fn set_background_integer_interpolation(
    target: &InspectorTarget,
    path: &str,
    timeline_id: uuid::Uuid,
    owner_id: uuid::Uuid,
    interpolation: usize,
) -> Result<(), String> {
    let result = with_controller(|controller| {
        controller.set_background_integer_interpolation(
            target,
            path,
            timeline_id,
            owner_id,
            interpolation,
        )
    });
    if result.is_ok() {
        EXPRESSION_DIRTY.set(true);
        GRAPH_DIRTY.set(true);
    }
    result
}

pub(super) fn copy_scalar_keyframes(
    target: &InspectorTarget,
    path: &str,
    times: &[shrimply_project_document::project::Time],
) -> Result<usize, String> {
    with_controller(|controller| controller.copy_scalar_keyframes(target, path, times))
}

pub(super) fn copy_background_integer_keyframes(
    target: &InspectorTarget,
    path: &str,
    timeline_id: uuid::Uuid,
    times: &[shrimply_project_document::project::Time],
) -> Result<usize, String> {
    with_controller(|controller| {
        controller.copy_background_integer_keyframes(target, path, timeline_id, times)
    })
}

pub(super) fn paste_scalar_keyframes(
    target: &InspectorTarget,
    path: &str,
    time: shrimply_project_document::project::Time,
    constraint: shrimply_inspector_core::NumberConstraint,
    commit_name: &str,
) -> Result<usize, String> {
    let result = with_controller(|controller| {
        controller.paste_scalar_keyframes(
            target,
            path,
            time,
            constraint,
            shrimply_inspector_core::InspectorCommit::Immediate(commit_name),
        )
    });
    if result.is_ok() {
        EXPRESSION_DIRTY.set(true);
        GRAPH_DIRTY.set(true);
    }
    result
}

pub(super) fn paste_background_integer_keyframes(
    target: &InspectorTarget,
    path: &str,
    timeline_id: uuid::Uuid,
    time: shrimply_project_document::project::Time,
) -> Result<usize, String> {
    let result = with_controller(|controller| {
        controller.paste_background_integer_keyframes(target, path, timeline_id, time)
    });
    if result.is_ok() {
        EXPRESSION_DIRTY.set(true);
        GRAPH_DIRTY.set(true);
    }
    result
}

pub(super) fn seek_scalar_keyframe(
    target: &InspectorTarget,
    time: shrimply_project_document::project::Time,
) -> Result<(), String> {
    with_controller(|controller| controller.seek_scalar_keyframe(target, time))
}

pub(super) fn move_vector2_keyframes(
    target: &InspectorTarget,
    path: &str,
    moves: &[(
        shrimply_project_document::project::Time,
        shrimply_project_document::project::Time,
    )],
    commit_name: &str,
) -> Result<Vec<shrimply_project_document::project::Time>, String> {
    let result = with_controller(|controller| {
        controller.move_vector2_keyframes(
            target,
            path,
            moves,
            shrimply_inspector_core::InspectorCommit::Coalesced(commit_name),
        )
    });
    if result.is_ok() {
        EXPRESSION_DIRTY.set(true);
        GRAPH_DIRTY.set(true);
    }
    result
}

pub(super) fn delete_vector2_keyframe(
    target: &InspectorTarget,
    path: &str,
    time: shrimply_project_document::project::Time,
    commit_name: &str,
) -> Result<(), String> {
    let result = with_controller(|controller| {
        controller.delete_vector2_keyframe(
            target,
            path,
            time,
            shrimply_inspector_core::InspectorCommit::Immediate(commit_name),
        )
    });
    if result.is_ok() {
        EXPRESSION_DIRTY.set(true);
        GRAPH_DIRTY.set(true);
    }
    result
}

pub(super) fn add_vector2_keyframe(
    target: &InspectorTarget,
    path: &str,
    time: shrimply_project_document::project::Time,
    commit_name: &str,
) -> Result<(), String> {
    let result = with_controller(|controller| {
        controller.add_vector2_keyframe(
            target,
            path,
            time,
            shrimply_inspector_core::InspectorCommit::Immediate(commit_name),
        )
    });
    if result.is_ok() {
        EXPRESSION_DIRTY.set(true);
        GRAPH_DIRTY.set(true);
    }
    result
}

pub(super) fn copy_vector2_keyframes(
    target: &InspectorTarget,
    path: &str,
    times: &[shrimply_project_document::project::Time],
) -> Result<usize, String> {
    with_controller(|controller| controller.copy_vector2_keyframes(target, path, times))
}

pub(super) fn paste_vector2_keyframes(
    target: &InspectorTarget,
    path: &str,
    time: shrimply_project_document::project::Time,
    commit_name: &str,
) -> Result<usize, String> {
    let result = with_controller(|controller| {
        controller.paste_vector2_keyframes(
            target,
            path,
            time,
            shrimply_inspector_core::InspectorCommit::Immediate(commit_name),
        )
    });
    if result.is_ok() {
        EXPRESSION_DIRTY.set(true);
        GRAPH_DIRTY.set(true);
    }
    result
}

pub(super) fn set_vector2_interpolation(
    target: &InspectorTarget,
    path: &str,
    owner_id: uuid::Uuid,
    interpolation: usize,
    commit_name: &str,
) -> Result<(), String> {
    let result = with_controller(|controller| {
        controller.set_vector2_interpolation(
            target,
            path,
            owner_id,
            interpolation,
            shrimply_inspector_core::InspectorCommit::Immediate(commit_name),
        )
    });
    if result.is_ok() {
        EXPRESSION_DIRTY.set(true);
        GRAPH_DIRTY.set(true);
    }
    result
}

pub(super) fn move_vector3_keyframes(
    target: &InspectorTarget,
    path: &str,
    moves: &[(
        shrimply_project_document::project::Time,
        shrimply_project_document::project::Time,
    )],
    commit_name: &str,
) -> Result<Vec<shrimply_project_document::project::Time>, String> {
    let result = with_controller(|controller| {
        controller.move_vector3_keyframes(
            target,
            path,
            moves,
            shrimply_inspector_core::InspectorCommit::Coalesced(commit_name),
        )
    });
    if result.is_ok() {
        EXPRESSION_DIRTY.set(true);
        GRAPH_DIRTY.set(true);
    }
    result
}

pub(super) fn delete_vector3_keyframe(
    target: &InspectorTarget,
    path: &str,
    time: shrimply_project_document::project::Time,
    commit_name: &str,
) -> Result<(), String> {
    let result = with_controller(|controller| {
        controller.delete_vector3_keyframe(
            target,
            path,
            time,
            shrimply_inspector_core::InspectorCommit::Immediate(commit_name),
        )
    });
    if result.is_ok() {
        EXPRESSION_DIRTY.set(true);
        GRAPH_DIRTY.set(true);
    }
    result
}

pub(super) fn add_vector3_keyframe(
    target: &InspectorTarget,
    path: &str,
    time: shrimply_project_document::project::Time,
    commit_name: &str,
) -> Result<(), String> {
    let result = with_controller(|controller| {
        controller.add_vector3_keyframe(
            target,
            path,
            time,
            shrimply_inspector_core::InspectorCommit::Immediate(commit_name),
        )
    });
    if result.is_ok() {
        EXPRESSION_DIRTY.set(true);
        GRAPH_DIRTY.set(true);
    }
    result
}

pub(super) fn copy_vector3_keyframes(
    target: &InspectorTarget,
    path: &str,
    times: &[shrimply_project_document::project::Time],
) -> Result<usize, String> {
    with_controller(|controller| controller.copy_vector3_keyframes(target, path, times))
}

pub(super) fn paste_vector3_keyframes(
    target: &InspectorTarget,
    path: &str,
    time: shrimply_project_document::project::Time,
    commit_name: &str,
) -> Result<usize, String> {
    let result = with_controller(|controller| {
        controller.paste_vector3_keyframes(
            target,
            path,
            time,
            shrimply_inspector_core::InspectorCommit::Immediate(commit_name),
        )
    });
    if result.is_ok() {
        EXPRESSION_DIRTY.set(true);
        GRAPH_DIRTY.set(true);
    }
    result
}

pub(super) fn set_vector3_interpolation(
    target: &InspectorTarget,
    path: &str,
    owner_id: uuid::Uuid,
    interpolation: usize,
    commit_name: &str,
) -> Result<(), String> {
    let result = with_controller(|controller| {
        controller.set_vector3_interpolation(
            target,
            path,
            owner_id,
            interpolation,
            shrimply_inspector_core::InspectorCommit::Immediate(commit_name),
        )
    });
    if result.is_ok() {
        EXPRESSION_DIRTY.set(true);
        GRAPH_DIRTY.set(true);
    }
    result
}

pub(super) fn move_color_keyframes(
    target: &InspectorTarget,
    path: &str,
    timeline_id: uuid::Uuid,
    moves: &[(
        shrimply_project_document::project::Time,
        shrimply_project_document::project::Time,
    )],
    commit_name: &str,
) -> Result<Vec<shrimply_project_document::project::Time>, String> {
    let result = with_controller(|controller| {
        controller.move_color_keyframes(
            target,
            path,
            timeline_id,
            moves,
            shrimply_inspector_core::InspectorCommit::Coalesced(commit_name),
        )
    });
    if result.is_ok() {
        EXPRESSION_DIRTY.set(true);
        GRAPH_DIRTY.set(true);
    }
    result
}

pub(super) fn delete_color_keyframe(
    target: &InspectorTarget,
    path: &str,
    timeline_id: uuid::Uuid,
    time: shrimply_project_document::project::Time,
    commit_name: &str,
) -> Result<(), String> {
    let result = with_controller(|controller| {
        controller.delete_color_keyframe(
            target,
            path,
            timeline_id,
            time,
            shrimply_inspector_core::InspectorCommit::Immediate(commit_name),
        )
    });
    if result.is_ok() {
        EXPRESSION_DIRTY.set(true);
        GRAPH_DIRTY.set(true);
    }
    result
}

pub(super) fn add_color_keyframe(
    target: &InspectorTarget,
    path: &str,
    timeline_id: uuid::Uuid,
    time: shrimply_project_document::project::Time,
    commit_name: &str,
) -> Result<(), String> {
    let result = with_controller(|controller| {
        controller.add_color_keyframe(
            target,
            path,
            timeline_id,
            time,
            shrimply_inspector_core::InspectorCommit::Immediate(commit_name),
        )
    });
    if result.is_ok() {
        EXPRESSION_DIRTY.set(true);
        GRAPH_DIRTY.set(true);
    }
    result
}

pub(super) fn copy_color_keyframes(
    target: &InspectorTarget,
    path: &str,
    timeline_id: uuid::Uuid,
    times: &[shrimply_project_document::project::Time],
) -> Result<usize, String> {
    with_controller(|controller| controller.copy_color_keyframes(target, path, timeline_id, times))
}

pub(super) fn paste_color_keyframes(
    target: &InspectorTarget,
    path: &str,
    timeline_id: uuid::Uuid,
    time: shrimply_project_document::project::Time,
    commit_name: &str,
) -> Result<usize, String> {
    let result = with_controller(|controller| {
        controller.paste_color_keyframes(
            target,
            path,
            timeline_id,
            time,
            shrimply_inspector_core::InspectorCommit::Immediate(commit_name),
        )
    });
    if result.is_ok() {
        EXPRESSION_DIRTY.set(true);
        GRAPH_DIRTY.set(true);
    }
    result
}

pub(super) fn set_color_interpolation(
    target: &InspectorTarget,
    path: &str,
    timeline_id: uuid::Uuid,
    owner_id: uuid::Uuid,
    interpolation: usize,
    commit_name: &str,
) -> Result<(), String> {
    let result = with_controller(|controller| {
        controller.set_color_interpolation(
            target,
            path,
            timeline_id,
            owner_id,
            interpolation,
            shrimply_inspector_core::InspectorCommit::Immediate(commit_name),
        )
    });
    if result.is_ok() {
        EXPRESSION_DIRTY.set(true);
        GRAPH_DIRTY.set(true);
    }
    result
}

pub(super) fn move_bool_keyframes(
    target: &InspectorTarget,
    path: &str,
    moves: &[(
        shrimply_project_document::project::Time,
        shrimply_project_document::project::Time,
    )],
) -> Result<Vec<shrimply_project_document::project::Time>, String> {
    with_controller(|controller| controller.move_bool_keyframes(target, path, moves))
}

pub(super) fn delete_bool_keyframe(
    target: &InspectorTarget,
    path: &str,
    time: shrimply_project_document::project::Time,
) -> Result<(), String> {
    with_controller(|controller| controller.delete_bool_keyframe(target, path, time))
}

pub(super) fn add_bool_keyframe(
    target: &InspectorTarget,
    path: &str,
    time: shrimply_project_document::project::Time,
) -> Result<(), String> {
    with_controller(|controller| controller.add_bool_keyframe(target, path, time))
}

pub(super) fn copy_bool_keyframes(
    target: &InspectorTarget,
    path: &str,
    times: &[shrimply_project_document::project::Time],
) -> Result<usize, String> {
    with_controller(|controller| controller.copy_bool_keyframes(target, path, times))
}

pub(super) fn paste_bool_keyframes(
    target: &InspectorTarget,
    path: &str,
    time: shrimply_project_document::project::Time,
) -> Result<usize, String> {
    with_controller(|controller| controller.paste_bool_keyframes(target, path, time))
}

pub(super) fn move_text_keyframes(
    target: &InspectorTarget,
    path: &str,
    timeline_id: uuid::Uuid,
    moves: &[(
        shrimply_project_document::project::Time,
        shrimply_project_document::project::Time,
    )],
    commits: shrimply_inspector_core::TextKeyframeCommits,
) -> Result<Vec<shrimply_project_document::project::Time>, String> {
    let result = with_controller(|controller| {
        controller.move_text_keyframes(target, path, timeline_id, moves, commits)
    });
    if result.is_ok() {
        EXPRESSION_DIRTY.set(true);
        GRAPH_DIRTY.set(true);
    }
    result
}

pub(super) fn delete_text_keyframe(
    target: &InspectorTarget,
    path: &str,
    timeline_id: uuid::Uuid,
    time: shrimply_project_document::project::Time,
    commits: shrimply_inspector_core::TextKeyframeCommits,
) -> Result<(), String> {
    with_controller(|controller| {
        controller.delete_text_keyframe(target, path, timeline_id, time, commits)
    })
}

pub(super) fn add_text_keyframe(
    target: &InspectorTarget,
    path: &str,
    timeline_id: uuid::Uuid,
    time: shrimply_project_document::project::Time,
    commits: shrimply_inspector_core::TextKeyframeCommits,
) -> Result<(), String> {
    with_controller(|controller| {
        controller.add_text_keyframe(target, path, timeline_id, time, commits)
    })
}

pub(super) fn copy_text_keyframes(
    target: &InspectorTarget,
    path: &str,
    timeline_id: uuid::Uuid,
    times: &[shrimply_project_document::project::Time],
) -> Result<usize, String> {
    with_controller(|controller| controller.copy_text_keyframes(target, path, timeline_id, times))
}

pub(super) fn paste_text_keyframes(
    target: &InspectorTarget,
    path: &str,
    timeline_id: uuid::Uuid,
    time: shrimply_project_document::project::Time,
    commits: shrimply_inspector_core::TextKeyframeCommits,
) -> Result<usize, String> {
    with_controller(|controller| {
        controller.paste_text_keyframes(target, path, timeline_id, time, commits)
    })
}

pub(super) fn set_text_keyframe_interpolation(
    target: &InspectorTarget,
    path: &str,
    timeline_id: uuid::Uuid,
    owner_id: uuid::Uuid,
    interpolation: usize,
    commit_name: &str,
) -> Result<(), String> {
    let result = with_controller(|controller| {
        controller.set_text_keyframe_interpolation(
            target,
            path,
            timeline_id,
            owner_id,
            interpolation,
            commit_name,
        )
    });
    if result.is_ok() {
        GRAPH_DIRTY.set(true);
    }
    result
}

pub(super) fn text_keyframe_text_interpolation(
    target: &InspectorTarget,
    path: &str,
    timeline_id: uuid::Uuid,
    owner_id: uuid::Uuid,
) -> Result<usize, String> {
    with_controller(|controller| {
        controller.text_keyframe_text_interpolation(target, path, timeline_id, owner_id)
    })
}

pub(super) fn set_text_keyframe_text_interpolation(
    target: &InspectorTarget,
    path: &str,
    timeline_id: uuid::Uuid,
    owner_id: uuid::Uuid,
    interpolation: usize,
    commit_name: &str,
) -> Result<(), String> {
    let result = with_controller(|controller| {
        controller.set_text_keyframe_text_interpolation(
            target,
            path,
            timeline_id,
            owner_id,
            interpolation,
            commit_name,
        )
    });
    if result.is_ok() {
        GRAPH_DIRTY.set(true);
    }
    result
}

pub(super) fn seek_discrete_keyframe(
    target: &InspectorTarget,
    time: shrimply_project_document::project::Time,
) -> Result<(), String> {
    with_controller(|controller| controller.seek_discrete_keyframe(target, time))
}

pub(super) fn move_step_keyframes(
    target: &InspectorTarget,
    path: &str,
    moves: &[(
        shrimply_project_document::project::Time,
        shrimply_project_document::project::Time,
    )],
    commit_name: &str,
) -> Result<Vec<shrimply_project_document::project::Time>, String> {
    let result = with_controller(|controller| {
        controller.move_step_keyframes(
            target,
            path,
            moves,
            shrimply_inspector_core::InspectorCommit::Coalesced(commit_name),
        )
    });
    if result.is_ok() {
        EXPRESSION_DIRTY.set(true);
        GRAPH_DIRTY.set(true);
    }
    result
}

pub(super) fn delete_step_keyframe(
    target: &InspectorTarget,
    path: &str,
    time: shrimply_project_document::project::Time,
    commit_name: &str,
) -> Result<(), String> {
    let result = with_controller(|controller| {
        controller.delete_step_keyframe(
            target,
            path,
            time,
            shrimply_inspector_core::InspectorCommit::Immediate(commit_name),
        )
    });
    if result.is_ok() {
        EXPRESSION_DIRTY.set(true);
        GRAPH_DIRTY.set(true);
    }
    result
}

pub(super) fn add_step_keyframe(
    target: &InspectorTarget,
    path: &str,
    time: shrimply_project_document::project::Time,
    commit_name: &str,
) -> Result<(), String> {
    let result = with_controller(|controller| {
        controller.add_step_keyframe(
            target,
            path,
            time,
            shrimply_inspector_core::InspectorCommit::Immediate(commit_name),
        )
    });
    if result.is_ok() {
        EXPRESSION_DIRTY.set(true);
        GRAPH_DIRTY.set(true);
    }
    result
}

pub(super) fn copy_step_keyframes(
    target: &InspectorTarget,
    path: &str,
    times: &[shrimply_project_document::project::Time],
) -> Result<usize, String> {
    with_controller(|controller| controller.copy_step_keyframes(target, path, times))
}

pub(super) fn paste_step_keyframes(
    target: &InspectorTarget,
    path: &str,
    time: shrimply_project_document::project::Time,
    commit_name: &str,
) -> Result<usize, String> {
    let result = with_controller(|controller| {
        controller.paste_step_keyframes(
            target,
            path,
            time,
            shrimply_inspector_core::InspectorCommit::Immediate(commit_name),
        )
    });
    if result.is_ok() {
        EXPRESSION_DIRTY.set(true);
        GRAPH_DIRTY.set(true);
    }
    result
}
