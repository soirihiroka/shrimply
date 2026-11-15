use std::{cell::RefCell, rc::Rc};

use shrimply_components_skia::canvas::TimelinePainter;
pub use shrimply_keyframe_graph_skia::{
    FrameGraphAction, FrameGraphCommand, FrameGraphComponentAction, FrameGraphComponents,
    FrameGraphInputResult, FrameGraphKey, FrameGraphModifiers, FrameGraphPointerButton,
    FrameGraphPointerPosition, FrameGraphScrollInput, FrameGraphState, FrameGraphStatus,
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
}

pub type SharedFrameGraphState = FrameGraphCore;

impl FrameGraphCore {
    pub fn new(components: FrameGraphComponents) -> Self {
        Self {
            components: Rc::new(RefCell::new(components)),
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
        self.components.borrow_mut().replace_components(components);
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
        self.components
            .borrow_mut()
            .input_begin_pointer(button, position, modifiers)
    }
    pub fn update_pointer(&self, position: FrameGraphPointerPosition) -> FrameGraphInputResult {
        self.components.borrow_mut().input_update_pointer(position)
    }
    pub fn end_pointer(&self) -> FrameGraphInputResult {
        self.components.borrow_mut().input_end_pointer()
    }
    pub fn scroll(
        &self,
        dx: f64,
        dy: f64,
        position: FrameGraphPointerPosition,
        zoom: bool,
        input: FrameGraphScrollInput,
    ) -> FrameGraphInputResult {
        self.components
            .borrow_mut()
            .input_scroll(dx, dy, position, zoom, input)
    }
    pub fn magnify(
        &self,
        magnification: f64,
        position: FrameGraphPointerPosition,
    ) -> FrameGraphInputResult {
        self.components
            .borrow_mut()
            .input_magnify(magnification, position)
    }
    pub fn key(&self, key: FrameGraphKey) -> FrameGraphInputResult {
        self.components.borrow_mut().input_key(key)
    }
    pub fn touch(
        &self,
        changes: &[shrimply_components_skia::input::TouchChange],
        position: FrameGraphPointerPosition,
    ) -> FrameGraphInputResult {
        self.components.borrow_mut().input_touch(changes, position)
    }
    pub fn touch_active(&self) -> bool {
        self.components.borrow().input_touch_active()
    }
    pub fn waiting_long_press(&self) -> bool {
        self.components.borrow().input_waiting_long_press()
    }
    pub fn poll_long_press(&self, position: FrameGraphPointerPosition) -> FrameGraphInputResult {
        self.components.borrow_mut().input_poll_long_press(position)
    }
    pub fn cancel_touch(&self) -> FrameGraphInputResult {
        self.components.borrow_mut().input_cancel_touch()
    }
}
