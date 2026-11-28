use super::Runtime;
use serde_json::{Value, json};
use shrimply_components_skia::Vec2;
use shrimply_timeline_skia::{
    ContextMenu, ContextMenuAction, ContextMenuControl, ContextMenuEntry, ContextMenuRequest,
    scene::{KeyAction, TrackDeletion},
};

pub(super) struct Menu {
    serial: u64,
    model: ContextMenu,
    point: Vec2,
    touch: bool,
    deletion: Option<TrackDeletion>,
    delete_count: Option<usize>,
    clipboard: Option<ContextMenuRequest>,
    keys: Vec<KeyAction>,
}

impl Menu {
    pub fn snapshot(&self) -> Value {
        let sections: Vec<_> = self.model.sections.iter().map(|section| section.iter().enumerate().map(|(index, entry)| match entry {
            ContextMenuEntry::Action(item) => json!({"action":item.key(),"label":self.model.label(*item),"enabled":item.enabled,"checked":item.checked}),
            ContextMenuEntry::Control(control) => json!({"control":index,"label":control.label(),"value":control.value(),"minimum":control.minimum(),"maximum":control.maximum(),"step":control.step(),"mixed":control.mixed()}),
        }).collect::<Vec<_>>()).collect();
        let clipboard = self.clipboard.map(|request| {
            json!({
                "write":matches!(request, ContextMenuRequest::SetTimelineClipboardMarker),
                "marker":shrimply_timeline_skia::TIMELINE_CLIPBOARD_MARKER,
                "command":{"op":"context_clipboard","serial":self.serial}
            })
        });
        json!({"clipboard":clipboard,"serial":self.serial,"bottom_sheet":shrimply_components_core::layout::menu_uses_bottom_sheet(if self.touch { "touch" } else { "pointer" }),"x":self.point.x,"y":self.point.y,"sections":sections,"delete_count":self.delete_count})
    }
}

impl Runtime {
    pub fn show_context_menu(&mut self, point: Vec2, touch: bool) {
        self.preview_context = None;
        let Some(scene) = self.timeline.as_mut() else {
            return;
        };
        let model = scene.prepare_context_menu_for_input(point, touch);
        if model.sections.is_empty() {
            self.context_menu = None;
        } else {
            self.open_context(model, point, touch);
        }
    }

    fn open_context(&mut self, model: ContextMenu, point: Vec2, touch: bool) {
        self.preview_context = None;
        self.context_serial = self
            .context_serial
            .checked_add(1)
            .expect("context serial overflow");
        self.context_menu = Some(Menu {
            serial: self.context_serial,
            model,
            point,
            touch,
            deletion: None,
            delete_count: None,
            clipboard: None,
            keys: Vec::new(),
        });
        self.inspector_stamp = None;
    }

    pub fn timeline_key_action(&mut self, action: KeyAction) -> Result<(), String> {
        if let Some(menu) = &mut self.context_menu
            && menu.clipboard.is_some()
        {
            // Preserve rapid shortcut order until Android acknowledges the clipboard.
            menu.keys.push(action);
            return Ok(());
        }
        self.context_menu = None;
        let request = self
            .timeline
            .as_mut()
            .ok_or("Open a project first")?
            .key_action(action)?;
        if let Some(request) = request {
            self.open_context(ContextMenu::default(), Vec2::ZERO, false);
            let mut response = json!({});
            if !self.dispatch_context_request(request, &mut response)? {
                self.context_menu = None;
            }
            assert_eq!(
                response,
                json!({}),
                "keyboard action requires native delivery"
            );
        }
        self.timeline_changed()
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
        let mut pending = None;
        let mut keys = Vec::new();
        match request["op"].as_str().expect("request operation") {
            "context_dismiss" => {}
            "context_clipboard" => {
                let clipboard = menu
                    .clipboard
                    .take()
                    .ok_or("No clipboard request is pending")?;
                keys = std::mem::take(&mut menu.keys);
                self.context_menu = None;
                if matches!(clipboard, ContextMenuRequest::PasteFromClipboard) {
                    if request["clipboard_marker"] != true {
                        return Err("The clipboard does not contain timeline items".into());
                    }
                    scene.paste_context_clipboard()?;
                }
            }
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
                    .find(|item| request["action"].as_str() == Some(item.key().as_ref()))
                    .ok_or("Context action is unavailable")?;
                pending = scene.activate_context_menu_action(item.action)?;
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
        if let Some(pending) = pending
            && self.dispatch_context_request(pending, &mut result)?
        {
            self.timeline_changed()?;
            return Ok(result);
        }
        self.context_menu = None;
        self.timeline_changed()?;
        for action in keys {
            self.timeline_key_action(action)?;
        }
        Ok(result)
    }

    fn dispatch_context_request(
        &mut self,
        request: ContextMenuRequest,
        result: &mut Value,
    ) -> Result<bool, String> {
        let scene = self.timeline.as_ref().ok_or("Open a project first")?;
        match request {
            ContextMenuRequest::DeleteTracks { clip_count }
            | ContextMenuRequest::DeleteFoldedTrack { clip_count } => {
                let menu = self.context_menu.as_mut().expect("open context request");
                menu.deletion = Some(scene.track_deletion());
                menu.delete_count = Some(clip_count);
                return Ok(true);
            }
            ContextMenuRequest::SetTimelineClipboardMarker
            | ContextMenuRequest::PasteFromClipboard => {
                self.context_menu
                    .as_mut()
                    .expect("open context request")
                    .clipboard = Some(request);
                return Ok(true);
            }
            ContextMenuRequest::CopyFrame(selection) | ContextMenuRequest::SaveFrame(selection) => {
                #[cfg(target_os = "android")]
                self.capture_selected_frame(
                    selection,
                    if matches!(request, ContextMenuRequest::CopyFrame(_)) {
                        shrimply_preview_render_core::playback::toolbar::ContextRequest::CopyImage
                    } else {
                        shrimply_preview_render_core::playback::toolbar::ContextRequest::SaveImage
                    },
                )?;
                #[cfg(not(target_os = "android"))]
                {
                    let _ = selection;
                    return Err("Image capture requires the native GPU host".into());
                }
            }
            ContextMenuRequest::ShowInFolder => {
                result["show_in_files"] = json!(
                    scene
                        .context_file_path()
                        .ok_or("The selected item has no file location")?
                );
            }
            ContextMenuRequest::GenerateSpeech
            | ContextMenuRequest::Transcribe
            | ContextMenuRequest::RemoveSilences
            | ContextMenuRequest::ExportAudio => self.open_timeline_service(request)?,
        }
        Ok(false)
    }
}
