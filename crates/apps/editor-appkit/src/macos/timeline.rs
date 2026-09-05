use super::layout::{BUTTON_SIZE, TOOLBAR_WIDTH, button, stack};
use objc2::rc::Retained;
use objc2::{MainThreadOnly, sel};
use objc2_app_kit::{NSBox, NSBoxType, NSStackView, NSView};
use objc2_foundation::{MainThreadMarker, NSEdgeInsets, NSRect};

// Match the GTK timeline's tool rail, track labels, ruler, and audio-meter widths.
const AUDIO_METER_WIDTH: f64 = shrimply_skia_adw_core::audio_meter::DEFAULT_WIDTH as f64;
const DIVIDER_WIDTH: f64 = 1.0;
const TOOL_GAP: f64 = 4.0;

#[derive(Clone, Copy)]
#[repr(isize)]
pub enum Tool {
    Magnet,
    BeatGrid,
    Pointer,
    Cut,
    Overwrite,
    Block,
    NewTrack,
}

impl Tool {
    pub fn selected(self, state: shrimply_timeline_core::ToolState) -> bool {
        use shrimply_timeline_core::{CursorTool, DragCollisionMode};
        match self {
            Self::Magnet => state.magnet,
            Self::BeatGrid => state.beat_grid,
            Self::Pointer => state.cursor == CursorTool::Pointer,
            Self::Cut => state.cursor == CursorTool::Cut,
            Self::Overwrite => state.drag_collision == DragCollisionMode::Overwrite,
            Self::Block => state.drag_collision == DragCollisionMode::Block,
            Self::NewTrack => state.drag_collision == DragCollisionMode::NewTrack,
        }
    }

    pub fn activate(self, tools: &shrimply_timeline_core::TimelineTools) {
        use shrimply_timeline_core::{CursorTool, DragCollisionMode};
        match self {
            Self::Magnet => tools.set_magnet(!tools.state().magnet),
            Self::BeatGrid => tools.set_beat_grid(!tools.state().beat_grid),
            Self::Pointer => tools.set_cursor(CursorTool::Pointer),
            Self::Cut => tools.set_cursor(CursorTool::Cut),
            Self::Overwrite => tools.set_drag_collision(DragCollisionMode::Overwrite),
            Self::Block => tools.set_drag_collision(DragCollisionMode::Block),
            Self::NewTrack => tools.set_drag_collision(DragCollisionMode::NewTrack),
        }
    }
}

pub fn build(
    session: std::rc::Rc<shrimply_cross_ui_core::editor::EditorSession>,
    imports: std::rc::Rc<std::cell::RefCell<super::media::Imports>>,
    mtm: MainThreadMarker,
) -> (
    Retained<NSStackView>,
    Retained<super::canvas::CanvasView>,
    Retained<super::canvas::CanvasView>,
) {
    let scene = shrimply_timeline_core::scene::Scene::new(
        session.project.clone(),
        session.player_state.clone(),
        session.selection_state.clone(),
        session.preferences.clone(),
        session.property_clipboard.clone(),
    );
    let tracks = super::canvas::new(
        super::canvas::Content::Timeline(Box::new(scene)),
        session.clone(),
        imports.clone(),
        mtm,
    );
    let tools = stack(true, mtm);
    tools.setSpacing(TOOL_GAP);
    tools.setAlignment(objc2_app_kit::NSLayoutAttribute::CenterX);
    tools.setEdgeInsets(NSEdgeInsets {
        top: TOOL_GAP,
        left: TOOL_GAP,
        bottom: TOOL_GAP,
        right: TOOL_GAP,
    });
    tools
        .widthAnchor()
        .constraintEqualToConstant(TOOLBAR_WIDTH)
        .setActive(true);
    for group in [
        &[
            (Tool::Magnet, "paperclip", "Magnet"),
            (Tool::BeatGrid, "metronome", "Beat Grid"),
        ][..],
        &[
            (Tool::Pointer, "cursorarrow", "Pointer"),
            (Tool::Cut, "scissors", "Cut"),
        ][..],
        &[
            (
                Tool::Overwrite,
                "rectangle.on.rectangle",
                "Overwrite/Insert",
            ),
            (Tool::Block, "pause.rectangle", "Block"),
            (Tool::NewTrack, "rectangle.badge.plus", "New Track"),
        ][..],
    ] {
        if tools.arrangedSubviews().count() != 0 {
            let divider = NSBox::initWithFrame(NSBox::alloc(mtm), NSRect::ZERO);
            divider.setBoxType(NSBoxType::Separator);
            divider
                .widthAnchor()
                .constraintEqualToConstant(BUTTON_SIZE)
                .setActive(true);
            divider
                .heightAnchor()
                .constraintEqualToConstant(DIVIDER_WIDTH)
                .setActive(true);
            tools.addArrangedSubview(&divider);
        }
        for (tool, icon, label) in group {
            let button = button(icon, label, mtm);
            button.setEnabled(true);
            button.setButtonType(objc2_app_kit::NSButtonType::PushOnPushOff);
            button.setTag(*tool as isize);
            unsafe {
                button.setTarget(Some(&*tracks));
                button.setAction(Some(sel!(changeTimelineTool:)));
            }
            tracks.register_tool(*tool, button.clone());
            tools.addArrangedSubview(&button);
        }
    }
    tools.addArrangedSubview(&NSView::initWithFrame(NSView::alloc(mtm), NSRect::ZERO));

    let meter = super::canvas::new(
        super::canvas::Content::Meter(Default::default()),
        session,
        imports,
        mtm,
    );
    meter
        .widthAnchor()
        .constraintEqualToConstant(AUDIO_METER_WIDTH)
        .setActive(true);
    let timeline = stack(false, mtm);
    timeline.setSpacing(DIVIDER_WIDTH);
    timeline.addArrangedSubview(&tools);
    timeline.addArrangedSubview(&tracks);
    timeline.addArrangedSubview(&meter);
    (timeline, tracks, meter)
}
