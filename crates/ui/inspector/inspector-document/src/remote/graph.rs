//! Retained shared framegraph session for native hosts with opaque control handles.
use serde_json::{Value, json};
use shrimply_framegraph_skia::{
    FRAME_GRAPH_CONTEXT_ACTIONS, FrameGraphAction, FrameGraphCommand, FrameGraphCore,
    FrameGraphInputResult,
};
use shrimply_i18n::text;
use shrimply_inspector_core::{
    InspectorControl, InspectorController, InspectorGraphEdit, InspectorTarget,
};

use crate::InspectorDocument;

pub struct GraphSession {
    pub core: FrameGraphCore,
    target: InspectorTarget,
    control: InspectorControl,
    pub serial: u64,
    pub pointer: shrimply_framegraph_skia::FrameGraphPointerPosition,
    edit: InspectorGraphEdit,
    menu: Option<Value>,
    snapping: (bool, f64),
}

impl GraphSession {
    pub fn open(
        controller: &InspectorController,
        document: &InspectorDocument,
        request: &Value,
        serial: u64,
    ) -> Result<Self, String> {
        if controller.target() != document.target {
            return Err("Inspector selection changed".into());
        }
        let control = super::read_control(document, request)?.clone();
        let state = shrimply_inspector_core::document::graph::frame_graph(&control)?;
        Ok(Self {
            core: FrameGraphCore::single(state),
            target: document.target.clone(),
            control,
            serial,
            pointer: shrimply_framegraph_skia::FrameGraphPointerPosition {
                x: 0.0,
                y: 0.0,
                width: 0.0,
                height: 0.0,
            },
            edit: Default::default(),
            menu: None,
            snapping: Default::default(),
        })
    }

    pub fn refresh(&mut self, controller: &InspectorController) -> Result<(), String> {
        if controller.target() != self.target {
            return Err("Inspector selection changed".into());
        }
        self.control.scalar_graph = controller.control_graph(&self.target, &self.control)?;
        self.update_view()
    }

    fn update_view(&mut self) -> Result<(), String> {
        let state = shrimply_inspector_core::document::graph::frame_graph(&self.control)?;
        self.core.replace_active_graph(state.graph().clone());
        let graph = self
            .control
            .scalar_graph
            .as_ref()
            .expect("frame graph requires a graph");
        self.core.set_view(
            graph.range,
            graph.frame_step,
            graph.playhead,
            self.snapping,
            true,
            self.control.kind == shrimply_inspector_core::ControlKind::LayeredText,
        );
        Ok(())
    }

    pub fn configure_snapping(&mut self, enabled: bool, radius: f64) {
        if self.snapping == (enabled, radius) {
            return;
        }
        self.snapping = (enabled, radius);
        let graph = self
            .control
            .scalar_graph
            .as_ref()
            .expect("frame graph requires a graph");
        self.core.set_view(
            graph.range,
            graph.frame_step,
            graph.playhead,
            (enabled, radius),
            true,
            self.control.kind == shrimply_inspector_core::ControlKind::LayeredText,
        );
    }

    pub fn snapshot(&self) -> Value {
        let state = self.core.status();
        let action = |id: &str, label: &str, icon: &str, enabled: bool| json!({"label":label,"icon":icon,"enabled":enabled && self.is_editable(),"command":{"op":"framegraph_action","serial":self.serial,"action":id}});
        json!({"serial":self.serial,"surface":self.serial,"enabled":self.is_editable(),"height":self.core.preferred_height(),
            "actions":[
                action("previous",state.command_label(FrameGraphCommand::PreviousKey).as_ref(),"go-previous-symbolic",state.can_previous),
                action("toggle",state.command_label(FrameGraphCommand::ToggleKey).as_ref(),if state.key_at_playhead {"list-remove-symbolic"} else {"list-add-symbolic"},true),
                action("next",state.command_label(FrameGraphCommand::NextKey).as_ref(),"go-next-symbolic",state.can_next)
            ],"menu":self.menu})
    }

    pub fn request(
        &mut self,
        controller: &InspectorController,
        request: &Value,
    ) -> Result<(), String> {
        if request["serial"].as_u64() != Some(self.serial) {
            return Err("Frame graph changed".into());
        }
        let action = request["action"]
            .as_str()
            .ok_or("Frame graph action is required")?;
        if action == "menu-dismiss" {
            self.menu = None;
            return Ok(());
        }
        if controller.target() != self.target {
            return Err("Inspector selection changed".into());
        }
        if !self.is_editable() {
            return Err("Inspector control is unavailable".into());
        }
        self.finish(controller)?;
        let result = match action {
            "previous" => self.core.command(FrameGraphCommand::PreviousKey),
            "toggle" => self.core.command(FrameGraphCommand::ToggleKey),
            "next" => self.core.command(FrameGraphCommand::NextKey),
            "interpolation" => {
                let owner = request["owner"]
                    .as_str()
                    .ok_or("Interpolation owner is required")?
                    .parse()
                    .map_err(|_| "Invalid interpolation owner")?;
                let index = usize::try_from(
                    request["index"]
                        .as_u64()
                        .ok_or("Interpolation choice is required")?,
                )
                .map_err(|_| "Interpolation choice is out of range")?;
                controller.set_control_interpolation(&self.target, &self.control, owner, index)?;
                self.menu = None;
                return self.refresh(controller);
            }
            "text-interpolation" => {
                let owner = request["owner"]
                    .as_str()
                    .ok_or("Interpolation owner is required")?
                    .parse()
                    .map_err(|_| "Invalid interpolation owner")?;
                let index = usize::try_from(
                    request["index"]
                        .as_u64()
                        .ok_or("Interpolation choice is required")?,
                )
                .map_err(|_| "Interpolation choice is out of range")?;
                controller.set_control_text_interpolation(
                    &self.target,
                    &self.control,
                    owner,
                    index,
                )?;
                self.menu = None;
                return self.refresh(controller);
            }
            id => self.core.key(
                FRAME_GRAPH_CONTEXT_ACTIONS
                    .into_iter()
                    .find(|action| action.id == id)
                    .ok_or("Unknown frame graph action")?
                    .key,
            ),
        };
        self.menu = None;
        self.apply(controller, result, false)?;
        self.finish(controller)
    }

    pub fn apply(
        &mut self,
        controller: &InspectorController,
        result: FrameGraphInputResult,
        touch: bool,
    ) -> Result<Option<bool>, String> {
        if controller.target() != self.target {
            // A native surface can deliver its last pointer packet or idle
            // poll before the replacement inspector has detached the old view.
            self.cancel(controller)?;
            return Ok(None);
        }
        if result.actions.is_empty()
            && !result.redraw
            && !result.cancelled
            && result.history.is_none()
        {
            return Ok(None);
        }
        if result.cancelled {
            self.edit.cancel(controller)?;
        }
        for action in result.actions {
            match action.action {
                FrameGraphAction::ContextMenuRequested { x, y, touch } => {
                    let items = FRAME_GRAPH_CONTEXT_ACTIONS.into_iter().map(|action| json!({
                        "label":text(action.label), "icon":action.icon, "enabled":self.is_editable(),
                        "command":{"op":"framegraph_action","serial":self.serial,"action":action.id}
                    })).collect();
                    self.menu = Some(self.choice_menu(items, x, y, touch));
                }
                FrameGraphAction::InterpolationRequested {
                    owner_id,
                    interpolation,
                    x,
                    y,
                    ..
                } => {
                    let items = shrimply_property_model::timeline_value::Interpolation::KEYFRAME.into_iter().enumerate().map(|(index,value)|json!({"label":text(value.label()),"selected":value == interpolation,"command":{"op":"framegraph_action","serial":self.serial,"action":"interpolation","owner":owner_id,"index":index}})).collect::<Vec<_>>();
                    self.menu = Some(self.choice_menu(items, x, y, touch));
                }
                FrameGraphAction::TextInterpolationRequested { owner_id, x, y, .. } => {
                    let menu = controller.control_text_interpolation_menu(
                        &self.target,
                        &self.control,
                        owner_id,
                    )?;
                    let items = menu.values.into_iter().zip(menu.labels).map(|(value,label)|json!({"label":text(&label),"selected":value == menu.value,"command":{"op":"framegraph_action","serial":self.serial,"action":"text-interpolation","owner":owner_id,"index":value.parse::<usize>().expect("text interpolation index")}})).collect();
                    self.menu = Some(self.choice_menu(items, x, y, touch));
                }
                action => self
                    .edit
                    .apply(controller, &self.target, action, |action| {
                        controller.apply_control_graph_action(&self.target, &self.control, action)
                    })?,
            }
        }
        self.refresh(controller)?;
        Ok(result.history)
    }

    pub fn touch(
        &mut self,
        controller: &InspectorController,
        changes: &[shrimply_components_skia::input::TouchChange],
        position: shrimply_framegraph_skia::FrameGraphPointerPosition,
    ) -> Result<Option<bool>, String> {
        let result = self.core.touch(changes, position);
        self.apply(controller, result, true)
    }

    pub fn is_editable(&self) -> bool {
        self.control.editable && self.control.sensitive
    }

    pub fn is_editing(&self) -> bool {
        self.edit.is_editing()
    }

    fn choice_menu(&self, items: Vec<Value>, x: f64, y: f64, touch: bool) -> Value {
        json!({"items":items,"x":x,"y":y,"bottom_sheet":shrimply_components_core::layout::menu_uses_bottom_sheet(if touch {"touch"} else {"pointer"}),"dismiss":{"op":"framegraph_action","serial":self.serial,"action":"menu-dismiss"}})
    }

    pub fn finish(&mut self, controller: &InspectorController) -> Result<(), String> {
        self.edit.finish(controller)
    }

    pub fn cancel(&mut self, controller: &InspectorController) -> Result<(), String> {
        let mut result = self.core.cancel_touch();
        // End-pointer may return EditFinished for a mouse drag. Roll back its
        // provisional edit before dispatching any returned lifecycle actions.
        self.edit.cancel(controller)?;
        result.cancelled = false;
        if result.actions.is_empty() || controller.target() != self.target {
            Ok(())
        } else {
            self.apply(controller, result, true).map(|_| ())
        }
    }
}

/// Per-property retained views. Native hosts only route surfaces by the advertised ID.
#[derive(Default)]
pub struct Graphs {
    pub sessions: std::collections::BTreeMap<usize, GraphSession>,
    handles: std::collections::HashMap<String, usize>,
    next_surface: usize,
}

impl Graphs {
    pub fn request(
        &mut self,
        controller: &InspectorController,
        request: &Value,
    ) -> Result<(), String> {
        let surface = request["serial"]
            .as_u64()
            .and_then(|id| usize::try_from(id).ok())
            .ok_or("Frame graph is required")?;
        match self.sessions.get_mut(&surface) {
            Some(graph) => graph.request(controller, request),
            None if request["action"] == "menu-dismiss" => Ok(()),
            None => Err("Frame graph changed".into()),
        }
    }

    pub fn clear(&mut self) {
        self.sessions.clear();
        self.handles.clear();
    }

    pub fn cancel(&mut self, controller: &InspectorController) -> Result<(), String> {
        for graph in self.sessions.values_mut() {
            graph.cancel(controller)?;
        }
        Ok(())
    }

    pub fn synchronize(
        &mut self,
        controller: &InspectorController,
        document: &InspectorDocument,
        snapshot: &mut Value,
    ) -> Result<(), String> {
        let mut retained = std::collections::HashSet::new();
        let mut errors = Vec::new();
        for category in snapshot["categories"]
            .as_array_mut()
            .expect("inspector categories")
        {
            for item in category["items"].as_array_mut().expect("inspector items") {
                let item_id = item["id"].clone();
                for control in item["controls"].as_array_mut().expect("inspector controls") {
                    self.decorate_control(
                        controller,
                        document,
                        &item_id,
                        control,
                        &mut retained,
                        &mut errors,
                    );
                }
            }
        }
        for (surface, graph) in &mut self.sessions {
            if !retained.contains(surface)
                && let Err(error) = graph.cancel(controller)
            {
                errors.push(error);
                retained.insert(*surface);
            }
        }
        self.sessions
            .retain(|surface, _| retained.contains(surface));
        self.handles.retain(|_, surface| retained.contains(surface));
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }

    fn decorate_control(
        &mut self,
        controller: &InspectorController,
        document: &InspectorDocument,
        item: &Value,
        control: &mut Value,
        retained: &mut std::collections::HashSet<usize>,
        errors: &mut Vec<String>,
    ) {
        if let Some(children) = control.get_mut("controls").and_then(Value::as_array_mut) {
            for child in children {
                self.decorate_control(controller, document, item, child, retained, errors);
            }
        }
        if control["expression_editor"].is_null() && control["graph"].is_null() {
            return;
        }
        let request = json!({"item":item,"control":control["id"]});
        let result = (|| {
            let model = super::read_control(document, &request)?;
            if !control["expression_editor"].is_null() {
                let output = match controller.control_expression_output(&document.target, model) {
                    Ok(output) => output,
                    Err(error) => shrimply_inspector_core::InspectorExpressionOutput {
                        value: String::new(),
                        error: Some(error),
                    },
                };
                control["expression_editor"]["output"] =
                    json!({"label":text("Output"),"value":output.value,"error":output.error});
            }
            if control["graph"].is_null() {
                return Ok::<_, String>(());
            }
            let handle = request.to_string();
            let surface = if let Some(surface) = self.handles.get(&handle) {
                *surface
            } else {
                let surface = self.next_surface.max(2);
                self.next_surface = surface
                    .checked_add(1)
                    .filter(|id| *id <= i32::MAX as usize)
                    .expect("inspector surface ID overflow");
                let graph = GraphSession::open(controller, document, &request, surface as u64)?;
                self.sessions.insert(surface, graph);
                self.handles.insert(handle, surface);
                surface
            };
            let graph = self.sessions.get_mut(&surface).expect("retained graph");
            if graph.is_editing() && (!model.editable || !model.sensitive) {
                graph.cancel(controller)?;
            }
            if !graph.is_editing() {
                graph.control = model.clone();
                graph.update_view()?;
            }
            retained.insert(surface);
            control["graph"] = graph.snapshot();
            Ok(())
        })();
        if let Err(error) = result {
            if !control["graph"].is_null() {
                control["graph"] = Value::Null;
                control["graph_error"] = json!(error);
            }
            if !control["expression_editor"].is_null()
                && control["expression_editor"]["output"].is_null()
            {
                control["expression_editor"]["output"] =
                    json!({"label":text("Output"),"value":"","error":error});
            }
            errors.push(error);
        }
    }
}
