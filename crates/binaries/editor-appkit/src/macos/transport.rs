use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSLayoutAttribute, NSLayoutConstraint, NSStackView, NSTextField,
    NSUserInterfaceLayoutOrientation,
};
use objc2_foundation::{MainThreadMarker, NSObjectProtocol, NSRect};
use std::cell::{Cell, OnceCell};

pub(super) const ROW_HEIGHT: f64 = 44.0;

struct State {
    buttons: Retained<NSStackView>,
    seek: Retained<NSStackView>,
    time: Retained<NSTextField>,
    height: OnceCell<Retained<NSLayoutConstraint>>,
    stacked: Cell<bool>,
}

define_class!(
    #[unsafe(super(NSStackView))]
    #[thread_kind = MainThreadOnly]
    #[ivars = State]
    struct Transport;

    unsafe impl NSObjectProtocol for Transport {}

    impl Transport {
        #[unsafe(method(layout))]
        fn layout(&self) {
            let stacked = shrimply_components_core::layout::transport_stacked(
                self.bounds().size.width as f32,
            );
            if let Some(height) = self.ivars().height.get()
                && stacked != self.ivars().stacked.replace(stacked)
            {
                let state = self.ivars();
                if stacked {
                    state.seek.removeArrangedSubview(&state.time);
                    state.time.removeFromSuperview();
                    state.buttons.addArrangedSubview(&state.time);
                } else {
                    state.buttons.removeArrangedSubview(&state.time);
                    state.time.removeFromSuperview();
                    state.seek.insertArrangedSubview_atIndex(&state.time, 1);
                }
                self.removeArrangedSubview(&state.seek);
                self.insertArrangedSubview_atIndex(&state.seek, if stacked { 0 } else { 1 });
                self.setOrientation(if stacked {
                    NSUserInterfaceLayoutOrientation::Vertical
                } else {
                    NSUserInterfaceLayoutOrientation::Horizontal
                });
                self.setAlignment(if stacked {
                    NSLayoutAttribute::Width
                } else {
                    NSLayoutAttribute::CenterY
                });
                height.setConstant(if stacked { ROW_HEIGHT * 2.0 } else { ROW_HEIGHT });
            }
            unsafe { let _: () = msg_send![super(self), layout]; }
        }
    }
);

pub(super) fn build(
    buttons: &Retained<NSStackView>,
    seek: &Retained<NSStackView>,
    time: &Retained<NSTextField>,
    mtm: MainThreadMarker,
) -> Retained<NSStackView> {
    let view = Transport::alloc(mtm).set_ivars(State {
        buttons: buttons.clone(),
        seek: seek.clone(),
        time: time.clone(),
        height: OnceCell::new(),
        stacked: Cell::new(false),
    });
    let view: Retained<Transport> = unsafe { msg_send![super(view), initWithFrame: NSRect::ZERO] };
    view.setOrientation(NSUserInterfaceLayoutOrientation::Horizontal);
    view.addArrangedSubview(buttons);
    view.addArrangedSubview(seek);
    let height = view.heightAnchor().constraintEqualToConstant(ROW_HEIGHT);
    height.setActive(true);
    view.ivars()
        .height
        .set(height)
        .expect("transport height installed once");
    view.into_super()
}
