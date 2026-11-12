//! Sanim reflection is ordinary typed inspector controls, backed by background observations.
use crate::{
    ControlKind, InspectorControl, InspectorController, InspectorSection, InspectorTarget,
    NumberSpec, VideoCard,
};
use shrimply_editor_state::player_state;
use shrimply_project_document::project::{Color, ItemAddress, VideoItem, VideoItemContent};
use shrimply_visual_core::sanim::{Viewport, reflection as source};
use std::{
    path::PathBuf,
    sync::{Arc, Weak},
};

pub const PARAMETER_PATH_PREFIX: &str = "/sanim/parameters/";
const COMMIT: &str = "sanim-parameter";

#[cfg(test)]
#[path = "sanim_parameters/tests.rs"]
mod tests;

#[derive(Default)]
pub(crate) struct Tracker {
    selected: Option<(uuid::Uuid, PathBuf)>,
    discovery: Option<source::Discovery>,
    observed: Weak<source::Observation>,
    generation: u64,
    prepared: Weak<source::sanim_lang::Prepared>,
    diagnostic: Option<source::Diagnostic>,
    preparing: bool,
    error: Option<String>,
}
impl Tracker {
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    fn observation(&self) -> Result<Option<Arc<source::Observation>>, String> {
        if let Some((id, path)) = &self.selected
            && let Some(observation) = source::observation(*id, path)
        {
            return Ok(Some(observation));
        }
        self.discovery
            .as_ref()
            .map_or(Ok(None), source::Discovery::observation)
    }
    pub fn poll(&mut self) -> bool {
        match self.observation() {
            Ok(Some(observation)) => {
                let state = observation.snapshot();
                let next = Arc::downgrade(&observation);
                let observation_changed = !Weak::ptr_eq(&self.observed, &next);
                if !observation_changed
                    && self.generation == state.generation
                    && self.error.is_none()
                {
                    return false;
                }
                let prepared = state
                    .prepared
                    .as_ref()
                    .map(Arc::downgrade)
                    .unwrap_or_default();
                // Progress counters do not change this card; preserve active controls/focus.
                let changed = observation_changed
                    || !Weak::ptr_eq(&self.prepared, &prepared)
                    || self.diagnostic != state.diagnostic
                    || (state.prepared.is_none() && self.preparing != state.preparing)
                    || self.error.is_some();
                self.observed = next;
                self.generation = state.generation;
                self.prepared = prepared;
                self.diagnostic = state.diagnostic;
                self.preparing = state.preparing;
                self.error = None;
                changed
            }
            Ok(None) => {
                let changed = !Weak::ptr_eq(&self.observed, &Weak::new()) || self.error.is_some();
                self.observed = Weak::new();
                self.prepared = Weak::new();
                self.diagnostic = None;
                self.preparing = false;
                self.generation = 0;
                self.error = None;
                changed
            }
            Err(error) => {
                let changed = self.error.as_ref() != Some(&error);
                self.error = Some(error);
                changed
            }
        }
    }
    fn snapshot(
        &mut self,
        item: &VideoItem,
        viewport: Viewport,
    ) -> Result<Option<source::Snapshot>, String> {
        let VideoItemContent::Sanim(settings) = &item.content else {
            return Ok(None);
        };
        self.selected = Some((item.id, item.file.path().to_owned()));
        if let Some(observation) = source::observation(item.id, item.file.path()) {
            self.discovery = None;
            return Ok(Some(observation.snapshot()));
        }
        if self
            .discovery
            .as_ref()
            .is_none_or(|d| !d.matches(item, &viewport))
        {
            self.discovery = Some(source::Discovery::new(item, viewport)?);
        }
        let discovery = self.discovery.as_ref().expect("Sanim discovery");
        discovery.set_inputs(&settings.parameters)?;
        Ok(discovery.observation()?.map(|o| o.snapshot()))
    }
}

pub fn parameter_key(path: &str) -> Option<&str> {
    path.strip_prefix(PARAMETER_PATH_PREFIX)
        .filter(|key| !key.is_empty())
}
impl InspectorController {
    /// Poll published preparation state only; dependency watching belongs to the runtime.
    pub fn poll_sanim_parameters(&self) -> bool {
        self.sanim_reflection.borrow_mut().poll()
    }

    pub fn sanim_card(&self, item: &VideoItem) -> Option<VideoCard> {
        if !matches!(item.content, VideoItemContent::Sanim(_)) {
            self.sanim_reflection.borrow_mut().clear();
            return None;
        }
        let project = self.project.borrow();
        let viewport = source::viewport(
            project.canvas_size.width,
            project.canvas_size.height,
            project.fps,
        );
        drop(project);
        let state = self.sanim_reflection.borrow_mut().snapshot(item, viewport);
        Some(card(item, state))
    }
    fn sanim_definition(
        &self,
        target: &InspectorTarget,
        key: &str,
    ) -> Result<(Arc<source::sanim_lang::Prepared>, Arc<source::Declaration>), String> {
        let InspectorTarget::Item(address @ ItemAddress::Video { .. }) = target else {
            return Err("reflection target must be a Sanim video clip".into());
        };
        let project = self.project.borrow();
        let item = project
            .video_item(address)
            .ok_or("Sanim clip is no longer available")?;
        let viewport = source::viewport(
            project.canvas_size.width,
            project.canvas_size.height,
            project.fps,
        );
        let state = self
            .sanim_reflection
            .borrow_mut()
            .snapshot(item, viewport)?
            .ok_or(
                "Sanim reflection is still preparing; try again once its controls are available",
            )?;
        let prepared = state.prepared.ok_or_else(|| {
            state
                .diagnostic
                .map_or("Sanim reflection is not ready".into(), |e| e.to_string())
        })?;
        let declaration = source::schema(&prepared)?
            .declaration(key)
            .map_err(|e| e.to_string())?
            .clone();
        Ok((prepared, declaration))
    }
    pub fn set_sanim_parameter(
        &self,
        target: &InspectorTarget,
        key: &str,
        value: Option<serde_json::Value>,
        commit: &str,
    ) -> Result<(), String> {
        if commit.is_empty() {
            return Err("reflection edits require a commit name".into());
        }
        let (prepared, _) = self.sanim_definition(target, key)?;
        self.store_sanim_parameter(target, &prepared, key, value, commit)
    }
    fn store_sanim_parameter(
        &self,
        target: &InspectorTarget,
        prepared: &source::sanim_lang::Prepared,
        key: &str,
        value: Option<serde_json::Value>,
        commit: &str,
    ) -> Result<(), String> {
        if commit.is_empty() {
            return Err("reflection edits require a commit name".into());
        }
        let InspectorTarget::Item(address) = target else {
            return Err("reflection target must be a Sanim clip".into());
        };
        let mut project = self.project.borrow_mut();
        let item = project
            .video_item_mut(address)
            .ok_or("Sanim clip is no longer available")?;
        if !source::set(item, prepared, key, value)? {
            return Ok(());
        }
        shrimply_project_document::project::commit_edit(&project, commit);
        drop(project);
        player_state::refresh_project(
            &self.player_state,
            player_state::ProjectChange {
                video: true,
                inspector: true,
                ..Default::default()
            },
        );
        Ok(())
    }
    pub fn set_sanim_text(
        &self,
        target: &InspectorTarget,
        path: &str,
        text: &str,
        commit: &str,
    ) -> Option<Result<(), String>> {
        let key = parameter_key(path)?;
        Some((|| {
            let (prepared, declaration) = self.sanim_definition(target, key)?;
            let value = if declaration.ty.as_ref() == &source::Type::String {
                serde_json::Value::String(text.into())
            } else {
                serde_json::from_str(text)
                    .unwrap_or_else(|_| serde_json::Value::String(text.into()))
            };
            self.store_sanim_parameter(target, &prepared, key, Some(value), commit)
        })())
    }
    pub fn set_sanim_fraction(
        &self,
        target: &InspectorTarget,
        path: &str,
        value: shrimply_math_core::Fraction,
        commit: &str,
    ) -> Option<Result<(), String>> {
        let key = parameter_key(path)?;
        Some((|| {
            let (prepared, declaration) = self.sanim_definition(target, key)?;
            let value = match declaration.ty.as_ref() {
                source::Type::Numeric(unit) => {
                    let (_, suffix) = source::math::Number {
                        value: 0.into(),
                        unit: *unit,
                    }
                    .display_parts()?;
                    source::Value::Number(source::math::Number::parse(&format!("{value}{suffix}"))?)
                }
                source::Type::Fraction => source::Value::Fraction(value),
                _ => {
                    return Err(format!(
                        "{} requires {}; it is not a scalar numeric control",
                        declaration.label, declaration.ty
                    ));
                }
            };
            self.store_sanim_parameter(
                target,
                &prepared,
                key,
                Some(source::to_json(&value)?),
                commit,
            )
        })())
    }
    pub fn set_sanim_color(
        &self,
        target: &InspectorTarget,
        path: &str,
        value: Color<u8>,
        commit: &str,
    ) -> Option<Result<(), String>> {
        let key = parameter_key(path)?;
        Some(self.set_sanim_parameter(
            target,
            key,
            Some(serde_json::Value::String(format!(
                "#{:02x}{:02x}{:02x}{:02x}",
                value.r, value.g, value.b, value.a
            ))),
            commit,
        ))
    }
}
fn card(item: &VideoItem, state: Result<Option<source::Snapshot>, String>) -> VideoCard {
    let mut section = InspectorSection::default();
    match state {
        Ok(Some(state)) => {
            if let Some(prepared) = state.prepared {
                let VideoItemContent::Sanim(settings) = &item.content else {
                    unreachable!()
                };
                let inputs = source::effective_inputs(&prepared, &settings.parameters);
                let inputs = match inputs {
                    Ok(inputs) => inputs,
                    Err(error) => {
                        section.add(message(&error));
                        prepared.reflected.iter().cloned().collect()
                    }
                };
                for input in inputs {
                    match control(&input) {
                        Ok(control) => section.add(control),
                        Err(error) => section.add(message(&error)),
                    }
                }
                if prepared.reflected.is_empty() {
                    section.add(message("This source declares no reflected inputs."));
                }
            } else if state.preparing {
                section.add(message("Preparing reflected controls…"));
            }
            if let Some(error) = state.diagnostic {
                section.add(message(&error.to_string()));
            }
        }
        Ok(None) => section.add(message("Preparing reflected controls…")),
        Err(error) => section.add(message(&error)),
    }
    VideoCard::new("sanim-parameters", "Sanim parameters", section).reset(
        "/content/parameters",
        serde_json::json!({}),
        "reset-sanim-parameters",
    )
}
fn message(text: &str) -> InspectorControl {
    InspectorControl::new(ControlKind::ReadOnly, "", "").value(text)
}
fn control(input: &source::ReflectedInput) -> Result<InspectorControl, String> {
    let path = format!("{PARAMETER_PATH_PREFIX}{}", input.key);
    let base = |kind| {
        InspectorControl::new(kind, path.clone(), &input.label)
            .tooltip(format!(
                "{}\n{}\n{}:{}",
                input.key, input.ty, input.source, input.span.line
            ))
            .immediate_commit(COMMIT)
    };
    if matches!(input.ty.as_ref(), source::Type::Union(_)) {
        return Ok(base(ControlKind::Text).value(
            serde_json::to_string(&source::to_json(&input.value)?)
                .map_err(|error| error.to_string())?,
        ));
    }
    Ok(match &input.value {
        source::Value::Color(color) => base(ControlKind::Color).components(
            color
                .0
                .iter()
                .map(|v| source::math::to_f64(*v).map(|v| (v * 255.).round().to_string()))
                .collect::<Result<Vec<_>, _>>()?,
        ),
        source::Value::Bool(value) => base(ControlKind::Boolean).value(value.to_string()),
        source::Value::String(value) => base(ControlKind::Text).value(value.to_string()),
        source::Value::Number(value) => {
            let (display, suffix) = value.display_parts()?;
            numeric_control(base, display, &input.value, suffix)?
        }
        source::Value::Fraction(value) => numeric_control(base, *value, &input.value, "")?,
        _ => base(ControlKind::Text).value(
            serde_json::to_string(&source::to_json(&input.value)?).map_err(|e| e.to_string())?,
        ),
    })
}
fn numeric_control(
    base: impl Fn(ControlKind) -> InspectorControl,
    value: shrimply_math_core::Fraction,
    original: &source::Value,
    suffix: &'static str,
) -> Result<InspectorControl, String> {
    let text = value.to_string();
    let (numerator, denominator) = text.split_once('/').unwrap_or((&text, "1"));
    if numerator.parse::<i64>().is_ok() && denominator.parse::<i64>().is_ok() {
        Ok(base(ControlKind::Fraction)
            .components(vec![numerator.into(), denominator.into()])
            .suffix(suffix)
            .number(NumberSpec {
                minimum: i64::MIN as f64,
                maximum: i64::MAX as f64,
                drag_step: 0.1,
                digits: 3,
                unit: suffix,
            }))
    } else {
        let json = source::to_json(original)?;
        Ok(base(ControlKind::Text).value(
            json.as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| json.to_string()),
        ))
    }
}
