use super::*;
use objc2_app_kit::{NSTouchPhase, NSTouchType};
use shrimply_components_skia::input::{TouchChange, TouchPhase};

impl CanvasView {
    pub(super) fn forward_touch(
        &self,
        event: &NSEvent,
        native_phase: NSTouchPhase,
        phase: TouchPhase,
    ) {
        let mut content = self.ivars().content.borrow_mut();
        let Content::Timeline(scene) = &mut *content else {
            return;
        };
        let changes = event
            .touchesMatchingPhase_inView(native_phase, Some(self))
            .iter()
            .filter(|touch| touch.r#type() == NSTouchType::Direct)
            .map(|touch| {
                let point = touch.locationInView(Some(self));
                TouchChange {
                    id: Retained::as_ptr(&touch.identity()) as usize as u64,
                    point: shrimply_components_skia::Vec2::new(point.x as f32, point.y as f32),
                    phase,
                }
            })
            .collect::<Vec<_>>();
        if !changes.is_empty() {
            scene.event(shrimply_timeline_skia::scene::Event::Touch(changes));
            self.ivars().surface_dirty.set(true);
        }
    }
}
