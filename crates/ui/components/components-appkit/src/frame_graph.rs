use crate::{
    StringChoice, action, column_append, column_stack, controls::show_searchable_popover_at,
    row_stack,
};
use objc2::rc::{Retained, Weak};
use objc2_app_kit::{NSButton, NSImage, NSLayoutConstraint, NSStackView};
use objc2_foundation::{MainThreadMarker, NSPoint, NSString};
use shrimply_framegraph_skia::FrameGraphCommand;
use shrimply_keyframe_graph_skia::{
    FrameGraphAction, FrameGraphComponentAction, FrameGraphComponents, FrameGraphState,
    FrameGraphStatus,
};
use shrimply_math_core::Time;
use shrimply_math_interpolation::Interpolation;
use std::cell::RefCell;
use std::rc::Rc;

pub use shrimply_framegraph_skia::SharedFrameGraphState;

type ActionHandler = Rc<dyn Fn(FrameGraphComponentAction)>;
type StatusHandler = Rc<dyn Fn(FrameGraphStatus)>;

#[derive(Clone)]
pub struct FrameGraph {
    root: Retained<NSStackView>,
    view: Retained<shrimply_framegraph_appkit::FrameGraphView>,
    state: SharedFrameGraphState,
    on_action: ActionHandler,
    sync: Rc<dyn Fn()>,
    status_handlers: Rc<RefCell<Vec<StatusHandler>>>,
    height: Retained<NSLayoutConstraint>,
}

impl FrameGraph {
    pub fn new(state: FrameGraphState, mtm: MainThreadMarker) -> Self {
        Self::with_actions(state, |_| {}, mtm)
    }

    pub fn with_actions(
        state: FrameGraphState,
        on_action: impl Fn(FrameGraphAction) + 'static,
        mtm: MainThreadMarker,
    ) -> Self {
        Self::with_components(
            FrameGraphComponents::single(state),
            move |action| on_action(action.action),
            mtm,
        )
    }

    pub fn with_components(
        state: FrameGraphComponents,
        on_action: impl Fn(FrameGraphComponentAction) + 'static,
        mtm: MainThreadMarker,
    ) -> Self {
        Self::with_shared_components(SharedFrameGraphState::new(state), on_action, mtm)
    }

    pub fn with_shared_components(
        state: SharedFrameGraphState,
        on_action: impl Fn(FrameGraphComponentAction) + 'static,
        mtm: MainThreadMarker,
    ) -> Self {
        let on_action: ActionHandler = Rc::new(on_action);
        let status_handlers = Rc::new(RefCell::new(Vec::<StatusHandler>::new()));
        let root = column_stack(0.0, mtm);
        let controls = row_stack(6.0, mtm);
        let spacer = objc2_app_kit::NSView::new(mtm);
        controls.addArrangedSubview(&spacer);
        let previous = graph_button("backward.end", "Previous keyframe", mtm);
        let toggle = graph_button("plus", "Add keyframe at playhead", mtm);
        let next = graph_button("forward.end", "Next keyframe", mtm);
        controls.addArrangedSubview(&previous);
        controls.addArrangedSubview(&toggle);
        controls.addArrangedSubview(&next);
        column_append(&root, &controls);

        let view_slot = Rc::new(RefCell::new(
            None::<Weak<shrimply_framegraph_appkit::FrameGraphView>>,
        ));
        let sync_slot = Rc::new(RefCell::new(None::<Rc<dyn Fn()>>));
        let action_state = state.clone();
        let action_handler = on_action.clone();
        let action_view = view_slot.clone();
        let action_sync = sync_slot.clone();
        let view = shrimply_framegraph_appkit::frame_graph_view(
            state.clone(),
            Rc::new(move |component_action| {
                if let FrameGraphAction::InterpolationRequested {
                    owner_id,
                    interpolation,
                    x,
                    y,
                } = component_action.action
                {
                    if let Some(view) = action_view.borrow().as_ref().and_then(Weak::load) {
                        show_interpolation_menu(
                            &view,
                            action_state.clone(),
                            action_handler.clone(),
                            component_action.component,
                            owner_id,
                            interpolation,
                            NSPoint::new(x, y),
                            mtm,
                        );
                    }
                } else {
                    action_handler(component_action);
                }
                if let Some(sync) = action_sync.borrow().as_ref() {
                    sync();
                }
            }),
            mtm,
        );
        view_slot.replace(Some(Weak::new(&view)));
        let height = view
            .heightAnchor()
            .constraintEqualToConstant(f64::from(state.preferred_height()));
        height.setActive(true);
        column_append(&root, &view);

        let sync = {
            let state = state.clone();
            let previous = Weak::new(&*previous);
            let toggle = Weak::new(&*toggle);
            let next = Weak::new(&*next);
            let handlers = status_handlers.clone();
            Rc::new(move || {
                let status = state.status();
                if let Some(previous) = previous.load() {
                    previous.setEnabled(status.can_previous);
                }
                if let Some(next) = next.load() {
                    next.setEnabled(status.can_next);
                }
                if let Some(toggle) = toggle.load() {
                    toggle.setImage(Some(&symbol(
                        if status.key_at_playhead {
                            "minus"
                        } else {
                            "plus"
                        },
                        if status.key_at_playhead {
                            "Delete keyframe"
                        } else {
                            "Add keyframe"
                        },
                    )));
                    toggle.setToolTip(Some(&NSString::from_str(if status.key_at_playhead {
                        "Delete keyframe at playhead"
                    } else {
                        "Add keyframe at playhead"
                    })));
                }
                for handler in handlers.borrow().iter() {
                    handler(status);
                }
            }) as Rc<dyn Fn()>
        };
        sync_slot.replace(Some(sync.clone()));

        attach_graph_button(
            &previous,
            &view,
            &state,
            &on_action,
            &sync,
            FrameGraphCommand::PreviousKey,
            mtm,
        );
        attach_graph_button(
            &toggle,
            &view,
            &state,
            &on_action,
            &sync,
            FrameGraphCommand::ToggleKey,
            mtm,
        );
        attach_graph_button(
            &next,
            &view,
            &state,
            &on_action,
            &sync,
            FrameGraphCommand::NextKey,
            mtm,
        );
        sync();
        Self {
            root,
            view,
            state,
            on_action,
            sync,
            status_handlers,
            height,
        }
    }

    /// Coordinate space used by graph pointer actions and anchored popovers.
    pub fn canvas_view(&self) -> &objc2_app_kit::NSView {
        &self.view
    }

    pub fn view(&self) -> &NSStackView {
        &self.root
    }

    pub(crate) fn retained_view(&self) -> Retained<NSStackView> {
        self.root.clone()
    }
    pub fn graph_view(&self) -> &shrimply_framegraph_appkit::FrameGraphView {
        &self.view
    }
    pub fn state(&self) -> SharedFrameGraphState {
        self.state.clone()
    }

    pub fn active_component(&self) -> usize {
        self.state.active_component()
    }

    pub fn edit_value(&self, value: f64) {
        let actions = self.state.edit_value(value);
        self.dispatch(actions);
    }

    pub fn edit_component_values(&self, active_component: usize, values: &[(usize, f64)]) {
        let actions = self.state.edit_component_values(active_component, values);
        self.dispatch(actions);
    }

    pub fn activate_component(&self, component: usize) {
        self.state.activate_component(component);
        self.refresh();
    }

    pub fn set_playhead(&self, playhead: Time) {
        self.state.set_playhead(playhead);
        self.refresh();
    }

    pub fn replace_state(&self, state: FrameGraphState) {
        self.replace_components(FrameGraphComponents::single(state));
    }

    pub fn replace_components(&self, states: FrameGraphComponents) {
        self.state.replace_components(states);
        self.refresh();
    }

    pub fn refresh(&self) {
        self.height
            .setConstant(f64::from(self.state.preferred_height()));
        (self.sync)();
        self.view.render();
    }

    pub fn connect_status(&self, handler: impl Fn(FrameGraphStatus) + 'static) {
        self.status_handlers.borrow_mut().push(Rc::new(handler));
    }

    fn dispatch(&self, actions: Vec<FrameGraphComponentAction>) {
        for action in actions {
            (self.on_action)(action);
        }
        self.refresh();
    }
}

fn graph_button(icon: &str, tooltip: &str, mtm: MainThreadMarker) -> Retained<NSButton> {
    let button =
        unsafe { NSButton::buttonWithImage_target_action(&symbol(icon, tooltip), None, None, mtm) };
    button.setBordered(false);
    button.setToolTip(Some(&NSString::from_str(tooltip)));
    button
}

fn symbol(name: &str, label: &str) -> Retained<NSImage> {
    NSImage::imageWithSystemSymbolName_accessibilityDescription(
        &NSString::from_str(name),
        Some(&NSString::from_str(label)),
    )
    .unwrap_or_else(|| panic!("macOS must provide the {name} system symbol"))
}

fn attach_graph_button(
    button: &NSButton,
    view: &shrimply_framegraph_appkit::FrameGraphView,
    state: &SharedFrameGraphState,
    handler: &ActionHandler,
    sync: &Rc<dyn Fn()>,
    command: FrameGraphCommand,
    mtm: MainThreadMarker,
) {
    let view = Weak::new(view);
    let state = state.clone();
    let handler = handler.clone();
    let sync = sync.clone();
    action::attach(
        button,
        move |_| {
            let result = state.command(command);
            for action in result.actions {
                handler(action);
            }
            sync();
            if let Some(view) = view.load() {
                view.window()
                    .expect("frame graph attached")
                    .makeFirstResponder(Some(&view));
                view.render();
            }
        },
        mtm,
    );
}

#[allow(clippy::too_many_arguments)]
fn show_interpolation_menu(
    view: &shrimply_framegraph_appkit::FrameGraphView,
    state: SharedFrameGraphState,
    handler: ActionHandler,
    component: usize,
    owner_id: uuid::Uuid,
    selected: Interpolation,
    point: NSPoint,
    mtm: MainThreadMarker,
) {
    let choices = Interpolation::KEYFRAME
        .into_iter()
        .map(|interpolation| StringChoice {
            value: interpolation.label().to_string(),
            label: interpolation.label().to_string(),
        })
        .collect();
    let view = Weak::new(view);
    let host = view.load().expect("interpolation graph must be attached");
    show_searchable_popover_at(
        &host,
        point,
        "Search interpolations",
        choices,
        selected.label().to_string(),
        move |value| {
            let interpolation = Interpolation::KEYFRAME
                .into_iter()
                .find(|interpolation| interpolation.label() == value)
                .expect("interpolation search returned a known choice");
            state.set_interpolation(owner_id, interpolation);
            handler(FrameGraphComponentAction {
                component,
                action: FrameGraphAction::InterpolationRequested {
                    owner_id,
                    interpolation,
                    x: point.x,
                    y: point.y,
                },
            });
            if let Some(view) = view.load() {
                view.render();
            }
        },
        mtm,
    );
}
