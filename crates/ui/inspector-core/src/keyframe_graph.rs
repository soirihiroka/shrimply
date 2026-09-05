use shrimply_project::project::{ItemAddress, Time};

pub use shrimply_keyframe_graph_core::*;

use crate::section::{ControlKind, InspectorControl, InspectorSection, NumberMapping};
use crate::{InspectorController, InspectorTarget};

pub fn vector_control_graph(
    value: &serde_json::Value,
    runtime: crate::InspectorRuntime,
    control: &InspectorControl,
) -> Option<Result<Option<ScalarGraph>, String>> {
    let path = control
        .timeline_path
        .as_deref()
        .unwrap_or(control.path.as_str());
    let timeline_id = match control.timeline_id {
        Some(timeline_id) => timeline_id,
        None => return Some(Err("vector control has no timeline ID".to_string())),
    };
    match control.kind {
        ControlKind::LayeredVector2 => Some(vector_graph::<glam::Vec2>(
            value,
            runtime,
            path,
            timeline_id,
        )),
        ControlKind::LayeredVector3 => Some(vector_graph::<glam::Vec3>(
            value,
            runtime,
            path,
            timeline_id,
        )),
        _ => None,
    }
}

fn vector_graph<T: shrimply_core::timeline_value::TimelineVector>(
    value: &serde_json::Value,
    runtime: crate::InspectorRuntime,
    path: &str,
    timeline_id: uuid::Uuid,
) -> Result<Option<ScalarGraph>, String>
where
    shrimply_core::timeline_value::TimelineValue<T>: serde::de::DeserializeOwned,
{
    let timeline: shrimply_core::timeline_value::TimelineValue<T> = serde_json::from_value(
        value
            .pointer(path)
            .cloned()
            .ok_or_else(|| format!("vector value is no longer available: {path}"))?,
    )
    .map_err(|error| format!("invalid vector value: {error}"))?;
    if timeline.id != timeline_id {
        return Err(format!("vector value is no longer available: {path}"));
    }
    Ok(crate::timeline_value::vector::scalar_speed_graph(
        &timeline, runtime,
    ))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InspectorGraphKind {
    Raw,
    Step,
    Speed,
}

impl InspectorGraphKind {
    pub fn for_control(kind: ControlKind) -> Option<Self> {
        match kind {
            ControlKind::LayeredNumber => Some(Self::Raw),
            ControlKind::LayeredBoolean | ControlKind::LayeredSelector => Some(Self::Step),
            ControlKind::LayeredColor
            | ControlKind::LayeredDrawing
            | ControlKind::LayeredText
            | ControlKind::LayeredVector2
            | ControlKind::LayeredVector3 => Some(Self::Speed),
            _ => None,
        }
    }

    pub fn uses_discrete_seek(kind: ControlKind) -> bool {
        matches!(
            kind,
            ControlKind::LayeredBoolean
                | ControlKind::LayeredDrawing
                | ControlKind::LayeredSelector
                | ControlKind::LayeredText
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GraphPoint {
    pub time: Time,
    pub value: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GraphSegment {
    pub owner_id: uuid::Uuid,
    pub start: Time,
    pub end: Time,
    pub start_value: f64,
    pub end_value: f64,
    pub interpolation: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ScalarGraph {
    pub points: Vec<GraphPoint>,
    pub segments: Vec<GraphSegment>,
    pub range: (Time, Time),
    pub frame_step: Time,
    pub playhead: Time,
}

impl ScalarGraph {
    pub fn map_control_values(&mut self, control: &InspectorControl) {
        if control.kind != ControlKind::LayeredNumber
            || (control.store_multiplier == 1.0 && control.number_mapping == NumberMapping::Linear)
        {
            return;
        }
        self.points
            .iter_mut()
            .for_each(|point| point.value = control.display_number(point.value));
        self.segments.iter_mut().for_each(|segment| {
            segment.start_value = control.display_number(segment.start_value);
            segment.end_value = control.display_number(segment.end_value);
        });
    }
}

impl InspectorController {
    pub fn control_graph_source(
        &self,
        target: &InspectorTarget,
    ) -> Result<(serde_json::Value, crate::InspectorRuntime), String> {
        let project = self.project.borrow();
        let value = crate::model::target_value(&project, target)
            .ok_or_else(|| "inspector target is no longer available".to_string())?
            .1;
        let runtime = crate::model::target_runtime(&project, &self.player_state, target);
        Ok((value, runtime))
    }

    pub fn control_graph(
        &self,
        target: &InspectorTarget,
        control: &InspectorControl,
    ) -> Result<Option<ScalarGraph>, String> {
        if control.kind == ControlKind::LayeredDrawing {
            if control.path != crate::paint::DRAWING_PATH
                || control.timeline_path.as_deref() != Some(crate::paint::DRAWING_PATH)
            {
                return Err("paint drawing control has invalid graph metadata".to_string());
            }
            return self.paint_drawing_graph(
                target,
                control
                    .timeline_id
                    .ok_or_else(|| "paint drawing control has no timeline ID".to_string())?,
            );
        }
        if matches!(
            control.kind,
            ControlKind::LayeredVector2 | ControlKind::LayeredVector3
        ) {
            let (value, runtime) = self.control_graph_source(target)?;
            return vector_control_graph(&value, runtime, control)
                .expect("layered vector control must have a vector graph result");
        }
        if let Some(graph) = crate::generated::control_graph(self, target, control) {
            return graph;
        }
        self.visual_modifier_control_graph(target, control)
    }

    pub fn visual_modifier_control_graph(
        &self,
        target: &InspectorTarget,
        control: &InspectorControl,
    ) -> Result<Option<ScalarGraph>, String> {
        if !control.path.starts_with("/modifiers/") {
            return Err("control is not a visual modifier".to_string());
        }
        let timeline_id = || {
            control
                .timeline_id
                .ok_or_else(|| "visual modifier control has no timeline ID".to_string())
        };
        let modifier_id = || {
            control
                .target_id
                .ok_or_else(|| "visual modifier control has no modifier target".to_string())
        };
        let path = control.path.as_str();
        let mut graph = match control.kind {
            ControlKind::LayeredNumber => {
                self.visual_modifier_number_graph(target, path, timeline_id()?)?
            }
            ControlKind::LayeredVector2 => {
                self.visual_modifier_vector2_graph(target, path, timeline_id()?)?
            }
            ControlKind::LayeredVector3 => {
                self.visual_modifier_vector3_graph(target, path, timeline_id()?)?
            }
            ControlKind::LayeredColor => {
                self.visual_modifier_color_graph(target, path, timeline_id()?)?
            }
            ControlKind::LayeredText => {
                self.visual_modifier_text_graph(target, path, timeline_id()?)?
            }
            ControlKind::LayeredSelector if path.ends_with("/effect/effect/config/operation") => {
                self.erode_dilate_operation_graph(target, path, timeline_id()?)?
            }
            ControlKind::LayeredSelector if path.ends_with("/effect/effect/sample_method") => {
                self.rasterize_sample_method_graph(target, path, timeline_id()?)?
            }
            ControlKind::LayeredSelector
                if path.ends_with("/effect/effect/config/row_offset_axis") =>
            {
                self.repeat_offset_axis_graph(target, path, modifier_id()?, timeline_id()?)?
            }
            ControlKind::LayeredSelector if path.ends_with("/effect/effect/config/method") => {
                self.sampling_method_graph(target, path, modifier_id()?, timeline_id()?)?
            }
            ControlKind::LayeredSelector
                if path.ends_with("/effect/effect/config/address_mode") =>
            {
                self.texture_bounds_address_mode_graph(
                    target,
                    path,
                    modifier_id()?,
                    timeline_id()?,
                )?
            }
            ControlKind::LayeredSelector if path.ends_with("/effect/effect/config/pattern") => {
                self.dithering_pattern_graph(target, path, timeline_id()?)?
            }
            ControlKind::LayeredSelector if path.ends_with("/effect/effect/config/color_mode") => {
                self.dithering_color_mode_graph(target, path, timeline_id()?)?
            }
            ControlKind::LayeredSelector if path.ends_with("/effect/effect/config/version") => {
                self.kuwahara_version_graph(target, path, timeline_id()?)?
            }
            ControlKind::LayeredSelector if path.ends_with("/effect/effect/config/mode") => self
                .mask_mode_graph(target, path, modifier_id()?, timeline_id()?)
                .or_else(|_| self.halftone_mode_graph(target, path, timeline_id()?))?,
            _ => return Err(format!("visual modifier control has no graph: {path}")),
        };
        if let Some(graph) = &mut graph {
            graph.map_control_values(control);
        }
        Ok(graph)
    }
}

pub fn is_transform_path(path: &str) -> bool {
    path.starts_with("/transform/") || path.contains("/effect/effect/config/transform/")
}

pub fn has_transform_controls(section: &InspectorSection) -> bool {
    section.controls.iter().any(|control| {
        matches!(
            control.kind,
            ControlKind::LayeredNumber | ControlKind::LayeredVector2
        ) && is_transform_path(&control.path)
    })
}

pub fn update_transform_graphs(
    section: &mut InspectorSection,
    live: &crate::transform::TransformLivePresentation,
) {
    for control in &mut section.controls {
        if matches!(
            control.kind,
            ControlKind::LayeredNumber | ControlKind::LayeredVector2
        ) && let Some(graph) = live.graph(&control.path)
        {
            control.scalar_graph = Some(graph.clone());
        }
    }
}

pub fn view_state_scope(item: Option<&ItemAddress>, scope: &str) -> String {
    let selected = item.map_or_else(
        || "none".to_string(),
        |item| format!("{:?}:{}:{}", item.kind(), item.track_id(), item.item_id()),
    );
    format!("{selected}:{scope}")
}
