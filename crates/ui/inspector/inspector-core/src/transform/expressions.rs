use shrimply_editor_state::player_state::ProjectChange;
use shrimply_project_document::project::{ItemAddress, Transform};
use shrimply_property_model::timeline_value::{TimelineExpressionValue, TimelineValue};

use super::{ScalarField, TransformField, Vec2Field};
use crate::{
    InspectorCommit, InspectorController, InspectorExpressionOutput, InspectorTarget,
    model::EditKind,
};

pub const TOGGLE_COMMIT: &str = "video-transform-expression";
pub const SOURCE_COMMIT: &str = "transform-expression";

pub fn source(transform: &Transform, field: TransformField) -> Option<&str> {
    match field {
        TransformField::Vec2(field) => field.timeline(transform).expression.as_ref(),
        TransformField::Scalar(field) => field.timeline(transform).expression.as_ref(),
    }
    .map(|expression| expression.source.as_str())
}

pub fn enabled(transform: &Transform, field: TransformField) -> bool {
    match field {
        TransformField::Vec2(field) => field.timeline(transform).expression.as_ref(),
        TransformField::Scalar(field) => field.timeline(transform).expression.as_ref(),
    }
    .is_some_and(|expression| expression.enabled)
}

pub fn format_vec2(field: Vec2Field, value: glam::Vec2) -> String {
    let number = field.number();
    let digits =
        usize::try_from(number.digits).expect("transform vector digits must be nonnegative");
    let x = format_number(f64::from(value.x), digits);
    let y = format_number(f64::from(value.y), digits);
    let unit = number.unit;
    format!("X {x}{unit}  Y {y}{unit}")
}

pub fn format_scalar(field: ScalarField, value: f32) -> String {
    let number = field.number();
    format!(
        "{}{}",
        format_number(
            f64::from(value),
            usize::try_from(number.digits).expect("transform scalar digits must be nonnegative"),
        ),
        number.unit,
    )
}

fn format_number(value: f64, digits: usize) -> String {
    let value = if value.abs() < 10.0_f64.powi(-(digits as i32)) / 2.0 {
        0.0
    } else {
        value
    };
    format!("{value:.digits$}")
}

impl InspectorController {
    pub fn set_transform_expression_enabled(
        &self,
        target: &InspectorTarget,
        field: TransformField,
        timeline_id: uuid::Uuid,
        enabled: bool,
        commit: InspectorCommit<'_>,
    ) -> Result<(), String> {
        self.ensure_timeline(target, field.path(), timeline_id)?;
        match field {
            TransformField::Vec2(field) => {
                let (mut timeline, _) = self.vector2_timeline(target, field.path())?;
                if !crate::timeline_value::vector::vec2::set_expression_enabled(
                    &mut timeline,
                    enabled,
                ) {
                    return Ok(());
                }
                self.replace_transform_expression(
                    target,
                    EditKind::Structural,
                    field.path(),
                    serde_json::to_value(timeline).expect("transform vector must serialize"),
                    commit,
                )
            }
            TransformField::Scalar(field) => {
                let mut timeline = self.scalar_timeline(target, field.path())?;
                if !crate::keyframe_model::set_expression_enabled(
                    &mut timeline,
                    enabled,
                    crate::timeline_value::SCALAR_EXPRESSION_DEFAULT,
                ) {
                    return Ok(());
                }
                self.replace_transform_expression(
                    target,
                    EditKind::Structural,
                    field.path(),
                    serde_json::to_value(timeline).expect("transform scalar must serialize"),
                    commit,
                )
            }
        }
    }

    pub fn set_transform_expression_source(
        &self,
        target: &InspectorTarget,
        field: TransformField,
        timeline_id: uuid::Uuid,
        source: String,
        commit: InspectorCommit<'_>,
    ) -> Result<(), String> {
        self.ensure_timeline(target, field.path(), timeline_id)?;
        match field {
            TransformField::Vec2(field) => {
                let (mut timeline, _) = self.vector2_timeline(target, field.path())?;
                if !crate::timeline_value::vector::vec2::set_expression_source(
                    &mut timeline,
                    source,
                ) {
                    return Ok(());
                }
                self.replace_transform_expression(
                    target,
                    EditKind::Live,
                    field.path(),
                    serde_json::to_value(timeline).expect("transform vector must serialize"),
                    commit,
                )
            }
            TransformField::Scalar(field) => {
                let mut timeline = self.scalar_timeline(target, field.path())?;
                if !crate::timeline_value::scalar::set_expression_source(&mut timeline, source) {
                    return Ok(());
                }
                self.replace_transform_expression(
                    target,
                    EditKind::Live,
                    field.path(),
                    serde_json::to_value(timeline).expect("transform scalar must serialize"),
                    commit,
                )
            }
        }
    }

    pub fn transform_vec2_expression_output(
        &self,
        target: &InspectorTarget,
        field: Vec2Field,
        timeline_id: uuid::Uuid,
    ) -> Result<Option<InspectorExpressionOutput<glam::Vec2>>, String> {
        transform_expression_output(self, target, timeline_id, |transform| {
            field.timeline(transform)
        })
    }

    pub fn transform_scalar_expression_output(
        &self,
        target: &InspectorTarget,
        field: ScalarField,
        timeline_id: uuid::Uuid,
    ) -> Result<Option<InspectorExpressionOutput>, String> {
        transform_expression_output(self, target, timeline_id, |transform| {
            field.timeline(transform)
        })
    }

    fn replace_transform_expression(
        &self,
        target: &InspectorTarget,
        kind: EditKind,
        path: &str,
        timeline: serde_json::Value,
        commit: InspectorCommit<'_>,
    ) -> Result<(), String> {
        self.replace_value_with_commit(
            target,
            kind,
            path,
            timeline,
            Some(ProjectChange {
                video: true,
                live_preview: true,
                ..ProjectChange::default()
            }),
            commit,
        )
    }
}

fn transform_expression_output<T: TimelineExpressionValue>(
    inspector: &InspectorController,
    target: &InspectorTarget,
    timeline_id: uuid::Uuid,
    timeline: impl for<'a> Fn(&'a Transform) -> &'a TimelineValue<T>,
) -> Result<Option<InspectorExpressionOutput<T>>, String> {
    if &inspector.target() != target {
        return Err("inspector target changed".to_string());
    }
    let InspectorTarget::Item(address @ ItemAddress::Video { .. }) = target else {
        return Err("transform expression target is not a video item".to_string());
    };
    let player = crate::model::player_snapshot(&inspector.player_state);
    let project = inspector.project.borrow();
    let item = project
        .video_item(address)
        .ok_or_else(|| "transform expression item is no longer available".to_string())?;
    let timeline = timeline(&item.transform);
    if timeline.id != timeline_id {
        return Err("transform expression timeline is no longer available".to_string());
    }
    if !active(timeline) {
        return Ok(None);
    }
    let audio =
        inspector
            .audio_sampler
            .borrow_mut()
            .sample(&project, player.position, player.revision);
    let outcome = crate::timeline_value::evaluate_visual_expression(
        &project,
        address,
        player.position,
        &audio,
        &mut inspector.expression_cache.borrow_mut(),
        timeline,
    )
    .ok_or_else(|| "transform expression time is no longer available".to_string())?;
    Ok(Some(InspectorExpressionOutput {
        value: outcome.value,
        error: outcome.error,
    }))
}

fn active<T: shrimply_property_model::timeline_value::TimelineValueType>(
    timeline: &TimelineValue<T>,
) -> bool {
    timeline
        .expression
        .as_ref()
        .is_some_and(|expression| expression.enabled && !expression.source.trim().is_empty())
}
