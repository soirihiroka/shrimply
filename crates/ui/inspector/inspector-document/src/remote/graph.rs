//! Retained shared framegraph session for native hosts with opaque control handles.
use serde_json::{Value, json};
use shrimply_framegraph_skia::{
    FrameGraphAction, FrameGraphCommand, FrameGraphCore, FrameGraphInputResult, FrameGraphKey,
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
        let control = super::resolve_control(document, request)?.clone();
        let state = shrimply_inspector_core::document::graph::frame_graph(&control)?;
        Ok(Self {
            core: FrameGraphCore::single(state),
            target: document.target.clone(),
            control,
            serial,
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
        let action = |id: &str, label: &str, icon: &str, enabled: bool| json!({"label":text(label),"icon":icon,"enabled":enabled,"command":{"op":"framegraph_action","serial":self.serial,"action":id}});
        json!({"serial":self.serial,"title":text(&self.control.label),"close_label":text("Close"),"height":self.core.preferred_height(),
            "actions":[
                action("previous","Previous keyframe","media-seek-backward-symbolic",state.can_previous),
                action("toggle",if state.key_at_playhead {"Delete keyframe at playhead"} else {"Add keyframe at playhead"},if state.key_at_playhead {"user-trash-symbolic"} else {"list-add-symbolic"},true),
                action("next","Next keyframe","media-seek-forward-symbolic",state.can_next),
                action("copy","Copy keyframes","edit-copy-symbolic",true),
                action("paste","Paste keyframes","edit-paste-symbolic",true),
                action("delete","Delete keyframes","user-trash-symbolic",true),
                action("zoom-in","Zoom In","zoom-in-symbolic",true),
                action("zoom-out","Zoom Out","zoom-out-symbolic",true)
            ],"menu":self.menu,"dismiss":{"op":"framegraph_close","serial":self.serial}})
    }

    pub fn request(
        &mut self,
        controller: &InspectorController,
        request: &Value,
    ) -> Result<(), String> {
        if request["serial"].as_u64() != Some(self.serial) {
            return Err("Frame graph changed".into());
        }
        self.finish(controller)?;
        let action = request["action"]
            .as_str()
            .ok_or("Frame graph action is required")?;
        let result = match action {
            "previous" => self.core.command(FrameGraphCommand::PreviousKey),
            "toggle" => self.core.command(FrameGraphCommand::ToggleKey),
            "next" => self.core.command(FrameGraphCommand::NextKey),
            "copy" => self.core.key(FrameGraphKey::Copy),
            "paste" => self.core.key(FrameGraphKey::Paste),
            "delete" => self.core.key(FrameGraphKey::Delete),
            "zoom-in" => self.core.key(FrameGraphKey::ZoomIn),
            "zoom-out" => self.core.key(FrameGraphKey::ZoomOut),
            "menu-dismiss" => {
                self.menu = None;
                return Ok(());
            }
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
            _ => return Err("Unknown frame graph action".into()),
        };
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
            return Err("Inspector selection changed".into());
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
        if result.actions.is_empty() {
            Ok(())
        } else {
            self.apply(controller, result, true).map(|_| ())
        }
    }
}
