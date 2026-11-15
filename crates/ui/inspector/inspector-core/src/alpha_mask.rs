use shrimply_editor_state::player_state;
use shrimply_project_document::project::{
    AlphaMaskShape, ItemAddress, Time, VideoItem, VisualAlphaMask, VisualAlphaMaskTarget,
};
use shrimply_property_model::timeline_value::{TimelineBase, TimelineValue};

use crate::{
    ControlKind, InspectorControl, InspectorController, InspectorRuntime, InspectorSection,
    InspectorTarget, LayeredState, NumberConstraint, NumberSpec,
    item::{ControlPreviewFocus, PreviewFocusTarget},
};

pub const SHAPE_COMMIT: &str = "alpha-mask-shape";
pub const INVERT_COMMIT: &str = "invert-alpha-mask";
pub const VECTOR_COMMIT: &str = "visual-alpha-mask-vector";
pub const SCALAR_COMMIT: &str = "visual-alpha-mask-scalar";
pub const SHAPE_CHOICES: [(AlphaMaskShape, &str, &str); 3] = [
    (AlphaMaskShape::Rectangle, "rectangle", "Rectangle"),
    (AlphaMaskShape::Ellipse, "ellipse", "Ellipse"),
    (AlphaMaskShape::Polygon, "polygon", "Polygon"),
];

#[derive(Clone, Debug, PartialEq)]
pub struct AlphaMaskPresentation {
    pub active: bool,
    pub section: InspectorSection,
}

pub fn presentation(
    mask: Option<&VisualAlphaMask>,
    base: &str,
    owner_id: Option<uuid::Uuid>,
    preview_focus: ControlPreviewFocus,
    runtime: InspectorRuntime,
) -> AlphaMaskPresentation {
    let active = mask.is_some_and(|mask| mask.enabled);
    let mut section = InspectorSection::default();
    let Some(mask) = mask.filter(|mask| mask.enabled) else {
        return AlphaMaskPresentation { active, section };
    };

    section.add(
        InspectorControl::new(ControlKind::Selector, format!("{base}/shape"), "Shape")
            .value(
                SHAPE_CHOICES
                    .iter()
                    .find_map(|(shape, value, _)| (*shape == mask.shape).then_some(*value))
                    .expect("current alpha-mask shape must be a declared choice"),
            )
            .choices(
                SHAPE_CHOICES
                    .iter()
                    .map(|(_, value, _)| (*value).to_string())
                    .collect(),
                SHAPE_CHOICES
                    .iter()
                    .map(|(_, _, label)| (*label).to_string())
                    .collect(),
            )
            .immediate_commit(SHAPE_COMMIT),
    );
    section.add(
        InspectorControl::new(ControlKind::Boolean, format!("{base}/invert"), "Invert")
            .value(mask.invert.to_string())
            .immediate_commit(INVERT_COMMIT),
    );
    section.add(vector_control(
        format!("{base}/center"),
        "Center",
        &mask.center,
        runtime,
        ["X", "Y"],
        NumberSpec {
            drag_step: 0.01,
            digits: 2,
            unit: "x",
            ..NumberSpec::default()
        },
    ));
    section.add(vector_control(
        format!("{base}/size"),
        "Size",
        &mask.size,
        runtime,
        ["W", "H"],
        NumberSpec {
            minimum: 0.0,
            drag_step: 0.01,
            digits: 2,
            unit: "x",
            ..NumberSpec::default()
        },
    ));
    section.add(scalar_control(
        format!("{base}/rotation_degrees"),
        "Rotation",
        &mask.rotation_degrees,
        runtime,
        NumberSpec {
            drag_step: 1.0,
            digits: 1,
            unit: "°",
            ..NumberSpec::default()
        },
        1.0,
        true,
    ));
    if mask.shape == AlphaMaskShape::Rectangle {
        section.add(scalar_control(
            format!("{base}/rounding"),
            "Roundness",
            &mask.rounding,
            runtime,
            percent_spec(),
            100.0,
            false,
        ));
    }
    section.add(scalar_control(
        format!("{base}/feather"),
        "Feather",
        &mask.feather,
        runtime,
        percent_spec(),
        100.0,
        false,
    ));
    if let Some(owner_id) = owner_id {
        section.set_target(owner_id);
    }
    section.set_preview_focus(preview_focus);
    AlphaMaskPresentation { active, section }
}

pub fn preview_focus(
    item_id: uuid::Uuid,
    target: VisualAlphaMaskTarget,
    mask: bool,
) -> ControlPreviewFocus {
    let (card_key, owner_id, facet) = match (target, mask) {
        (VisualAlphaMaskTarget::Compositing, true) => (
            "compositing".to_string(),
            item_id,
            shrimply_project_document::project::COMPOSITING_ALPHA_MASK_PREVIEW_FACET,
        ),
        (VisualAlphaMaskTarget::Compositing, false) => (
            "compositing".to_string(),
            item_id,
            shrimply_project_document::project::ITEM_PREVIEW_FACET,
        ),
        (VisualAlphaMaskTarget::Modifier(id), true) => (
            format!("modifier:{id}"),
            id,
            shrimply_project_document::project::MODIFIER_ALPHA_MASK_PREVIEW_FACET,
        ),
        (VisualAlphaMaskTarget::Modifier(id), false) => (
            format!("modifier:{id}"),
            id,
            shrimply_visual_modifiers::MODIFIER_PREVIEW_FACET,
        ),
    };
    ControlPreviewFocus::new(
        card_key,
        PreviewFocusTarget::target(shrimply_preview_provider_skia::PreviewTarget::new(
            owner_id, facet,
        )),
    )
}

impl InspectorController {
    pub fn alpha_mask_presentation(
        &self,
        target: &InspectorTarget,
        mask_target: VisualAlphaMaskTarget,
    ) -> Result<AlphaMaskPresentation, String> {
        let project = self.project.borrow();
        let address = video_address(target)?;
        let item = project
            .video_item(address)
            .ok_or_else(|| "alpha-mask item is no longer available".to_string())?;
        let (base, owner_id) = target_path(item, mask_target)?;
        Ok(presentation(
            item.alpha_mask(mask_target),
            &base,
            owner_id,
            preview_focus(item.id, mask_target, true),
            crate::model::target_runtime(&project, &self.player_state, target),
        ))
    }

    pub fn set_alpha_mask_enabled(
        &self,
        target: &InspectorTarget,
        mask_target: VisualAlphaMaskTarget,
        enabled: bool,
    ) -> Result<(), String> {
        let mut project = self.project.borrow_mut();
        let item = project
            .video_item_mut(video_address(target)?)
            .ok_or_else(|| "alpha-mask item is no longer available".to_string())?;
        target_path(item, mask_target)?;
        if enabled {
            if let Some(mask) = item.alpha_mask_mut(mask_target) {
                if mask.enabled {
                    return Ok(());
                }
                mask.enabled = true;
            } else if !item.set_alpha_mask(mask_target, Some(VisualAlphaMask::default())) {
                return Err("alpha-mask target is no longer available".to_string());
            }
        } else if item.alpha_mask(mask_target).is_none() {
            return Ok(());
        } else if !item.set_alpha_mask(mask_target, None) {
            return Err("alpha-mask target is no longer available".to_string());
        }
        shrimply_project_document::project::commit_edit(
            &project,
            if enabled {
                "add-alpha-mask"
            } else {
                "remove-alpha-mask"
            },
        );
        drop(project);
        player_state::refresh_project(
            &self.player_state,
            crate::refresh::target_change(target, None, true),
        );
        Ok(())
    }

    pub fn set_alpha_mask_shape(
        &self,
        target: &InspectorTarget,
        mask_target: VisualAlphaMaskTarget,
        shape: AlphaMaskShape,
    ) -> Result<(), String> {
        self.edit_alpha_mask(target, mask_target, SHAPE_COMMIT, |mask| {
            if mask.shape == shape {
                return false;
            }
            mask.shape = shape;
            true
        })
    }

    pub fn set_alpha_mask_inverted(
        &self,
        target: &InspectorTarget,
        mask_target: VisualAlphaMaskTarget,
        invert: bool,
    ) -> Result<(), String> {
        self.edit_alpha_mask(target, mask_target, INVERT_COMMIT, |mask| {
            if mask.invert == invert {
                return false;
            }
            mask.invert = invert;
            true
        })
    }

    fn edit_alpha_mask(
        &self,
        target: &InspectorTarget,
        mask_target: VisualAlphaMaskTarget,
        commit_name: &'static str,
        edit: impl FnOnce(&mut VisualAlphaMask) -> bool,
    ) -> Result<(), String> {
        let mut project = self.project.borrow_mut();
        let item = project
            .video_item_mut(video_address(target)?)
            .ok_or_else(|| "alpha-mask item is no longer available".to_string())?;
        target_path(item, mask_target)?;
        let mask = item
            .alpha_mask_mut(mask_target)
            .filter(|mask| mask.enabled)
            .ok_or_else(|| "alpha mask is no longer enabled".to_string())?;
        if !edit(mask) {
            return Ok(());
        }
        shrimply_project_document::project::commit_edit(&project, commit_name);
        drop(project);
        player_state::refresh_project(
            &self.player_state,
            crate::refresh::target_change(target, None, true),
        );
        Ok(())
    }
}

fn target_path(
    item: &VideoItem,
    target: VisualAlphaMaskTarget,
) -> Result<(String, Option<uuid::Uuid>), String> {
    match target {
        VisualAlphaMaskTarget::Compositing => Ok(("/compositing/alpha_mask".to_string(), None)),
        VisualAlphaMaskTarget::Modifier(id) => item
            .modifiers
            .iter()
            .position(|modifier| modifier.id == id)
            .map(|index| (format!("/modifiers/{index}/alpha_mask"), Some(id)))
            .ok_or_else(|| "alpha-mask modifier is no longer available".to_string()),
    }
}

fn video_address(target: &InspectorTarget) -> Result<&ItemAddress, String> {
    let InspectorTarget::Item(address @ ItemAddress::Video { .. }) = target else {
        return Err("alpha-mask target is not a video item".to_string());
    };
    Ok(address)
}

fn vector_control(
    path: String,
    label: &'static str,
    timeline: &TimelineValue<glam::Vec2>,
    runtime: InspectorRuntime,
    prefixes: [&'static str; 2],
    number: NumberSpec,
) -> InspectorControl {
    let value = timeline.value_at(runtime.local_time.unwrap_or(Time::ZERO));
    InspectorControl::new(ControlKind::LayeredVector2, path.clone(), label)
        .components(vec![value.x.to_string(), value.y.to_string()])
        .number(number)
        .width_characters(7)
        .prefixes(prefixes)
        .layered(path, LayeredState::from(timeline))
        .timeline(
            timeline.id,
            crate::transform::vector_speed_graph(timeline, runtime),
        )
        .live_commit(VECTOR_COMMIT)
        .timeline_commits(VECTOR_COMMIT, VECTOR_COMMIT)
}

fn scalar_control(
    path: String,
    label: &'static str,
    timeline: &TimelineValue<f32>,
    runtime: InspectorRuntime,
    number: NumberSpec,
    display_multiplier: f64,
    rotating: bool,
) -> InspectorControl {
    let value = f64::from(timeline.value_at(runtime.local_time.unwrap_or(Time::ZERO)));
    let graph = matches!(timeline.base, TimelineBase::Keyframes(_)).then(|| {
        let mut graph = crate::transform::scalar_graph(timeline, value as f32, runtime)
            .expect("keyframed alpha-mask scalar must have a graph");
        graph
            .points
            .iter_mut()
            .for_each(|point| point.value *= display_multiplier);
        graph.segments.iter_mut().for_each(|segment| {
            segment.start_value *= display_multiplier;
            segment.end_value *= display_multiplier;
        });
        graph
    });
    let defaults = NumberSpec::default();
    let constraint = NumberConstraint {
        minimum: (number.minimum != defaults.minimum)
            .then_some(number.minimum / display_multiplier),
        maximum: (number.maximum != defaults.maximum)
            .then_some(number.maximum / display_multiplier),
        ..NumberConstraint::default()
    };
    let control = InspectorControl::new(ControlKind::LayeredNumber, path.clone(), label)
        .value((value * display_multiplier).to_string())
        .number(number)
        .number_constraint(constraint)
        .store_multiplier(display_multiplier.recip())
        .width_characters(8)
        .layered(path, LayeredState::from(timeline))
        .timeline(timeline.id, graph)
        .live_commit(SCALAR_COMMIT)
        .timeline_commits(SCALAR_COMMIT, SCALAR_COMMIT);
    if rotating {
        control.rotating_icon("rotation.svg", 0.0)
    } else {
        control
    }
}

fn percent_spec() -> NumberSpec {
    NumberSpec {
        minimum: 0.0,
        maximum: 100.0,
        drag_step: 1.0,
        digits: 1,
        unit: "%",
    }
}
