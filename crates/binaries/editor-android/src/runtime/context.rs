use super::Runtime;
use serde_json::{Value, json};
use shrimply_components_skia::Vec2;
use shrimply_timeline_skia::{
    ContextMenu, ContextMenuAction, ContextMenuControl, ContextMenuEntry, ContextMenuRequest,
    scene::TrackDeletion,
};

pub(super) struct Menu {
    serial: u64,
    model: ContextMenu,
    point: Vec2,
    touch: bool,
    deletion: Option<TrackDeletion>,
    delete_count: Option<usize>,
}

impl Menu {
    pub fn snapshot(&self) -> Value {
        let sections: Vec<_> = self.model.sections.iter().map(|section| section.iter().enumerate().map(|(index, entry)| match entry {
            ContextMenuEntry::Action(item) => json!({"action":item.action.id(),"label":item.label(),"enabled":item.enabled}),
            ContextMenuEntry::Control(control) => json!({"control":index,"label":control.label(),"value":control.value(),"minimum":control.minimum(),"maximum":control.maximum(),"step":control.step(),"mixed":control.mixed()}),
        }).collect::<Vec<_>>()).collect();
        json!({"serial":self.serial,"touch":self.touch,"x":self.point.x,"y":self.point.y,"sections":sections,"delete_count":self.delete_count,"clipboard_marker":shrimply_timeline_skia::TIMELINE_CLIPBOARD_MARKER})
    }
}

impl Runtime {
    pub fn show_context_menu(&mut self, point: Vec2, touch: bool) {
        let Some(scene) = self.timeline.as_mut() else {
            return;
        };
        let model = scene.prepare_context_menu(point);
        self.context_serial = self
            .context_serial
            .checked_add(1)
            .expect("context serial overflow");
        self.context_menu = (!model.sections.is_empty()).then_some(Menu {
            serial: self.context_serial,
            model,
            point,
            touch,
            deletion: None,
            delete_count: None,
        });
        self.inspector_stamp = None;
    }

    pub(super) fn context_request(&mut self, request: &Value) -> Result<Value, String> {
        let menu = self
            .context_menu
            .as_mut()
            .ok_or("The timeline menu is closed")?;
        if request["serial"].as_u64() != Some(menu.serial) {
            return Err("The timeline menu changed; open it again".into());
        }
        let scene = self.timeline.as_mut().ok_or("Open a project first")?;
        let mut result = json!({});
        match request["op"].as_str().expect("request operation") {
            "context_dismiss" => {}
            "context_delete" => {
                scene.confirm_delete_selected_tracks(
                    menu.deletion
                        .take()
                        .ok_or("No track deletion awaits confirmation")?,
                )?;
            }
            "context_control" => {
                let section = request["section"]
                    .as_u64()
                    .ok_or("Context section is required")? as usize;
                let index = request["control"]
                    .as_u64()
                    .ok_or("Context control is required")? as usize;
                let Some(ContextMenuEntry::Control(control)) = menu
                    .model
                    .sections
                    .get(section)
                    .and_then(|section| section.get(index))
                else {
                    return Err("Context control is unavailable".into());
                };
                let value = request["value"]
                    .as_f64()
                    .filter(|value| value.is_finite())
                    .ok_or("Invalid context control value")?;
                if value < control.minimum() || value > control.maximum() {
                    return Err("Context control is outside its range".into());
                }
                let next = match control {
                    ContextMenuControl::PlaybackSpeed { .. } => ContextMenuControl::PlaybackSpeed {
                        position: value,
                        mixed: false,
                    },
                    ContextMenuControl::AudioTrackGain { .. } => {
                        ContextMenuControl::AudioTrackGain { db: value as f32 }
                    }
                };
                scene.set_context_menu_control(next, value)?;
                menu.model.sections[section][index] = ContextMenuEntry::Control(next);
                self.timeline_changed()?;
                return Ok(result);
            }
            "context_action" => {
                let item = menu
                    .model
                    .actions()
                    .find(|item| request["action"] == item.action.id())
                    .ok_or("Context action is unavailable")?;
                let action = scene.activate_context_menu_action(item.action)?;
                match action {
                    Some(ContextMenuRequest::DeleteTracks {clip_count} | ContextMenuRequest::DeleteFoldedTrack {clip_count}) => {
                        menu.deletion = Some(scene.track_deletion());
                        menu.delete_count = Some(clip_count);
                        return Ok(result);
                    }
                    Some(ContextMenuRequest::SetTimelineClipboardMarker) => result["clipboard"] = json!(shrimply_timeline_skia::TIMELINE_CLIPBOARD_MARKER),
                    Some(ContextMenuRequest::PasteFromClipboard) => {
                        if request["clipboard_marker"] != true { return Err("The clipboard does not contain timeline items".into()); }
                        scene.paste_context_clipboard()?;
                    }
                    Some(_) => return Err("This timeline action requires a native service that is still being integrated".into()),
                    None => {},
                }
                if item.action == ContextMenuAction::InspectProperties {
                    self.inspector_visible = true;
                    self.inspect_serial = self
                        .inspect_serial
                        .checked_add(1)
                        .expect("inspector serial overflow");
                    self.inspector_stamp = None;
                }
            }
            _ => unreachable!("context operation dispatcher"),
        }
        self.context_menu = None;
        self.timeline_changed()?;
        Ok(result)
    }
}
