//! Native preview events bind shared playback presentation and guide interaction.
use super::Runtime;
use serde_json::{Value, json};
use shrimply_player_core as player;
use shrimply_preview_interaction_skia::surface::{
    Clock, Configuration, PointerEvent, PointerInput, PointerSample, PreviewResponse,
};

impl Runtime {
    pub fn preview_render_update(
        &mut self,
        loading: Option<bool>,
        elapsed: Option<std::time::Duration>,
    ) -> bool {
        self.preview_toolbar.update(loading, elapsed)
    }

    pub(super) fn preview_controls(&self) -> Value {
        let transport = player::snapshot(&self.player);
        let preferences = shrimply_preferences_core::snapshot(&self.backend.preferences);
        let model = self.preview_toolbar.presentation(transport.playback_speed);
        json!({
            "visible": preferences.preview_controls_visible,
            "loading": model.loading,
            "status": model.status,
            "status_icon": "check-plain-symbolic",
            "frame_rate": model.frame_rate,
            "frame_rate_label": model.frame_rate_tooltip,
            "speed": model.speed,
            "speed_label": model.speed_tooltip,
            "close_label": shrimply_i18n::text("Close"),
            "guide": {"label":model.guides,"icon":"ruler-angled-symbolic",
                "active":preferences.preview_guides_visible,"enabled":self.backend.project().is_some(),
                "command":{"op":"preview_guides","visible":!preferences.preview_guides_visible}}
        })
    }

    pub(super) fn preview_request(&mut self, request: &Value) -> Result<Value, String> {
        let visible = request["visible"]
            .as_bool()
            .ok_or("guide visibility requires a boolean")?;
        if !visible {
            self.cancel_preview_edit()?;
        }
        shrimply_preferences_core::set_preview_guides_visible(&self.backend.preferences, visible);
        Ok(self.preview_controls())
    }

    /// Native coordinates are logical pixels; all controls and gestures are shared.
    pub fn preview_native_pointer(
        &mut self,
        phase: u8,
        position: glam::Vec2,
        size: glam::Vec2,
        flags: u32,
    ) -> Result<bool, String> {
        if phase == 3 {
            return self.cancel_preview_edit();
        }
        let input = native_pointer(position, flags);
        let event = match phase {
            0 => PointerEvent::Begin(input),
            1 => PointerEvent::Samples {
                input,
                samples: &[],
            },
            2 => PointerEvent::End(input),
            4 => PointerEvent::Hover(input),
            5 => PointerEvent::Leave,
            _ => return Err("Invalid native preview pointer phase".into()),
        };
        self.dispatch_preview(size, Event::Pointer(event))
            .map(|(committed, _)| committed)
    }

    pub fn preview_pen(
        &mut self,
        phase: u8,
        flags: u32,
        size: glam::Vec2,
        samples: &[PointerSample],
    ) -> Result<bool, String> {
        if phase == 3 {
            return self.cancel_preview_edit();
        }
        let sample = samples.last().ok_or("Pen event requires samples")?;
        let input = PointerInput {
            sample: *sample,
            ..native_pointer(sample.position, flags)
        };
        let event = match phase {
            0 => PointerEvent::Begin(input),
            1 => PointerEvent::Samples { input, samples },
            2 => PointerEvent::End(input),
            4 => PointerEvent::Hover(input),
            5 => PointerEvent::Leave,
            _ => return Err("Invalid native pen phase".into()),
        };
        self.dispatch_preview(size, Event::Pointer(event))
            .map(|(committed, _)| committed)
    }

    pub fn preview_keyboard(
        &mut self,
        event: shrimply_preview_interaction_skia::surface::KeyboardEvent,
    ) -> Result<bool, String> {
        self.dispatch_preview(self.preview_size, Event::Keyboard(event))
            .map(|(_, handled)| handled)
    }

    pub fn preview_scroll(
        &mut self,
        position: glam::Vec2,
        delta: glam::Vec2,
        flags: u32,
    ) -> Result<bool, String> {
        let configuration = self.preview_configuration(self.preview_size);
        let delta = shrimply_preview_interaction_skia::surface::preview_scroll_delta(
            delta,
            flags & 64 != 0,
            configuration.navigation_enabled,
        );
        self.dispatch_preview(
            self.preview_size,
            Event::Pointer(PointerEvent::Scroll {
                input: native_pointer(position, flags),
                delta,
            }),
        )
        .map(|(committed, _)| committed)
    }

    fn preview_configuration(&self, size: glam::Vec2) -> Configuration {
        let preferences = shrimply_preferences_core::snapshot(&self.backend.preferences);
        Configuration {
            size,
            padding: preferences.preview_padding_px,
            guides_visible: preferences.preview_guides_visible,
            fullscreen: false,
            snap_enabled: preferences.timeline_magnet == "true",
            snap_radius: preferences.timeline_snap_radius_px as f32,
            navigation_enabled: preferences.preview_zoom_pan_enabled,
        }
    }

    fn dispatch_preview(
        &mut self,
        size: glam::Vec2,
        event: Event<'_>,
    ) -> Result<(bool, bool), String> {
        let Some(project) = self.backend.shared_project() else {
            return Ok((false, false));
        };
        let selected = self
            .timeline
            .as_ref()
            .and_then(|scene| scene.focused_video_address());
        let player = player::snapshot(&self.player);
        let clock = Clock {
            revision: player.revision,
            position: player.position,
            moving: player.playing || player.scrubbing,
        };
        let configuration = self.preview_configuration(size);
        self.preview_size = size;
        let output = {
            let mut project = project.borrow_mut();
            match event {
                Event::Pointer(event) => {
                    shrimply_preview_interaction_skia::surface::TouchResponse {
                        response: self.preview_interaction.pointer(
                            &mut project,
                            selected.as_ref(),
                            clock,
                            configuration,
                            event,
                        )?,
                        ..Default::default()
                    }
                }
                Event::Keyboard(event) => {
                    shrimply_preview_interaction_skia::surface::TouchResponse {
                        response: self.preview_interaction.keyboard(
                            &mut project,
                            selected.as_ref(),
                            clock,
                            configuration,
                            event,
                        )?,
                        ..Default::default()
                    }
                }
                Event::Touch(changes) => self.preview_interaction.touch(
                    &mut project,
                    selected.as_ref(),
                    clock,
                    configuration,
                    changes,
                )?,
                Event::Prepare => shrimply_preview_interaction_skia::surface::TouchResponse {
                    response: self.preview_interaction.prepare_overlay(
                        &mut project,
                        selected.as_ref(),
                        clock,
                        configuration,
                    )?,
                    ..Default::default()
                },
            }
        };
        let changed = self.apply_preview_response(output.response)?;
        self.schedule_preview_base()?;
        if let Some(redo) = output.history {
            self.request(&json!({"op":if redo {"redo"} else {"undo"}}))?;
            return Ok((true, true));
        }
        Ok((changed, output.response.handled))
    }

    fn apply_preview_response(&mut self, response: PreviewResponse) -> Result<bool, String> {
        use shrimply_preview_interaction_skia::surface::PreviewRefresh;
        let committed = self.preview_interaction.take_committed();
        if response.edit.refresh != PreviewRefresh::NONE {
            player::refresh_project(
                &self.player,
                player::ProjectChange {
                    video: response.edit.refresh.contains(PreviewRefresh::PREVIEW),
                    live_preview: response.edit.is_live(),
                    inspector: response.edit.refresh.contains(PreviewRefresh::INSPECTOR),
                    ..Default::default()
                },
            );
        }
        if response.edit.commits() {
            self.preview_interaction
                .project_committed(player::snapshot(&self.player).revision);
        }
        if committed {
            self.inspector_stamp = None;
            if response.edit.refresh == PreviewRefresh::NONE {
                self.timeline_changed()?;
            }
        }
        Ok(committed)
    }

    fn schedule_preview_base(&mut self) -> Result<(), String> {
        #[cfg(target_os = "android")]
        if let Some(media) = &self.media {
            use shrimply_preview_render_core::{CompositeAccuracy, worker::VideoCommand};
            if let Some(exclusion) = self.preview_interaction.exclusion_update() {
                let sender = media.sender();
                sender
                    .send(VideoCommand::SetPreviewExclusion(exclusion))
                    .map_err(|error| error.to_string())?;
                sender
                    .send(VideoCommand::Render {
                        position: player::current_time(&self.player),
                        accuracy: CompositeAccuracy::FULLY_ACCURATE,
                    })
                    .map_err(|error| error.to_string())?;
            }
            if self.preview_interaction.take_live_base_request() {
                player::refresh_project(
                    &self.player,
                    player::ProjectChange {
                        video: true,
                        live_preview: true,
                        ..Default::default()
                    },
                );
                self.preview_interaction
                    .live_base_requested(player::snapshot(&self.player).revision);
            }
        }
        Ok(())
    }

    pub fn preview_presented<F>(
        &mut self,
        event: &shrimply_preview_render_core::worker::VideoEvent<F>,
    ) -> bool {
        use shrimply_preview_render_core::worker::VideoEvent;
        match event {
            VideoEvent::Frame {
                revision,
                position,
                excluded_item_id,
                audio_analysis,
                ..
            }
            | VideoEvent::Clear {
                revision,
                position,
                excluded_item_id,
                audio_analysis,
                ..
            } => self.preview_interaction.presented(
                *revision,
                *position,
                *excluded_item_id,
                audio_analysis,
            ),
            _ => false,
        }
    }

    pub fn preview_position(&self) -> shrimply_math_core::Time {
        let player = player::snapshot(&self.player);
        self.preview_interaction.displayed_position(Clock {
            revision: player.revision,
            position: player.position,
            moving: player.playing || player.scrubbing,
        })
    }

    pub fn prepare_preview_overlay(&mut self, size: glam::Vec2) -> Result<(), String> {
        self.dispatch_preview(size, Event::Prepare).map(|_| ())
    }

    pub fn cancel_preview_edit(&mut self) -> Result<bool, String> {
        let Some(project) = self.backend.shared_project() else {
            return Ok(false);
        };
        let response = self.preview_interaction.cancel(&mut project.borrow_mut());
        self.apply_preview_response(response)
    }
}

enum Event<'a> {
    Pointer(PointerEvent<'a>),
    Keyboard(shrimply_preview_interaction_skia::surface::KeyboardEvent),
    Touch(&'a [shrimply_components_skia::input::TouchChange]),
    Prepare,
}

pub(super) struct Menu {
    serial: u64,
    point: glam::Vec2,
    touch: bool,
}

impl Runtime {
    pub(super) fn preview_context_snapshot(&self) -> Value {
        let Some(menu) = &self.preview_context else {
            return Value::Null;
        };
        let items = shrimply_preview_render_core::playback::toolbar::context_menu(&self.backend.preferences,cfg!(target_os = "android")).into_iter().map(|item| {
            #[cfg(target_os = "android")]
            let enabled = item.checked.is_some() || self.preview_image_available();
            #[cfg(not(target_os = "android"))]
            let enabled = true;
            json!({"label":item.label,"checked":item.checked,"enabled":enabled,"command":{"op":"preview_context_action","project_path":self.backend.path(),"serial":menu.serial,"action":item.action.id()}})
        }).collect::<Vec<_>>();
        json!({"serial":menu.serial,"bottom_sheet":shrimply_components_core::layout::menu_uses_bottom_sheet(if menu.touch {"touch"} else {"pointer"}),"x":menu.point.x,"y":menu.point.y,"items":items,"dismiss":{"op":"preview_context_dismiss","project_path":self.backend.path(),"serial":menu.serial}})
    }

    pub fn show_preview_context(&mut self, point: glam::Vec2, touch: bool) {
        if let Err(error) = self.cancel_preview_edit() {
            self.error = Some(error);
            return;
        }
        self.context_menu = None;
        self.context_serial = self
            .context_serial
            .checked_add(1)
            .expect("preview context serial overflow");
        self.preview_context = Some(Menu {
            serial: self.context_serial,
            point,
            touch,
        });
    }

    pub(super) fn preview_context_request(&mut self, request: &Value) -> Result<Value, String> {
        let menu = self
            .preview_context
            .as_ref()
            .ok_or("Preview menu is closed")?;
        if request["serial"].as_u64() != Some(menu.serial) {
            return Err("Preview menu changed; open it again".into());
        }
        if request["op"] == "preview_context_action" {
            let action = shrimply_preview_render_core::playback::toolbar::context_menu(
                &self.backend.preferences,
                cfg!(target_os = "android"),
            )
            .into_iter()
            .find(|item| request["action"] == item.action.id())
            .ok_or("Preview action is unavailable")?
            .action;
            if let Some(_capture) = action.activate(&self.backend.preferences) {
                #[cfg(target_os = "android")]
                self.capture_preview_image(_capture)?;
                #[cfg(not(target_os = "android"))]
                return Err("Preview image delivery requires the native host".into());
            }
        }
        self.preview_context = None;
        Ok(self.preview_controls())
    }

    pub fn preview_touch(
        &mut self,
        changes: &[shrimply_components_skia::input::TouchChange],
        size: glam::Vec2,
    ) -> Result<bool, String> {
        self.dispatch_preview(size, Event::Touch(changes))
            .map(|(committed, _)| committed)
    }

    pub fn poll_preview_input(&mut self) -> bool {
        if !self.foreground {
            return false;
        }
        if let Some(point) = self.preview_interaction.poll_context() {
            self.show_preview_context(point, true);
            return true;
        }
        false
    }
}

fn native_pointer(position: glam::Vec2, flags: u32) -> PointerInput {
    use shrimply_preview_interaction_skia::surface::{PointerButton, PointerTool};
    PointerInput {
        sample: PointerSample {
            position,
            ..Default::default()
        },
        tool: if flags & 128 != 0 {
            PointerTool::Eraser
        } else if flags & 1 != 0 {
            PointerTool::Pen
        } else {
            PointerTool::Mouse
        },
        button: if flags & 2 != 0 {
            PointerButton::Middle
        } else {
            PointerButton::Primary
        },
        modifiers: crate::input::preview_modifiers(flags),
    }
}
