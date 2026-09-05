use super::*;
use shrimply_preview_runtime::guides::{GuideCursor, GuideInput};

pub(super) fn move_to(
    input: &Rc<RefCell<GuideInput>>,
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    state: &Rc<RefCell<VideoSurfaceState>>,
    position: GlamVec2,
) -> GuideCursor {
    let mut project = project.borrow_mut();
    let state = state.borrow();
    let viewport = surface_viewport(area, &project, &state);
    let mut input = input.borrow_mut();
    input.pointer_move(
        &mut project.preview_guides,
        viewport,
        state.guides_visible,
        position,
    );
    input.cursor()
}

pub(super) fn press(
    input: &Rc<RefCell<GuideInput>>,
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    state: &Rc<RefCell<VideoSurfaceState>>,
    position: GlamVec2,
) -> bool {
    let mut project = project.borrow_mut();
    let state = state.borrow();
    let viewport = surface_viewport(area, &project, &state);
    input.borrow_mut().pointer_press(
        &mut project.preview_guides,
        viewport,
        state.guides_visible,
        position,
    )
}

pub(super) fn finish(
    input: &Rc<RefCell<GuideInput>>,
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    state: &Rc<RefCell<VideoSurfaceState>>,
    controller: &Rc<RefCell<PreviewControllerState>>,
    position: GlamVec2,
) {
    let mut project_state = project.borrow_mut();
    let state = state.borrow();
    let viewport = surface_viewport(area, &project_state, &state);
    let changed = input
        .borrow_mut()
        .pointer_release(&mut project_state.preview_guides, viewport, position)
        .expect("active guide input must finish a drag");
    drop(project_state);
    if changed {
        guides::commit_edit(&project.borrow());
    }
    let mut controller = controller.borrow_mut();
    controller.core.sequence = PointerSequence::Idle;
    controller.core.context_invalidated = changed;
    drop(controller);
    area.set_cursor_from_name(None);
    area.queue_render();
}

pub(super) fn cancel(
    input: &Rc<RefCell<GuideInput>>,
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    controller: &Rc<RefCell<PreviewControllerState>>,
) -> bool {
    let canceled = input
        .borrow_mut()
        .pointer_cancel(&mut project.borrow_mut().preview_guides);
    if canceled {
        controller.borrow_mut().core.sequence = PointerSequence::Idle;
        area.set_cursor_from_name(None);
        area.queue_render();
    }
    canceled
}
