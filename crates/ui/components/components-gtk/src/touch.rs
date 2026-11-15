//! Native contact coordinates; gesture recognition belongs to shared TouchInput.
use gtk::prelude::*;
use gtk::{gdk, glib};
use shrimply_components_skia::input::{TouchChange, TouchPhase};

pub fn connect(widget: &impl IsA<gtk::Widget>, changed: impl Fn(TouchChange) + 'static) {
    let widget = widget.as_ref().downgrade();
    let controller = gtk::EventControllerLegacy::new();
    controller.set_propagation_phase(gtk::PropagationPhase::Capture);
    controller.connect_event({
        let widget = widget.clone();
        move |_, event| {
            let phase = match event.event_type() {
                gdk::EventType::TouchBegin => TouchPhase::Begin,
                gdk::EventType::TouchUpdate => TouchPhase::Move,
                gdk::EventType::TouchEnd => TouchPhase::End,
                gdk::EventType::TouchCancel => TouchPhase::Cancel,
                _ => return glib::Propagation::Proceed,
            };
            let widget = widget
                .upgrade()
                .expect("contact widget is alive during input");
            let (x, y) = event.position().expect("touch event has a position");
            let native = widget.native().expect("touch widget has a native surface");
            let (offset_x, offset_y) = native.surface_transform();
            let native = native
                .dynamic_cast::<gtk::Widget>()
                .expect("GTK native is a widget");
            let point = native
                .compute_point(
                    &widget,
                    &gtk::graphene::Point::new((x + offset_x) as f32, (y + offset_y) as f32),
                )
                .expect("touch widget shares its native coordinate system");
            changed(TouchChange {
                id: event.event_sequence().as_ptr() as usize as u64,
                point: shrimply_components_skia::Vec2::new(point.x(), point.y()),
                phase,
            });
            // Do not apply GTK mouse emulation to the same shared contact.
            glib::Propagation::Stop
        }
    });
    widget
        .upgrade()
        .expect("contact widget is alive")
        .add_controller(controller);
}
