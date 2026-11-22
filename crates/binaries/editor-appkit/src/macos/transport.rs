use super::layout::{BUTTON_SIZE, GAP};
use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSLayoutConstraint, NSLayoutConstraintOrientation, NSLayoutPriorityDefaultLow, NSLineBreakMode,
    NSSlider, NSTextAlignment, NSTextField, NSView,
};
use objc2_foundation::{MainThreadMarker, NSObjectProtocol, NSPoint, NSRect, NSSize};
use std::cell::{Cell, OnceCell};

pub(super) const ROW_HEIGHT: f64 = 44.0;

struct State {
    buttons: [Retained<NSView>; 3],
    progress: Retained<NSSlider>,
    time: Retained<NSTextField>,
    fullscreen: Retained<NSView>,
    height: OnceCell<Retained<NSLayoutConstraint>>,
    stacked: Cell<bool>,
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[ivars = State]
    struct Transport;

    unsafe impl NSObjectProtocol for Transport {}

    impl Transport {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(setFrameSize:))]
        fn set_frame_size(&self, size: NSSize) {
            unsafe { let _: () = msg_send![super(self), setFrameSize: size]; }
            self.sync_height(size.width);
            self.setNeedsLayout(true);
        }

        #[unsafe(method(layout))]
        fn layout(&self) {
            unsafe { let _: () = msg_send![super(self), layout]; }
            let width = self.bounds().size.width;
            if width <= 0.0 {
                return;
            }
            let state = self.ivars();
            let stacked = self.sync_height(width);
            let slider_h = state.progress.fittingSize().height;
            let time_size = state.time.fittingSize();
            let time_h = time_size.height;
            let fullscreen_x = (width - GAP - BUTTON_SIZE).max(GAP);
            let button_y = (ROW_HEIGHT - BUTTON_SIZE) / 2.0;

            if stacked {
                let slider_w = (fullscreen_x - GAP - GAP).max(0.0);
                state.progress.setFrame(NSRect::new(
                    NSPoint::new(GAP, (ROW_HEIGHT - slider_h) / 2.0),
                    NSSize::new(slider_w, slider_h),
                ));
                state.fullscreen.setFrame(NSRect::new(
                    NSPoint::new(fullscreen_x, button_y),
                    NSSize::new(BUTTON_SIZE, BUTTON_SIZE),
                ));
                let mut x = GAP;
                for button in &state.buttons {
                    button.setFrame(NSRect::new(
                        NSPoint::new(x, ROW_HEIGHT + button_y),
                        NSSize::new(BUTTON_SIZE, BUTTON_SIZE),
                    ));
                    x += BUTTON_SIZE + GAP;
                }
                let time_w = (width - GAP - x).max(0.0);
                state.time.setFrame(NSRect::new(
                    NSPoint::new(x, ROW_HEIGHT + (ROW_HEIGHT - time_h) / 2.0),
                    NSSize::new(time_w, time_h),
                ));
            } else {
                let mut x = GAP;
                for button in &state.buttons {
                    button.setFrame(NSRect::new(
                        NSPoint::new(x, button_y),
                        NSSize::new(BUTTON_SIZE, BUTTON_SIZE),
                    ));
                    x += BUTTON_SIZE + GAP;
                }
                let time_w = time_size.width.ceil().min((fullscreen_x - GAP - x).max(0.0));
                let time_x = (fullscreen_x - GAP - time_w).max(x);
                let slider_w = (time_x - GAP - x).max(0.0);
                state.progress.setFrame(NSRect::new(
                    NSPoint::new(x, (ROW_HEIGHT - slider_h) / 2.0),
                    NSSize::new(slider_w, slider_h),
                ));
                state.time.setFrame(NSRect::new(
                    NSPoint::new(time_x, (ROW_HEIGHT - time_h) / 2.0),
                    NSSize::new(time_w, time_h),
                ));
                state.fullscreen.setFrame(NSRect::new(
                    NSPoint::new(fullscreen_x, button_y),
                    NSSize::new(BUTTON_SIZE, BUTTON_SIZE),
                ));
            }
        }
    }
);

impl Transport {
    fn sync_height(&self, width: f64) -> bool {
        if width <= 0.0 {
            return self.ivars().stacked.get();
        }
        let stacked = shrimply_components_core::layout::transport_stacked(width as f32);
        if let Some(height) = self.ivars().height.get()
            && stacked != self.ivars().stacked.replace(stacked)
        {
            height.setConstant(if stacked {
                ROW_HEIGHT * 2.0
            } else {
                ROW_HEIGHT
            });
        }
        stacked
    }
}

pub(super) fn build(
    buttons: [Retained<NSView>; 3],
    progress: Retained<NSSlider>,
    time: Retained<NSTextField>,
    fullscreen: Retained<NSView>,
    mtm: MainThreadMarker,
) -> Retained<NSView> {
    time.setAlignment(NSTextAlignment::Right);
    time.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    let view = Transport::alloc(mtm).set_ivars(State {
        buttons,
        progress,
        time,
        fullscreen,
        height: OnceCell::new(),
        stacked: Cell::new(false),
    });
    let view: Retained<Transport> = unsafe { msg_send![super(view), initWithFrame: NSRect::ZERO] };
    view.setTranslatesAutoresizingMaskIntoConstraints(false);
    view.setContentHuggingPriority_forOrientation(
        NSLayoutPriorityDefaultLow,
        NSLayoutConstraintOrientation::Horizontal,
    );
    view.setContentCompressionResistancePriority_forOrientation(
        NSLayoutPriorityDefaultLow,
        NSLayoutConstraintOrientation::Horizontal,
    );
    let state = view.ivars();
    for button in &state.buttons {
        view.addSubview(button);
    }
    view.addSubview(&state.progress);
    view.addSubview(&state.time);
    view.addSubview(&state.fullscreen);
    let height = view.heightAnchor().constraintEqualToConstant(ROW_HEIGHT);
    height.setActive(true);
    state
        .height
        .set(height)
        .expect("transport height installed once");
    view.into_super()
}
