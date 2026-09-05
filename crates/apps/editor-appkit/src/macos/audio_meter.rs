use std::cell::RefCell;

use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, rc::Retained};
use objc2_app_kit::{NSView, NSViewLayerContentsRedrawPolicy};
use objc2_foundation::{MainThreadMarker, NSRect, NSSize};
use shrimply_skia_adw_core::audio_meter::AudioMeter;
use shrimply_skia_metal::Renderer;

pub struct MeterState {
    renderer: RefCell<Renderer>,
    meter: AudioMeter,
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[ivars = MeterState]
    pub struct MeterView;

    impl MeterView {
        #[unsafe(method(drawRect:))]
        fn draw(&self, _dirty: NSRect) {
            let size = self.bounds().size;
            if size.width <= 0.0 || size.height <= 0.0 { return; }
            let scale = self.window().map_or(1.0, |window| window.backingScaleFactor());
            let mut renderer = self.ivars().renderer.borrow_mut();
            renderer.layer().setContentsScale(scale);
            renderer.layer().setDrawableSize(NSSize::new((size.width * scale).ceil(), (size.height * scale).ceil()));
            renderer.draw(|canvas| {
                canvas.clear(shrimply_cross_ui_theme::current().view_bg);
                canvas.scale((scale as f32, scale as f32));
                self.ivars().meter.draw(canvas, size.width as f32, size.height as f32);
            });
        }

        #[unsafe(method(viewDidChangeBackingProperties))]
        fn backing_changed(&self) {
            unsafe { let _: () = msg_send![super(self), viewDidChangeBackingProperties]; }
            self.setNeedsDisplay(true);
        }
    }
);

pub fn new(mtm: MainThreadMarker) -> Retained<MeterView> {
    let view = MeterView::alloc(mtm).set_ivars(MeterState {
        renderer: RefCell::new(Renderer::default()),
        meter: AudioMeter::default(),
    });
    let view: Retained<MeterView> = unsafe { msg_send![super(view), initWithFrame: NSRect::ZERO] };
    view.setWantsLayer(true);
    view.setLayer(Some(view.ivars().renderer.borrow().layer()));
    view.setLayerContentsRedrawPolicy(NSViewLayerContentsRedrawPolicy::DuringViewResize);
    view.setNeedsDisplay(true);
    view
}
