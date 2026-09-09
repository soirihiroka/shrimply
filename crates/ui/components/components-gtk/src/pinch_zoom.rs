use std::cell::Cell;
use std::rc::Rc;

use gtk::prelude::*;

pub fn connect_pinch_zoom(
    area: &gtk::GLArea,
    pointer: impl Fn() -> Option<(f64, f64)> + 'static,
    magnify: impl Fn((f64, f64), f64) + 'static,
) {
    let zoom = gtk::GestureZoom::new();
    let previous_scale = Rc::new(Cell::new(1.0));
    zoom.connect_begin({
        let previous_scale = previous_scale.clone();
        move |_, _| previous_scale.set(1.0)
    });
    zoom.connect_end({
        let previous_scale = previous_scale.clone();
        move |_, _| previous_scale.set(1.0)
    });
    zoom.connect_cancel({
        let previous_scale = previous_scale.clone();
        move |_, _| previous_scale.set(1.0)
    });
    zoom.connect_scale_changed({
        let area = area.clone();
        move |gesture, scale| {
            let Some(magnification) =
                shrimply_math_core::pinch_magnification(scale, previous_scale.get())
            else {
                return;
            };
            previous_scale.set(scale);
            let point = gesture
                .bounding_box_center()
                .or_else(|| gesture.current_event().and_then(|event| event.position()))
                .or_else(&pointer)
                .unwrap_or_else(|| {
                    (
                        f64::from(area.width()) / 2.0,
                        f64::from(area.height()) / 2.0,
                    )
                });
            magnify(point, magnification);
            area.queue_render();
        }
    });
    area.add_controller(zoom);
}
