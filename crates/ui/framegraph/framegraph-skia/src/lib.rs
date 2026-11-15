use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use shrimply_components_skia::canvas::TimelinePainter;
pub use shrimply_keyframe_graph_skia::{
    FrameGraphAction, FrameGraphComponentAction, FrameGraphComponents, FrameGraphKey,
    FrameGraphModifiers, FrameGraphPointerButton, FrameGraphPointerPosition, FrameGraphScrollInput,
    FrameGraphState, FrameGraphStatus,
};
use shrimply_keyframe_graph_skia::{GraphDomain, KeyframeGraph};
use shrimply_math_core::Time;
use shrimply_math_interpolation::Interpolation;
use uuid::Uuid;

/// Toolkit-independent frame-graph controller. Native bridges translate events and draw the
/// returned state; they cannot reach into selection, hit-test, drag, or view state.
#[derive(Clone)]
pub struct FrameGraphCore {
    components: Rc<RefCell<FrameGraphComponents>>,
    touch: Rc<RefCell<shrimply_components_skia::input::TouchInput>>,
    touch_playhead: Rc<Cell<Option<Time>>>,
}

pub type SharedFrameGraphState = FrameGraphCore;

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

#[derive(Clone, Copy)]
pub enum FrameGraphCommand {
    PreviousKey,
    ToggleKey,
    NextKey,
}

impl FrameGraphCore {
    pub fn new(components: FrameGraphComponents) -> Self {
        Self {
            components: Rc::new(RefCell::new(components)),
            touch: Rc::new(RefCell::new(Default::default())),
            touch_playhead: Rc::new(Cell::new(None)),
        }
    }

    pub fn single(state: FrameGraphState) -> Self {
        Self::new(FrameGraphComponents::single(state))
    }

    pub fn active_component(&self) -> usize {
        self.components.borrow().active_component()
    }

    pub fn preferred_height(&self) -> i32 {
        self.components.borrow().preferred_height()
    }

    pub fn status(&self) -> FrameGraphStatus {
        self.components.borrow().status()
    }

    pub fn is_animating(&self) -> bool {
        self.components.borrow().is_animating()
    }

    pub fn draw(&self, painter: &TimelinePainter, width: f64, height: f64) {
        self.components.borrow_mut().draw(painter, width, height);
    }

    pub fn edit_value(&self, value: f64) -> Vec<FrameGraphComponentAction> {
        self.components
            .borrow_mut()
            .active_actions(|state| state.set_value(value))
    }

    pub fn edit_component_values(
        &self,
        active_component: usize,
        values: &[(usize, f64)],
    ) -> Vec<FrameGraphComponentAction> {
        self.components
            .borrow_mut()
            .set_component_values(active_component, values)
    }

    pub fn activate_component(&self, component: usize) {
        self.components.borrow_mut().activate(component);
    }

    pub fn replace_components(&self, components: FrameGraphComponents) {
        *self.components.borrow_mut() = components;
    }

    pub fn replace_active_graph(&self, graph: KeyframeGraph) {
        let component = self.active_component();
        self.components
            .borrow_mut()
            .replace_component_graph(component, graph);
    }

    pub fn set_view(
        &self,
        item_range: GraphDomain,
        frame_step: Time,
        playhead: Time,
        snapping: (bool, f64),
        external_clipboard: bool,
        text_interpolation: bool,
    ) {
        let mut state = self.components.borrow_mut();
        state.set_item_range(item_range);
        state.set_frame_step(frame_step);
        state.set_playhead(playhead);
        state.set_snapping(snapping.0, snapping.1);
        state.set_external_clipboard(external_clipboard);
        state.set_text_interpolation(text_interpolation);
    }

    pub fn set_playhead(&self, playhead: Time) {
        self.components.borrow_mut().set_playhead(playhead);
    }

    pub fn set_interpolation(&self, owner_id: Uuid, interpolation: Interpolation) {
        self.components
            .borrow_mut()
            .set_interpolation(owner_id, interpolation);
    }

    pub fn command(&self, command: FrameGraphCommand) -> FrameGraphInputResult {
        let actions = self
            .components
            .borrow_mut()
            .active_actions(|state| match command {
                FrameGraphCommand::PreviousKey => state.previous_key(),
                FrameGraphCommand::ToggleKey => state.toggle_key(),
                FrameGraphCommand::NextKey => state.next_key(),
            });
        FrameGraphInputResult {
            actions,
            focus: true,
            handled: true,
            redraw: true,
            ..Default::default()
        }
    }

    pub fn pointer_moved(&self, x: f64, y: f64) -> FrameGraphInputResult {
        self.components.borrow_mut().pointer_moved(x, y);
        FrameGraphInputResult {
            redraw: true,
            ..Default::default()
        }
    }

    pub fn pointer_left(&self) -> FrameGraphInputResult {
        self.components.borrow_mut().pointer_left();
        FrameGraphInputResult {
            redraw: true,
            ..Default::default()
        }
    }

    pub fn begin_pointer(
        &self,
        button: FrameGraphPointerButton,
        position: FrameGraphPointerPosition,
        modifiers: FrameGraphModifiers,
    ) -> FrameGraphInputResult {
        let actions = self.components.borrow_mut().active_actions(|state| {
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

    pub fn update_pointer(&self, position: FrameGraphPointerPosition) -> FrameGraphInputResult {
        let mut actions = self.components.borrow_mut().active_actions(|state| {
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

    pub fn end_pointer(&self) -> FrameGraphInputResult {
        FrameGraphInputResult {
            actions: self
                .components
                .borrow_mut()
                .active_actions(FrameGraphState::end_pointer),
            handled: true,
            redraw: true,
            ..Default::default()
        }
    }

    pub fn scroll(
        &self,
        dx: f64,
        dy: f64,
        position: FrameGraphPointerPosition,
        zoom: bool,
        input: FrameGraphScrollInput,
    ) -> FrameGraphInputResult {
        let handled = self
            .components
            .borrow_mut()
            .scroll(dx, dy, position, zoom, input);
        FrameGraphInputResult {
            handled,
            redraw: handled,
            ..Default::default()
        }
    }

    pub fn magnify(
        &self,
        magnification: f64,
        position: FrameGraphPointerPosition,
    ) -> FrameGraphInputResult {
        self.components
            .borrow_mut()
            .magnify(magnification, position.x, position.width);
        FrameGraphInputResult {
            handled: true,
            redraw: true,
            ..Default::default()
        }
    }

    pub fn key(&self, key: FrameGraphKey) -> FrameGraphInputResult {
        FrameGraphInputResult {
            actions: self
                .components
                .borrow_mut()
                .active_actions(|state| state.key(key)),
            handled: true,
            redraw: true,
            ..Default::default()
        }
    }

    /// Native hosts forward logical contacts; gesture recognition stays shared.
    pub fn touch(
        &self,
        changes: &[shrimply_components_skia::input::TouchChange],
        position: FrameGraphPointerPosition,
    ) -> FrameGraphInputResult {
        use shrimply_components_skia::input::TouchAction;
        if !self.touch_active()
            && changes
                .iter()
                .any(|change| change.phase == shrimply_components_skia::input::TouchPhase::Begin)
        {
            self.touch_playhead
                .set(Some(self.components.borrow().playhead()));
        }
        let gestures = self.touch.borrow_mut().update(changes);
        let mut result = FrameGraphInputResult::default();
        for gesture in gestures {
            let update = match gesture {
                TouchAction::Press(point) => self.begin_pointer(
                    FrameGraphPointerButton::Primary,
                    FrameGraphPointerPosition {
                        x: f64::from(point.x),
                        y: f64::from(point.y),
                        ..position
                    },
                    FrameGraphModifiers::default(),
                ),
                TouchAction::Move(point) => self.update_pointer(FrameGraphPointerPosition {
                    x: f64::from(point.x),
                    y: f64::from(point.y),
                    ..position
                }),
                TouchAction::Release(_) => self.end_pointer(),
                TouchAction::Cancel(_) => self.cancel_pointer(),
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
                    let mut update = self.scroll(
                        f64::from(previous_center.x - center.x),
                        0.0,
                        anchor,
                        false,
                        FrameGraphScrollInput::Surface,
                    );
                    let zoom = self.magnify(f64::from(scale).ln(), anchor);
                    update.redraw |= zoom.redraw;
                    update.handled |= zoom.handled;
                    update
                }
                TouchAction::History { redo } => {
                    result.history = Some(redo);
                    self.end_pointer()
                }
            };
            result.actions.extend(update.actions);
            result.redraw |= update.redraw;
            result.handled |= update.handled;
            result.focus |= update.focus;
            result.cancelled |= update.cancelled;
        }
        if !self.touch_active() {
            self.touch_playhead.set(None);
        }
        result
    }

    pub fn touch_active(&self) -> bool {
        self.touch.borrow().active()
    }

    pub fn waiting_long_press(&self) -> bool {
        self.touch.borrow().waiting_long_press()
    }

    pub fn poll_long_press(&self, position: FrameGraphPointerPosition) -> FrameGraphInputResult {
        let point = self.touch.borrow_mut().poll_long_press();
        point.map_or_else(FrameGraphInputResult::default, |point| {
            let mut result = self.begin_pointer(
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
                    FrameGraphAction::InterpolationRequested { touch, .. }
                    | FrameGraphAction::TextInterpolationRequested { touch, .. } => *touch = true,
                    _ => {}
                }
            }
            result
        })
    }

    pub fn cancel_touch(&self) -> FrameGraphInputResult {
        if self.touch.borrow_mut().cancel().is_empty() {
            self.end_pointer()
        } else {
            self.cancel_pointer()
        }
    }

    fn cancel_pointer(&self) -> FrameGraphInputResult {
        self.end_pointer();
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
