//! Retained control edits shared by native inspector bridges.
use std::collections::HashMap;

use serde_json::{Value, json};
use shrimply_components_core::number::{self, NumberConfig, NumberDrag};
use shrimply_i18n::text;
use shrimply_inspector_core::{
    ControlKind, InspectorControl, InspectorController, InspectorEditTransaction, InspectorTarget,
};
use shrimply_math_core::{Fraction, fraction_as_f64};

use crate::InspectorDocument;

#[derive(Default)]
pub struct Interaction {
    list: shrimply_inspector_core::list::InspectorListState,
    active: Option<Edit>,
    locks: HashMap<String, Lock>,
    sessions: HashMap<u64, ControlSession>,
    expression: Option<u64>,
    next_session: u64,
}

struct ControlSession {
    key: String,
    target: InspectorTarget,
    control: InspectorControl,
    history_revision: u64,
    displayed: bool,
}

struct Lock {
    enabled: bool,
    values: Vec<Fraction>,
}

struct Edit {
    serial: u64,
    detached: bool,
    key: String,
    target: InspectorTarget,
    control: InspectorControl,
    axis: usize,
    origin: [f64; 2],
    touch: bool,
    drag: Option<NumberDrag>,
    values: Vec<Fraction>,
    transaction: InspectorEditTransaction,
}

fn key(request: &Value) -> String {
    json!([request["item"], request["control"]]).to_string()
}

pub(super) fn values(control: &InspectorControl) -> Result<Vec<Fraction>, String> {
    let values = if control.components.is_empty() {
        vec![control.value.as_str()]
    } else {
        control.components.iter().map(String::as_str).collect()
    };
    values
        .into_iter()
        .map(|value| {
            number::parse_fraction(value).ok_or_else(|| "Inspector number must be finite".into())
        })
        .collect()
}

fn config(control: &InspectorControl, value: Fraction) -> NumberConfig {
    NumberConfig {
        minimum: number::from_f64(control.number.minimum),
        maximum: number::from_f64(control.number.maximum),
        drag_step: number::from_f64(control.number.drag_step),
        digits: usize::try_from(control.number.digits)
            .expect("numeric control digits must be nonnegative"),
        fallback: value,
        ..NumberConfig::new(value)
    }
}

pub(super) fn presentation(control: &InspectorControl, values: &[Fraction], locked: bool) -> Value {
    let fields = values.iter().enumerate().map(|(axis,value)| {
        let config = config(control,*value);
        let degrees = if control.prefix_icon_rotates {i64::from(shrimply_components_core::math::number_rotation(fraction_as_f64(*value),control.prefix_icon_rotation_offset_degrees))} else {0};
        json!({"axis":axis,"display":number::format_value(&config,*value),"edit_value":value.to_string(),"prefix":control.prefixes.get(axis).map_or("",String::as_str),"suffix":if control.suffix.is_empty() {control.number.unit} else {&control.suffix},
            "icon":control.prefix_icon,"icon_rotation":degrees,"digits":config.digits,"minimum":config.minimum.to_string(),"maximum":config.maximum.to_string(),"drag_step":config.drag_step.to_string(),"keyboard":if control.kind == ControlKind::Fraction {"text"} else {"decimal"}})
    }).collect::<Vec<_>>();
    json!({"interaction_active":false,"fields":fields,"lock":control.lock.then(||json!({"checked":locked,"label":text("Lock ratio"),"icon":if locked {"padlock2-symbolic"} else {"padlock2-open-symbolic"}}))})
}

impl Interaction {
    pub fn active(&self) -> bool {
        self.active.as_ref().is_some_and(|edit| !edit.detached)
    }

    pub fn synchronize(&mut self, controller: &InspectorController) -> Result<(), String> {
        if self
            .active
            .as_ref()
            .is_some_and(|edit| !edit.detached && edit.target != controller.target())
        {
            self.cancel(controller)?;
        }
        Ok(())
    }

    /// A disposed editor closes its own history group, even after selection
    /// changes. It must never resolve the replacement control or end its edit.
    fn complete(&mut self, request: &Value) -> Result<Option<Value>, String> {
        if request["action"] == "expression_finish" {
            if !request["item"].is_string() || !request["control"].is_string() {
                return Err("Expression completion requires its control handle".into());
            }
            let serial = request["edit_serial"]
                .as_u64()
                .ok_or("Expression session is required")?;
            if self
                .sessions
                .get(&serial)
                .is_some_and(|edit| edit.key != key(request))
            {
                return Err("Expression control changed".into());
            }
            if self.expression == Some(serial) {
                self.finish_expression();
            }
            self.release_session(request);
            return Ok(Some(Value::Null));
        }
        if request["op"] == "inspector_numeric"
            && matches!(
                request["event"].as_str(),
                Some("move" | "end" | "cancel" | "commit" | "release")
            )
            && self.active.as_ref().is_none_or(|edit| {
                edit.key != key(request) || request["edit_serial"].as_u64() != Some(edit.serial)
            })
        {
            if request["event"] == "release" {
                self.release_session(request);
            }
            return Ok(Some(
                json!({"active":false,"capture":false,"editing":false,"valid":true}),
            ));
        }
        Ok(None)
    }

    fn release_session(&mut self, request: &Value) {
        if let Some(serial) = request["edit_serial"].as_u64()
            && self
                .sessions
                .get(&serial)
                .is_some_and(|edit| !edit.displayed && edit.key == key(request))
            && self.expression != Some(serial)
            && self
                .active
                .as_ref()
                .is_none_or(|edit| edit.serial != serial)
        {
            self.sessions.remove(&serial);
        }
    }

    fn finish_expression(&mut self) {
        if let Some(serial) = self.expression.take()
            && let Some(edit) = self.sessions.get(&serial)
            && edit.history_revision == shrimply_project_document::project::history_revision()
        {
            shrimply_project_document::project::finish_coalesced_edit();
        }
    }

    /// Drain an advertised editor against its original target, even when native
    /// focus loss follows a timeline selection change. Other edits invalidate it.
    pub fn request(
        &mut self,
        controller: &InspectorController,
        request: &Value,
        busy: bool,
    ) -> Result<Option<Value>, String> {
        if request["op"] == "inspector_numeric"
            && self.active.as_ref().is_some_and(|edit| {
                edit.detached
                    && edit.key == key(request)
                    && request["edit_serial"].as_u64() == Some(edit.serial)
            })
            && matches!(
                request["event"].as_str(),
                Some("text" | "commit" | "release" | "cancel")
            )
        {
            if busy {
                return Err(
                    "Finish the current control gesture before completing the number".into(),
                );
            }
            let edit = self.active.as_ref().expect("retained number");
            let target = edit.target.clone();
            let control = edit.control.clone();
            return self
                .apply_number(controller, &target, &control, request)
                .map(Some);
        }
        if let Some(completed) = self.complete(request)? {
            return Ok(Some(completed));
        }
        if request["action"] != "edit" || request["editor"] != "expression" {
            return Ok(None);
        }
        let serial = request["edit_serial"]
            .as_u64()
            .ok_or("Expression session is required")?;
        let edit = self
            .sessions
            .get(&serial)
            .ok_or("Expression editor is closed")?;
        if edit.key != key(request) {
            return Err("Expression control changed".into());
        }
        if busy || self.active.is_some() {
            return Err("Finish the current control gesture before editing the expression".into());
        }
        let revision = shrimply_project_document::project::history_revision();
        if edit.history_revision != revision {
            return Err("The project changed while the expression editor was open".into());
        }
        if self.expression != Some(serial) {
            self.finish_expression();
        }
        let edit = self.sessions.get_mut(&serial).expect("retained expression");
        let result = super::apply_expression(controller, &edit.target, &edit.control, request);
        self.advance_sessions(revision);
        self.expression = Some(serial);
        result?;
        Ok(Some(Value::Null))
    }

    pub fn apply_control(
        &mut self,
        controller: &InspectorController,
        document: &InspectorDocument,
        request: &Value,
    ) -> Result<Option<String>, String> {
        if request["action"] == "category" {
            if controller.target() != document.target {
                return Err("Inspector selection changed".into());
            }
            let category = request["category"]
                .as_str()
                .ok_or("Inspector category is required")?;
            if !document
                .categories
                .iter()
                .any(|candidate| candidate.key == category)
            {
                return Err("Inspector category is unavailable".into());
            }
            self.list.set_active_category(&document.target, category);
            return Ok(None);
        }
        if request["action"] == "expand" {
            if controller.target() != document.target {
                return Err("Inspector selection changed".into());
            }
            let (_, header) = super::resolve_item(document, request)?;
            let header = header.ok_or("Inspector section has no header")?;
            let expanded = request["expanded"]
                .as_bool()
                .ok_or("Expansion state is required")?;
            self.list
                .set_expanded(&document.target, &header.presentation.key, expanded);
            return Ok(None);
        }
        if self.request(controller, request, false)?.is_some() {
            return Ok(None);
        }
        self.synchronize(controller)?;
        self.finish_expression();
        super::apply(controller, document, request)
    }

    pub fn clear(&mut self) {
        self.list = Default::default();
        self.active = None;
        self.locks.clear();
        self.sessions.clear();
        self.expression = None;
    }

    pub fn cancel(&mut self, controller: &InspectorController) -> Result<(), String> {
        if let Some(edit) = self.active.take() {
            edit.transaction.cancel(controller)?;
        }
        Ok(())
    }

    pub fn snapshot(&mut self, document: &InspectorDocument) -> Value {
        for edit in self.sessions.values_mut() {
            edit.displayed = false;
        }
        let mut snapshot = super::snapshot(document);
        let categories = snapshot["categories"]
            .as_array_mut()
            .expect("inspector categories");
        let selected = self
            .list
            .active_category(&document.target)
            .and_then(|key| {
                categories
                    .iter()
                    .position(|category| category["key"] == key)
            })
            .unwrap_or_default();
        for (index, category) in categories.iter_mut().enumerate() {
            category["selected"] = json!(index == selected);
            category["command"] = json!({"action":"category","category":category["key"]});
            for item in category["items"].as_array_mut().expect("inspector items") {
                if item["card"] == true {
                    let expanded = self.list.expanded(
                        &document.target,
                        item["key"].as_str().expect("inspector item key"),
                    );
                    item["expanded"] = json!(expanded);
                    item["expansion"] = json!({"label":text(if expanded {"Collapse"} else {"Expand"}),
                        "command":{"item":item["id"],"action":"expand","expanded":!expanded}});
                }
                for control in item["controls"].as_array_mut().expect("inspector controls") {
                    if control["expression_editor"].is_object() {
                        let serial =
                            self.advertise(document, &control["expression_editor"]["command"]);
                        control["expression_editor"]["command"]["edit_serial"] = json!(serial);
                        control["expression_editor"]["finish"]["edit_serial"] = json!(serial);
                    }
                    if control["widget"] != "number" {
                        continue;
                    }
                    let serial = self.advertise(document, &control["command"]);
                    control["command"]["edit_serial"] = json!(serial);
                    let key = key(&control["command"]);
                    control["interaction_active"] =
                        json!(self.active.as_ref().is_some_and(|edit| edit.key == key));
                    if let Some(edit) = self.active.as_ref().filter(|edit| edit.key == key) {
                        let locked = self.locked(&key, &edit.control);
                        let number = presentation(&edit.control, &edit.values, locked);
                        for (field, value) in number.as_object().expect("number widget") {
                            if field != "interaction_active" {
                                control[field] = value.clone();
                            }
                        }
                    } else if let Some(lock) = self.locks.get(&key)
                        && control["lock"].is_object()
                    {
                        control["lock"]["checked"] = json!(lock.enabled);
                        control["lock"]["icon"] = json!(if lock.enabled {
                            "padlock2-symbolic"
                        } else {
                            "padlock2-open-symbolic"
                        });
                    }
                }
            }
        }
        let revision = shrimply_project_document::project::history_revision();
        self.sessions.retain(|serial, edit| {
            edit.displayed
                || edit.history_revision == revision
                || self.expression == Some(*serial)
                || self
                    .active
                    .as_ref()
                    .is_some_and(|active| active.serial == *serial)
        });
        snapshot
    }

    fn advertise(&mut self, document: &InspectorDocument, request: &Value) -> u64 {
        let key = key(request);
        let revision = shrimply_project_document::project::history_revision();
        let serial = self
            .sessions
            .iter()
            .find_map(|(serial, edit)| {
                (edit.key == key && edit.history_revision == revision).then_some(*serial)
            })
            .unwrap_or_else(|| {
                self.next_session = self
                    .next_session
                    .checked_add(1)
                    .expect("control session overflow");
                let mut control = super::read_control(document, request)
                    .expect("advertised inspector control")
                    .clone();
                // Source editing needs property identity/configuration, not a graph picture.
                control.scalar_graph = None;
                self.sessions.insert(
                    self.next_session,
                    ControlSession {
                        key,
                        target: document.target.clone(),
                        control,
                        history_revision: revision,
                        displayed: false,
                    },
                );
                self.next_session
            });
        self.sessions
            .get_mut(&serial)
            .expect("retained control")
            .displayed = true;
        serial
    }

    fn advance_sessions(&mut self, revision: u64) {
        let next = shrimply_project_document::project::history_revision();
        for edit in self.sessions.values_mut() {
            if edit.history_revision == revision {
                edit.history_revision = next;
            }
        }
    }

    /// Focus loss follows raw native input. Preserve typed edits until their last packet.
    pub fn external_input(&mut self, controller: &InspectorController) -> Result<(), String> {
        if let Some(edit) = self.active.as_mut().filter(|edit| edit.drag.is_none()) {
            edit.detached = true;
            Ok(())
        } else {
            self.finish(controller)
        }
    }

    fn locked(&self, key: &str, control: &InspectorControl) -> bool {
        self.locks
            .get(key)
            .map_or(control.lock, |lock| lock.enabled)
    }

    pub fn finish(&mut self, controller: &InspectorController) -> Result<(), String> {
        if let Some(mut edit) = self.active.take() {
            let revision = shrimply_project_document::project::history_revision();
            let commit = |controller: &InspectorController| {
                controller.commit_basic_control(&edit.target, &edit.control)
            };
            if edit.detached {
                edit.transaction.apply_retained(controller, commit)?;
            } else {
                edit.transaction.apply(controller, commit)?;
            }
            edit.transaction.finish(controller)?;
            self.advance_sessions(revision);
        }
        Ok(())
    }

    pub fn apply(
        &mut self,
        controller: &InspectorController,
        document: &InspectorDocument,
        request: &Value,
    ) -> Result<Value, String> {
        self.synchronize(controller)?;
        if let Some(completed) = self.complete(request)? {
            return Ok(completed);
        }
        if controller.target() != document.target {
            return Err("Inspector selection changed".into());
        }
        let control = super::resolve_control(document, request)?;
        self.apply_number(controller, &document.target, control, request)
    }

    fn apply_number(
        &mut self,
        controller: &InspectorController,
        target: &InspectorTarget,
        control: &InspectorControl,
        request: &Value,
    ) -> Result<Value, String> {
        let result = self.update_number(controller, target, control, request);
        if let Err(error) = &result
            && self.active.as_ref().is_some_and(|edit| {
                edit.key == key(request) && request["edit_serial"].as_u64() == Some(edit.serial)
            })
        {
            self.cancel(controller)
                .map_err(|cleanup| format!("{error}; {cleanup}"))?;
        }
        result
    }

    fn update_number(
        &mut self,
        controller: &InspectorController,
        target: &InspectorTarget,
        control: &InspectorControl,
        request: &Value,
    ) -> Result<Value, String> {
        if !control.editable
            || !matches!(
                control.kind,
                ControlKind::Number
                    | ControlKind::Fraction
                    | ControlKind::LayeredNumber
                    | ControlKind::Vector2
                    | ControlKind::Vector3
                    | ControlKind::LayeredVector2
                    | ControlKind::LayeredVector3
            )
        {
            return Err("Inspector control is not an editable number".into());
        }
        let key = key(request);
        let event = request["event"]
            .as_str()
            .ok_or("Numeric input event is required")?;
        if !matches!(
            event,
            "begin" | "edit" | "move" | "end" | "cancel" | "text" | "commit" | "lock" | "release"
        ) {
            return Err("Unknown numeric input event".into());
        }
        let serial = request["edit_serial"]
            .as_u64()
            .ok_or("Numeric session is required")?;
        let session = self
            .sessions
            .get(&serial)
            .ok_or("Numeric editor is closed")?;
        if session.key != key
            || session.history_revision != shrimply_project_document::project::history_revision()
            || self
                .active
                .as_ref()
                .is_some_and(|edit| edit.key == key && edit.serial != serial)
        {
            return Err("Numeric control changed".into());
        }
        let current = self
            .active
            .as_ref()
            .filter(|edit| edit.key == key)
            .map_or_else(|| values(control), |edit| Ok(edit.values.clone()))?;
        if event == "lock" {
            self.finish(controller)?;
            if !control.lock {
                return Err("Numeric control does not support a ratio lock".into());
            }
            let enabled = !self.locked(&key, control);
            self.locks.insert(
                key.clone(),
                Lock {
                    enabled,
                    values: current.clone(),
                },
            );
            return Ok(self.response(control, &current, false, true, &key));
        }
        if matches!(event, "begin" | "edit") {
            self.finish(controller)?;
            let axis = request["axis"]
                .as_u64()
                .and_then(|axis| usize::try_from(axis).ok())
                .filter(|axis| *axis < current.len())
                .ok_or("Numeric axis is unavailable")?;
            let enabled = self.locked(&key, control);
            self.locks.insert(
                key.clone(),
                Lock {
                    enabled,
                    values: current.clone(),
                },
            );
            let origin = if event == "begin" {
                position(request)?
            } else {
                [0.0; 2]
            };
            self.active = Some(Edit {
                serial,
                detached: false,
                key: key.clone(),
                target: target.clone(),
                control: control.clone(),
                axis,
                origin,
                touch: request["touch"].as_bool().unwrap_or(false),
                drag: (event == "begin").then(|| NumberDrag::begin(current[axis])),
                values: current.clone(),
                transaction: controller.begin_control_edit(target)?,
            });
        } else if self.active.as_ref().is_none_or(|edit| edit.key != key) {
            if matches!(event, "move" | "end" | "cancel" | "commit") {
                return Ok(self.response(control, &current, false, true, &key));
            }
            return Err("Numeric editor is closed".into());
        }
        if request["contacts"]
            .as_u64()
            .is_some_and(|contacts| contacts > 1)
        {
            return self.cancel_input(controller, control, &key);
        }
        if event == "release" {
            if self.active.as_ref().is_some_and(|edit| edit.drag.is_none()) {
                let values = self
                    .active
                    .as_ref()
                    .expect("retained number")
                    .values
                    .clone();
                self.finish(controller)?;
                self.release_session(request);
                return Ok(self.response(control, &values, false, true, &key));
            }
            return self.cancel_input(controller, control, &key);
        }
        if event == "cancel" {
            return self.cancel_input(controller, control, &key);
        }
        if matches!(event, "text" | "commit")
            && request["axis"].as_u64().is_some_and(|axis| {
                axis != self.active.as_ref().expect("numeric input state").axis as u64
            })
        {
            return Ok(self.response(control, &current, false, true, &key));
        }
        let mut capture = false;
        let mut valid = true;
        if matches!(event, "move" | "end") {
            let position = position(request)?;
            let edit = self.active.as_ref().expect("numeric input state");
            let offset = [position[0] - edit.origin[0], position[1] - edit.origin[1]];
            if edit.touch
                && !edit.drag.as_ref().is_some_and(NumberDrag::moved)
                && offset[1].abs() > offset[0].abs()
                && offset[1].abs() >= number::DRAG_THRESHOLD_PIXELS
            {
                return self.cancel_input(controller, control, &key);
            }
            let edit = self.active.as_mut().expect("numeric input state");
            if let Some(drag) = &mut edit.drag {
                capture = drag.update_absolute(offset[0]);
                if capture {
                    let value = drag.value(&config(&edit.control, edit.values[edit.axis]));
                    apply_value(edit, self.locks.get(&key), controller, value)?;
                }
            }
        }
        if matches!(event, "text" | "commit") {
            let text = request["text"]
                .as_str()
                .ok_or("Numeric input requires text")?;
            if let Some(value) = number::parse_fraction(text.trim()) {
                let edit = self.active.as_mut().expect("numeric input state");
                let config = config(&edit.control, edit.values[edit.axis]);
                let value = if edit.control.kind == ControlKind::Fraction {
                    number::clamped_value(value, config.minimum, config.maximum)
                } else {
                    number::accepted_value(&config, value)
                };
                apply_value(edit, self.locks.get(&key), controller, value)?;
            } else {
                valid = false;
            }
        }
        let completed_values = self
            .active
            .as_ref()
            .expect("numeric input state")
            .values
            .clone();
        if event == "end" {
            let edit = self.active.as_mut().expect("numeric input state");
            let offset = position(request)?[0] - edit.origin[0];
            if edit.drag.as_ref().is_some_and(|drag| drag.is_click(offset)) {
                edit.drag = None;
            } else {
                self.finish(controller)?;
            }
        } else if event == "commit" && valid {
            self.finish(controller)?;
        }
        let values = self
            .active
            .as_ref()
            .filter(|edit| edit.key == key)
            .map_or(completed_values, |edit| edit.values.clone());
        let mut response = self.response(control, &values, capture, valid, &key);
        if event == "text" {
            response["validation_message"] = json!("");
        }
        Ok(response)
    }

    fn cancel_input(
        &mut self,
        controller: &InspectorController,
        control: &InspectorControl,
        key: &str,
    ) -> Result<Value, String> {
        let baseline = values(self.active.as_ref().map_or(control, |edit| &edit.control))?;
        self.cancel(controller)?;
        Ok(self.response(control, &baseline, false, true, key))
    }

    fn response(
        &self,
        control: &InspectorControl,
        values: &[Fraction],
        capture: bool,
        valid: bool,
        key: &str,
    ) -> Value {
        let editing_axis = self
            .active
            .as_ref()
            .filter(|edit| edit.key == key && edit.drag.is_none())
            .map(|edit| edit.axis);
        json!({"number":presentation(control,values,self.locked(key,control)),"capture":capture && self.active.as_ref().is_some_and(|edit|edit.key == key && edit.drag.is_some()),"editing":editing_axis.is_some(),"editing_axis":editing_axis,"valid":valid,"validation_message":if valid {String::new()} else {text("Invalid finite number or fraction").into_owned()},"active":self.active.as_ref().is_some_and(|edit|edit.key == key)})
    }
}

fn position(request: &Value) -> Result<[f64; 2], String> {
    let x = request["x"]
        .as_f64()
        .filter(|value| value.is_finite())
        .ok_or("Numeric input requires a finite x coordinate")?;
    let y = request["y"]
        .as_f64()
        .filter(|value| value.is_finite())
        .ok_or("Numeric input requires a finite y coordinate")?;
    Ok([x, y])
}

fn apply_value(
    edit: &mut Edit,
    lock: Option<&Lock>,
    controller: &InspectorController,
    value: Fraction,
) -> Result<(), String> {
    let mut values = edit.values.clone();
    values[edit.axis] = value;
    if let Some(lock) = lock.filter(|lock| lock.enabled) {
        values = match lock.values.as_slice() {
            [first, second] => {
                number::locked_pair(edit.axis, value, number::pair_ratio(*first, *second)).to_vec()
            }
            [first, second, third] => number::locked_triple(
                edit.axis,
                value,
                number::triple_ratios([*first, *second, *third]),
            )
            .to_vec(),
            _ => values,
        };
        for value in &mut values {
            *value = number::accepted_value(&config(&edit.control, *value), *value);
        }
    }
    if edit.values == values {
        return Ok(());
    }
    let write = |controller: &InspectorController| {
        if edit.control.kind == ControlKind::Fraction {
            controller.set_basic_control_fraction(&edit.target, &edit.control, values[0])
        } else if edit.control.components.is_empty() {
            controller.set_basic_control_value(
                &edit.target,
                &edit.control,
                &fraction_as_f64(values[0]).to_string(),
            )
        } else {
            controller.set_basic_control_components(
                &edit.target,
                &edit.control,
                &values
                    .iter()
                    .copied()
                    .map(fraction_as_f64)
                    .collect::<Vec<_>>(),
            )
        }
    };
    if edit.detached {
        edit.transaction.apply_retained(controller, write)?;
    } else {
        edit.transaction.apply(controller, write)?;
    }
    edit.values = values;
    Ok(())
}
