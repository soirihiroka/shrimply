use super::*;

#[derive(Default)]
pub struct FrameGraphInputResult {
    pub actions: Vec<FrameGraphComponentAction>,
    pub focus: bool,
    pub handled: bool,
    pub redraw: bool,
    pub history: Option<bool>,
    pub cancelled: bool,
}

impl FrameGraphInputResult {
    /// Keep native adapters from dropping gesture cancellation or history requests.
    pub fn into_actions(mut self, component: usize) -> Vec<FrameGraphComponentAction> {
        if self.cancelled {
            self.actions.insert(
                0,
                FrameGraphComponentAction {
                    component,
                    action: FrameGraphAction::EditCancelled,
                },
            );
        }
        if let Some(redo) = self.history {
            self.actions.push(FrameGraphComponentAction {
                component,
                action: FrameGraphAction::HistoryRequested { redo },
            });
        }
        self.actions
    }
}

impl FrameGraphComponents {
    pub fn input_begin_pointer(
        &mut self,
        button: FrameGraphPointerButton,
        position: FrameGraphPointerPosition,
        modifiers: FrameGraphModifiers,
    ) -> FrameGraphInputResult {
        let actions = self.active_actions(|state| {
            state.begin_pointer(
                button,
                position.x,
                position.y,
                position.width,
                position.height,
                modifiers,
            )
        });
        FrameGraphInputResult {
            actions,
            focus: true,
            handled: true,
            redraw: true,
            ..Default::default()
        }
    }

    pub fn input_update_pointer(
        &mut self,
        position: FrameGraphPointerPosition,
    ) -> FrameGraphInputResult {
        let mut actions = self.active_actions(|state| {
            state.update_pointer(position.x, position.y, position.width, position.height)
        });
        if actions
            .iter()
            .any(|action| matches!(action.action, FrameGraphAction::KeysMoved(_)))
        {
            actions.insert(
                0,
                FrameGraphComponentAction {
                    component: self.active_component(),
                    action: FrameGraphAction::EditStarted,
                },
            );
        }
        FrameGraphInputResult {
            actions,
            handled: true,
            redraw: true,
            ..Default::default()
        }
    }

    pub fn input_end_pointer(&mut self) -> FrameGraphInputResult {
        FrameGraphInputResult {
            actions: self.active_actions(FrameGraphState::end_pointer),
            handled: true,
            redraw: true,
            ..Default::default()
        }
    }

    pub fn input_scroll(
        &mut self,
        dx: f64,
        dy: f64,
        position: FrameGraphPointerPosition,
        zoom: bool,
        input: FrameGraphScrollInput,
    ) -> FrameGraphInputResult {
        let handled = self.scroll(dx, dy, position, zoom, input);
        FrameGraphInputResult {
            handled,
            redraw: handled,
            ..Default::default()
        }
    }

    pub fn input_magnify(
        &mut self,
        magnification: f64,
        position: FrameGraphPointerPosition,
    ) -> FrameGraphInputResult {
        self.magnify(magnification, position.x, position.width);
        FrameGraphInputResult {
            handled: true,
            redraw: true,
            ..Default::default()
        }
    }

    pub fn input_key(&mut self, key: FrameGraphKey) -> FrameGraphInputResult {
        FrameGraphInputResult {
            actions: self.active_actions(|state| state.key(key)),
            handled: true,
            redraw: true,
            ..Default::default()
        }
    }

    /// Native hosts forward logical contacts; gesture recognition stays shared.
    pub fn input_touch(
        &mut self,
        changes: &[shrimply_components_skia::input::TouchChange],
        position: FrameGraphPointerPosition,
    ) -> FrameGraphInputResult {
        use shrimply_components_skia::input::TouchAction;
        if !self.input_touch_active()
            && changes
                .iter()
                .any(|change| change.phase == shrimply_components_skia::input::TouchPhase::Begin)
        {
            self.touch_playhead = Some(self.playhead());
        }
        let gestures = self.touch.update(changes);
        let mut result = FrameGraphInputResult::default();
        for gesture in gestures {
            let update = match gesture {
                TouchAction::Press(point) => self.input_begin_pointer(
                    FrameGraphPointerButton::Primary,
                    FrameGraphPointerPosition {
                        x: f64::from(point.x),
                        y: f64::from(point.y),
                        ..position
                    },
                    FrameGraphModifiers::default(),
                ),
                TouchAction::Move(point) => self.input_update_pointer(FrameGraphPointerPosition {
                    x: f64::from(point.x),
                    y: f64::from(point.y),
                    ..position
                }),
                TouchAction::Release(_) => self.input_end_pointer(),
                TouchAction::Cancel(_) => self.input_cancel_pointer(),
                TouchAction::Transform {
                    previous_center,
                    center,
                    scale,
                } => {
                    let anchor = FrameGraphPointerPosition {
                        x: f64::from(center.x),
                        y: f64::from(center.y),
                        ..position
                    };
                    let mut update = self.input_scroll(
                        f64::from(previous_center.x - center.x),
                        0.0,
                        anchor,
                        false,
                        FrameGraphScrollInput::Surface,
                    );
                    let zoom = self.input_magnify(f64::from(scale).ln(), anchor);
                    update.redraw |= zoom.redraw;
                    update.handled |= zoom.handled;
                    update
                }
                TouchAction::History { redo } => {
                    result.history = Some(redo);
                    self.input_end_pointer()
                }
            };
            result.actions.extend(update.actions);
            result.redraw |= update.redraw;
            result.handled |= update.handled;
            result.focus |= update.focus;
            result.cancelled |= update.cancelled;
        }
        if !self.input_touch_active() {
            self.touch_playhead = None;
        }
        result
    }

    pub fn input_touch_active(&self) -> bool {
        self.touch.active()
    }

    pub fn input_waiting_long_press(&self) -> bool {
        self.touch.waiting_long_press()
    }

    pub fn input_poll_long_press(
        &mut self,
        position: FrameGraphPointerPosition,
    ) -> FrameGraphInputResult {
        let point = self.touch.poll_long_press();
        point.map_or_else(FrameGraphInputResult::default, |point| {
            let mut result = self.input_begin_pointer(
                FrameGraphPointerButton::Secondary,
                FrameGraphPointerPosition {
                    x: f64::from(point.x),
                    y: f64::from(point.y),
                    ..position
                },
                FrameGraphModifiers::default(),
            );
            for action in &mut result.actions {
                match &mut action.action {
                    FrameGraphAction::ContextMenuRequested { touch, .. }
                    | FrameGraphAction::InterpolationRequested { touch, .. }
                    | FrameGraphAction::TextInterpolationRequested { touch, .. } => *touch = true,
                    _ => {}
                }
            }
            result
        })
    }

    pub fn input_cancel_touch(&mut self) -> FrameGraphInputResult {
        if self.touch.cancel().is_empty() {
            self.input_end_pointer()
        } else {
            self.input_cancel_pointer()
        }
    }

    fn input_cancel_pointer(&mut self) -> FrameGraphInputResult {
        self.input_end_pointer();
        FrameGraphInputResult {
            actions: self
                .touch_playhead
                .take()
                .map(|time| FrameGraphComponentAction {
                    component: self.active_component(),
                    action: FrameGraphAction::PlayheadChanged(time),
                })
                .into_iter()
                .collect(),
            cancelled: true,
            redraw: true,
            ..Default::default()
        }
    }
}
