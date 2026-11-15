use super::*;
use shrimply_components_skia::input::TouchPhase;

pub(super) fn connect(
    area: &gtk::GLArea,
    state: &SharedFrameGraphState,
    handler: &ActionHandler,
    sync: &Rc<dyn Fn()>,
    animation: &Rc<Cell<bool>>,
) {
    crate::touch::connect(area, {
        let area = area.downgrade();
        let state = state.clone();
        let handler = handler.clone();
        let sync = sync.clone();
        let animation = animation.clone();
        move |change| {
            let Some(area) = area.upgrade() else {
                return;
            };
            if change.phase == TouchPhase::Begin {
                area.grab_focus();
            }
            let result = state.touch(
                &[change],
                FrameGraphPointerPosition {
                    x: f64::from(change.point.x),
                    y: f64::from(change.point.y),
                    width: f64::from(area.width().max(1)),
                    height: f64::from(area.height().max(1)),
                },
            );
            finish_input(&area, &state, &handler, &sync, result);
            start_animation_if_needed(&area, &state, &animation, &handler, &sync);
        }
    });
    area.connect_unmap({
        let state = state.clone();
        let handler = handler.clone();
        let sync = sync.clone();
        move |area| finish_input(area, &state, &handler, &sync, state.cancel_touch())
    });
}
