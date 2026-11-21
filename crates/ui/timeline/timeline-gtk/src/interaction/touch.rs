use super::*;
use shrimply_components_skia::input::{TouchChange, TouchPhase};
use shrimply_timeline_skia::scene::Event;

pub(super) fn add_controller(area: &gtk::GLArea, runtime: &Rc<RefCell<TimelineRuntime>>) {
    let controller = gtk::EventControllerLegacy::new();
    controller.set_propagation_phase(gtk::PropagationPhase::Capture);
    let event_area = area.clone();
    let runtime = runtime.clone();
    controller.connect_event(move |_, event| {
        let phase = match event.event_type() {
            gdk::EventType::TouchBegin => TouchPhase::Begin,
            gdk::EventType::TouchUpdate => TouchPhase::Move,
            gdk::EventType::TouchEnd => TouchPhase::End,
            gdk::EventType::TouchCancel => TouchPhase::Cancel,
            _ => return glib::Propagation::Proceed,
        };
        let (x, y) = event.position().expect("touch event has a position");
        let native = event_area
            .native()
            .expect("realized timeline has a native surface");
        let (offset_x, offset_y) = native.surface_transform();
        let native_widget = native
            .dynamic_cast::<gtk::Widget>()
            .expect("GTK native is a widget");
        let point = native_widget
            .compute_point(
                &event_area,
                &gtk::graphene::Point::new((x + offset_x) as f32, (y + offset_y) as f32),
            )
            .expect("timeline shares its native widget's coordinate system");
        let change = TouchChange {
            id: event.event_sequence().as_ptr() as usize as u64,
            point: vec2(point.x(), point.y()),
            phase,
        };
        if phase == TouchPhase::Begin {
            event_area.grab_focus();
        }
        runtime.borrow_mut().scene.event(Event::Touch(vec![change]));
        event_area.queue_render();
        start_timeline_animation_tick(&event_area, runtime.clone());
        // The shared input layer owns touch. GTK's mouse emulation and pinch
        // recognizer must not apply the same contact a second time.
        glib::Propagation::Stop
    });
    area.add_controller(controller);
}
