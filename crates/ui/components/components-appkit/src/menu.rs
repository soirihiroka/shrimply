use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSBezelStyle, NSButton, NSControl, NSEvent, NSPopUpButton, NSTouchPhase, NSTouchType,
    NSTouchTypeMask, NSView,
};
use objc2_foundation::{MainThreadMarker, NSObjectProtocol, NSRect, NSString};
use shrimply_components_skia::input::{TouchAction, TouchChange, TouchInput, TouchPhase};
use std::cell::{Cell, RefCell};

#[derive(Default)]
struct MenuTrigger {
    input: RefCell<TouchInput>,
    dispatching_touch: Cell<bool>,
}

impl MenuTrigger {
    fn update(
        &self,
        view: &NSView,
        event: &NSEvent,
        native: NSTouchPhase,
        phase: TouchPhase,
    ) -> bool {
        let changes = event
            .touchesMatchingPhase_inView(native, Some(view))
            .iter()
            .filter(|touch| touch.r#type() == NSTouchType::Direct)
            .map(|touch| {
                let point = touch.locationInView(Some(view));
                TouchChange {
                    id: Retained::as_ptr(&touch.identity()) as usize as u64,
                    point: shrimply_components_skia::Vec2::new(point.x as f32, point.y as f32),
                    phase,
                }
            })
            .collect::<Vec<_>>();
        if changes.is_empty() {
            return false;
        }
        matches!(
            self.input.borrow_mut().update(&changes).as_slice(),
            [TouchAction::Press(_), TouchAction::Release(_)]
        )
    }

    fn pointer(&self, view: &NSView, event: &NSEvent) -> bool {
        if self.input.borrow().active()
            || event
                .touchesMatchingPhase_inView(NSTouchPhase::Any, Some(view))
                .iter()
                .any(|touch| touch.r#type() == NSTouchType::Direct)
        {
            return false;
        }
        self.input.borrow_mut().cancel();
        true
    }
}

define_class!(
    #[unsafe(super(NSButton))]
    #[thread_kind = MainThreadOnly]
    #[ivars = MenuTrigger]
    struct MenuButton;

    unsafe impl NSObjectProtocol for MenuButton {}

    impl MenuButton {
        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            if self.ivars().pointer(self, event) {
                unsafe { let _: () = msg_send![super(self), mouseDown: event]; }
            }
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            self.ivars().input.borrow_mut().cancel();
            unsafe { let _: () = msg_send![super(self), keyDown: event]; }
        }

        #[unsafe(method(touchesBeganWithEvent:))]
        fn touches_began(&self, event: &NSEvent) {
            self.ivars().update(self, event, NSTouchPhase::Began, TouchPhase::Begin);
        }

        #[unsafe(method(touchesMovedWithEvent:))]
        fn touches_moved(&self, event: &NSEvent) {
            self.ivars().update(self, event, NSTouchPhase::Moved, TouchPhase::Move);
        }

        #[unsafe(method(touchesEndedWithEvent:))]
        fn touches_ended(&self, event: &NSEvent) {
            if self.ivars().update(self, event, NSTouchPhase::Ended, TouchPhase::End)
                && self.isEnabled()
            {
                self.ivars().dispatching_touch.set(true);
                unsafe { self.sendAction_to(self.action(), self.target().as_deref()); }
                self.ivars().dispatching_touch.set(false);
            }
        }

        #[unsafe(method(touchesCancelledWithEvent:))]
        fn touches_cancelled(&self, event: &NSEvent) {
            self.ivars().update(self, event, NSTouchPhase::Cancelled, TouchPhase::Cancel);
        }

        #[unsafe(method(viewDidMoveToWindow))]
        fn view_did_move_to_window(&self) {
            unsafe { let _: () = msg_send![super(self), viewDidMoveToWindow]; }
            if self.window().is_none() {
                self.ivars().input.borrow_mut().cancel();
            }
        }
    }
);

define_class!(
    #[unsafe(super(NSPopUpButton))]
    #[thread_kind = MainThreadOnly]
    #[ivars = MenuTrigger]
    struct MenuPopup;

    unsafe impl NSObjectProtocol for MenuPopup {}

    impl MenuPopup {
        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            if self.ivars().pointer(self, event) {
                unsafe { let _: () = msg_send![super(self), mouseDown: event]; }
            }
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            self.ivars().input.borrow_mut().cancel();
            unsafe { let _: () = msg_send![super(self), keyDown: event]; }
        }

        #[unsafe(method(touchesBeganWithEvent:))]
        fn touches_began(&self, event: &NSEvent) {
            self.ivars().update(self, event, NSTouchPhase::Began, TouchPhase::Begin);
        }

        #[unsafe(method(touchesMovedWithEvent:))]
        fn touches_moved(&self, event: &NSEvent) {
            self.ivars().update(self, event, NSTouchPhase::Moved, TouchPhase::Move);
        }

        #[unsafe(method(touchesEndedWithEvent:))]
        fn touches_ended(&self, event: &NSEvent) {
            if self.ivars().update(self, event, NSTouchPhase::Ended, TouchPhase::End)
                && self.isEnabled()
            {
                crate::controls::show_popup_sheet(self, self.mtm());
            }
        }

        #[unsafe(method(touchesCancelledWithEvent:))]
        fn touches_cancelled(&self, event: &NSEvent) {
            self.ivars().update(self, event, NSTouchPhase::Cancelled, TouchPhase::Cancel);
        }

        #[unsafe(method(viewDidMoveToWindow))]
        fn view_did_move_to_window(&self) {
            unsafe { let _: () = msg_send![super(self), viewDidMoveToWindow]; }
            if self.window().is_none() {
                self.ivars().input.borrow_mut().cancel();
            }
        }
    }
);

pub(crate) fn button(title: &str, mtm: MainThreadMarker) -> Retained<NSButton> {
    let button = MenuButton::alloc(mtm).set_ivars(MenuTrigger::default());
    let button: Retained<MenuButton> =
        unsafe { msg_send![super(button), initWithFrame: NSRect::ZERO] };
    button.setTitle(&NSString::from_str(title));
    button.setBezelStyle(NSBezelStyle::Push);
    button.setAllowedTouchTypes(NSTouchTypeMask::Direct);
    button.into_super()
}

pub(crate) fn touch_triggered(control: &NSControl) -> bool {
    control
        .downcast_ref::<MenuButton>()
        .is_some_and(|button| button.ivars().dispatching_touch.get())
}

/// Native popup behavior remains with AppKit; direct touch opens the same choices in a sheet.
pub fn popup(frame: NSRect, pulls_down: bool, mtm: MainThreadMarker) -> Retained<NSPopUpButton> {
    let popup = MenuPopup::alloc(mtm).set_ivars(MenuTrigger::default());
    let popup: Retained<MenuPopup> =
        unsafe { msg_send![super(popup), initWithFrame: frame, pullsDown: pulls_down] };
    popup.setAllowedTouchTypes(NSTouchTypeMask::Direct);
    popup.into_super()
}
