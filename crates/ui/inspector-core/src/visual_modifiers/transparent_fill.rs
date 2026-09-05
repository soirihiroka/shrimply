use shrimply_core::timeline_value::TimelineValue;
use shrimply_video_cuda::transparent_fill_analysis::{self, Status};
use shrimply_video_modifiers::{
    ModifierEffect, RasterModifierEffect,
    transparent_fill::{MAXIMUM_GAP, TransparentFillModifier},
};

use crate::{
    AnalysisControlPresentation, AnalysisTooltip, ControlKind, ControlRowRole, InspectorControl,
    InspectorControlAction, InspectorController, InspectorRuntime, InspectorSection,
    InspectorTarget, NumberSpec,
    model::{AnalysisTransitionKey, CachedTransparentFillStatus},
};

pub const EDIT_COMMIT: &str = "edit-transparent-fill";
pub const ANALYZE_TOOLTIP: &str = "Precompute exact one-bit transparency masks for every frame";

pub(super) fn presentation(
    value: &TransparentFillModifier,
    index: usize,
    modifier_id: uuid::Uuid,
    status: Status,
    runtime: InspectorRuntime,
) -> InspectorSection {
    let base = format!("/modifiers/{index}/effect/effect/config");
    let mut section = InspectorSection::default();
    section.add(super::modifier_scalar_control(
        format!("{base}/tolerance"),
        "Tolerance",
        &value.tolerance,
        runtime,
        NumberSpec {
            minimum: 0.0,
            maximum: 1.0,
            drag_step: 0.01,
            digits: 2,
            unit: "",
        },
        false,
    ));
    section.add(
        InspectorControl::new(
            ControlKind::Number,
            format!("{base}/maximum_gap"),
            "Maximum gap",
        )
        .value(value.maximum_gap.to_string())
        .number(NumberSpec {
            minimum: 0.0,
            maximum: f64::from(MAXIMUM_GAP),
            drag_step: 1.0,
            digits: 0,
            unit: "",
        })
        .integer()
        .tooltip("0 disables gap closing; positive values set the maximum gap in pixels")
        .live_commit(EDIT_COMMIT),
    );
    for (point_index, point) in value.points.iter().enumerate() {
        section.add(
            super::modifier_vector2_control(
                format!("{base}/points/{point_index}/position"),
                shrimply_i18n_core::text_args(
                    "Point %{number}",
                    &[("number", (point_index + 1).to_string())],
                ),
                &point.position,
                runtime,
                NumberSpec {
                    minimum: 0.0,
                    maximum: 1.0,
                    drag_step: 0.01,
                    digits: 2,
                    unit: "x",
                },
                false,
            )
            .row_group(point.id, ControlRowRole::Primary),
        );
        let mut remove = InspectorControl::new(
            ControlKind::Action,
            format!("{base}/points/{point_index}/remove"),
            "",
        )
        .value("Remove point")
        .tooltip("Remove point")
        .action(InspectorControlAction::RemoveTransparentFillPoint {
            modifier_id,
            point_id: point.id,
        })
        .row_group(point.id, ControlRowRole::TrailingAction);
        remove.prefix_icon = "user-trash-symbolic".to_string();
        section.add(remove);
    }
    section.add(super::modifier_analysis_control(
        format!("{base}/analyze"),
        analysis_status(status, !value.points.is_empty()),
        InspectorControlAction::ToggleTransparentFillAnalysis { modifier_id },
    ));
    section.set_target(modifier_id);
    section
}

fn analysis_status(status: Status, can_analyze: bool) -> AnalysisControlPresentation {
    match status {
        Status::Running { completed, total } => AnalysisControlPresentation {
            label: "Analyzing…".to_string(),
            progress: if total == 0 {
                -1.0
            } else {
                completed as f64 / total as f64
            },
            tooltip: AnalysisTooltip::MessageKey(ANALYZE_TOOLTIP),
            sensitive: true,
            running: true,
            cancelling: false,
            terminal: false,
            suggested: false,
        },
        Status::Complete => AnalysisControlPresentation {
            label: "Reanalyze".to_string(),
            progress: -1.0,
            tooltip: AnalysisTooltip::MessageKey(ANALYZE_TOOLTIP),
            sensitive: can_analyze,
            running: false,
            cancelling: false,
            terminal: true,
            suggested: false,
        },
        Status::Failed(error) => AnalysisControlPresentation {
            label: "Analyze".to_string(),
            progress: -1.0,
            tooltip: AnalysisTooltip::RawError(error),
            sensitive: can_analyze,
            running: false,
            cancelling: false,
            terminal: true,
            suggested: true,
        },
        Status::Cancelled => AnalysisControlPresentation {
            label: "Analyze".to_string(),
            progress: -1.0,
            tooltip: AnalysisTooltip::MessageKey(ANALYZE_TOOLTIP),
            sensitive: can_analyze,
            running: false,
            cancelling: false,
            terminal: true,
            suggested: true,
        },
        Status::Missing => AnalysisControlPresentation {
            label: "Analyze".to_string(),
            progress: -1.0,
            tooltip: AnalysisTooltip::MessageKey(ANALYZE_TOOLTIP),
            sensitive: can_analyze,
            running: false,
            cancelling: false,
            terminal: false,
            suggested: true,
        },
    }
}

pub(super) fn number<'a>(
    value: &'a TransparentFillModifier,
    field: &str,
    timeline_id: uuid::Uuid,
) -> Option<&'a TimelineValue<f32>> {
    (field == "effect/effect/config/tolerance" && value.tolerance.id == timeline_id)
        .then_some(&value.tolerance)
}

pub(super) fn vector2<'a>(
    value: &'a TransparentFillModifier,
    field: &str,
    timeline_id: uuid::Uuid,
) -> Option<&'a TimelineValue<glam::Vec2>> {
    let point_index = field
        .strip_prefix("effect/effect/config/points/")?
        .strip_suffix("/position")?
        .parse::<usize>()
        .ok()?;
    value
        .points
        .get(point_index)
        .map(|point| &point.position)
        .filter(|position| position.id == timeline_id)
}

impl InspectorController {
    pub fn retain_analysis_transitions(&self) {
        let project = self.project.borrow();
        self.analysis_transitions
            .borrow_mut()
            .retain(|key, generation| {
                transparent_fill_modifier_at(&project, &key.item, key.modifier_id)
                    .is_some_and(|fill| fill.analysis_generation == *generation)
            });
        self.transparent_fill_statuses
            .borrow_mut()
            .retain(|key, _| {
                transparent_fill_modifier_at(&project, &key.item, key.modifier_id).is_some()
            });
    }

    pub fn observe_analysis_transition(
        &self,
        target: &InspectorTarget,
        action: InspectorControlAction,
        presentation: &AnalysisControlPresentation,
    ) -> bool {
        let InspectorControlAction::ToggleTransparentFillAnalysis { modifier_id } = action else {
            return false;
        };
        let Ok(item) = super::video_address(target).cloned() else {
            return false;
        };
        let generation = {
            let project = self.project.borrow();
            let Some(fill) = transparent_fill_modifier_at(&project, &item, modifier_id) else {
                self.analysis_transitions
                    .borrow_mut()
                    .remove(&AnalysisTransitionKey { item, modifier_id });
                return false;
            };
            fill.analysis_generation
        };
        let key = AnalysisTransitionKey { item, modifier_id };
        let mut transitions = self.analysis_transitions.borrow_mut();
        if presentation.active() {
            transitions.insert(key, generation);
            return false;
        }
        let finished = presentation.terminal && transitions.get(&key) == Some(&generation);
        transitions.remove(&key);
        finished
    }

    pub fn transparent_fill_presentation(
        &self,
        target: &InspectorTarget,
        modifier_id: uuid::Uuid,
    ) -> Result<InspectorSection, String> {
        let status = transparent_fill_analysis::status_prepared(
            &self.transparent_fill_prepared_status(target, modifier_id)?,
        );
        let project = self.project.borrow();
        let address = super::video_address(target)?;
        let runtime = crate::model::target_runtime(&project, &self.player_state, target);
        let item = project
            .video_item(address)
            .ok_or_else(|| "Transparent Fill item is no longer available".to_string())?;
        let (index, modifier) = item
            .modifiers
            .iter()
            .enumerate()
            .find(|(_, modifier)| modifier.id == modifier_id)
            .ok_or_else(|| "Transparent Fill modifier is no longer available".to_string())?;
        let ModifierEffect::Raster(effect) = &modifier.effect else {
            return Err("Transparent Fill modifier is no longer available".to_string());
        };
        let RasterModifierEffect::TransparentFill(value) = &**effect else {
            return Err("Transparent Fill modifier is no longer available".to_string());
        };
        Ok(presentation(value, index, modifier_id, status, runtime))
    }

    pub fn transparent_fill_analysis_control(
        &self,
        target: &InspectorTarget,
        modifier_id: uuid::Uuid,
    ) -> Result<AnalysisControlPresentation, String> {
        let status = transparent_fill_analysis::status_prepared(
            &self.transparent_fill_prepared_status(target, modifier_id)?,
        );
        let project = self.project.borrow();
        let fill = transparent_fill_modifier(&project, target, modifier_id)?;
        Ok(analysis_status(status, !fill.points.is_empty()))
    }

    pub fn set_transparent_fill_maximum_gap(
        &self,
        target: &InspectorTarget,
        modifier_id: uuid::Uuid,
        maximum_gap: u32,
    ) -> Result<(), String> {
        if maximum_gap > MAXIMUM_GAP {
            return Err(format!("maximum gap must not exceed {MAXIMUM_GAP}"));
        }
        let mut project = self.project.borrow_mut();
        let fill = transparent_fill_modifier_mut(&mut project, target, modifier_id)?;
        if fill.maximum_gap == maximum_gap {
            return Ok(());
        }
        fill.maximum_gap = maximum_gap;
        shrimply_project::project::commit_edit(&project, EDIT_COMMIT);
        drop(project);
        super::refresh(&self.player_state);
        Ok(())
    }

    pub fn remove_transparent_fill_point(
        &self,
        target: &InspectorTarget,
        modifier_id: uuid::Uuid,
        point_id: uuid::Uuid,
    ) -> Result<(), String> {
        let mut project = self.project.borrow_mut();
        let fill = transparent_fill_modifier_mut(&mut project, target, modifier_id)?;
        let index = fill
            .points
            .iter()
            .position(|point| point.id == point_id)
            .ok_or_else(|| "Transparent Fill point is no longer available".to_string())?;
        fill.points.remove(index);
        shrimply_project::project::commit_edit(&project, EDIT_COMMIT);
        drop(project);
        super::refresh(&self.player_state);
        Ok(())
    }

    pub fn toggle_transparent_fill_analysis(
        &self,
        target: &InspectorTarget,
        modifier_id: uuid::Uuid,
    ) -> Result<(), String> {
        let address = super::video_address(target)?.clone();
        let active_run = {
            let prepared = self.transparent_fill_prepared_status(target, modifier_id)?;
            transparent_fill_analysis::active_run_prepared(&prepared)
        };
        if let Some(run_id) = active_run
            && transparent_fill_analysis::cancel(run_id)
        {
            return Ok(());
        }
        let mut project = self.project.borrow_mut();
        let fill = transparent_fill_modifier_mut(&mut project, target, modifier_id)?;
        if fill.points.is_empty() {
            return Err("Transparent Fill analysis requires at least one point".to_string());
        }
        fill.analysis_generation = fill.analysis_generation.wrapping_add(1).max(1);
        let generation = fill.analysis_generation;
        shrimply_project::project::commit_edit(&project, EDIT_COMMIT);
        let snapshot = project.clone();
        drop(project);
        let result = transparent_fill_analysis::analyze(snapshot, &address, modifier_id).map(drop);
        let key = AnalysisTransitionKey {
            item: address,
            modifier_id,
        };
        if result.is_ok() {
            self.analysis_transitions
                .borrow_mut()
                .insert(key, generation);
        } else {
            self.analysis_transitions.borrow_mut().remove(&key);
        }
        super::refresh(&self.player_state);
        result
    }

    fn transparent_fill_prepared_status(
        &self,
        target: &InspectorTarget,
        modifier_id: uuid::Uuid,
    ) -> Result<transparent_fill_analysis::PreparedStatus, String> {
        let item = super::video_address(target)?.clone();
        let key = AnalysisTransitionKey {
            item: item.clone(),
            modifier_id,
        };
        let revision = shrimply_state::player_state::snapshot(&self.player_state).revision;
        if let Some(cached) = self
            .transparent_fill_statuses
            .borrow()
            .get(&key)
            .filter(|cached| cached.revision == revision)
        {
            return Ok(cached.prepared.clone());
        }
        let prepared =
            transparent_fill_analysis::prepare_status(&self.project.borrow(), &item, modifier_id)?;
        self.transparent_fill_statuses.borrow_mut().insert(
            key,
            CachedTransparentFillStatus {
                revision,
                prepared: prepared.clone(),
            },
        );
        Ok(prepared)
    }
}

fn transparent_fill_modifier_at<'a>(
    project: &'a shrimply_project::project::Project,
    address: &shrimply_project::project::ItemAddress,
    modifier_id: uuid::Uuid,
) -> Option<&'a TransparentFillModifier> {
    project
        .video_item(address)?
        .modifiers
        .iter()
        .find(|modifier| modifier.id == modifier_id)
        .and_then(|modifier| match &modifier.effect {
            ModifierEffect::Raster(effect) => match &**effect {
                RasterModifierEffect::TransparentFill(fill) => Some(fill),
                _ => None,
            },
            _ => None,
        })
}

fn transparent_fill_modifier<'a>(
    project: &'a shrimply_project::project::Project,
    target: &InspectorTarget,
    modifier_id: uuid::Uuid,
) -> Result<&'a TransparentFillModifier, String> {
    transparent_fill_modifier_at(project, super::video_address(target)?, modifier_id)
        .ok_or_else(|| "Transparent Fill modifier is no longer available".to_string())
}

fn transparent_fill_modifier_mut<'a>(
    project: &'a mut shrimply_project::project::Project,
    target: &InspectorTarget,
    modifier_id: uuid::Uuid,
) -> Result<&'a mut TransparentFillModifier, String> {
    project
        .video_item_mut(super::video_address(target)?)
        .and_then(|item| {
            item.modifiers
                .iter_mut()
                .find(|modifier| modifier.id == modifier_id)
        })
        .and_then(|modifier| match &mut modifier.effect {
            ModifierEffect::Raster(effect) => match &mut **effect {
                RasterModifierEffect::TransparentFill(fill) => Some(fill),
                _ => None,
            },
            _ => None,
        })
        .ok_or_else(|| "Transparent Fill modifier is no longer available".to_string())
}
