use objc2::runtime::ProtocolObject;
use objc2::{AnyThread, DefinedClass, MainThreadOnly, define_class, msg_send, rc::Retained};
use objc2_app_kit::{
    NSDragOperation, NSDraggingDestination, NSDraggingInfo, NSEvent, NSEventModifierFlags,
    NSPasteboard, NSPasteboardTypeFileURL, NSPasteboardTypePNG, NSPasteboardTypeString,
    NSPasteboardTypeTIFF, NSPasteboardTypeURL, NSTrackingArea, NSTrackingAreaOptions, NSView,
};
use objc2_foundation::{MainThreadMarker, NSRect, NSSize, NSString, NSURL};
use objc2_foundation::{NSArray, NSObjectProtocol};
use shrimply_components_skia::audio_meter::AudioMeter;
use shrimply_cross_ui_core::editor::EditorSession;
use shrimply_preview_provider_skia::{KeyState, PointerEvent};
use shrimply_surface_metal_skia::Renderer;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

mod context_audio;
mod context_captions;
mod context_frame;
mod context_menu;
mod context_video;
pub(super) mod preview;
mod screen_recording;
mod track_actions;

pub enum Content {
    Timeline(Box<shrimply_timeline_skia::scene::Scene>),
    Preview(Box<preview::State>),
    Meter(AudioMeter),
}

pub struct CanvasState {
    renderer: RefCell<Renderer>,
    surface_dirty: Rc<Cell<bool>>,
    content: RefCell<Content>,
    session: Rc<EditorSession>,
    imports: Rc<RefCell<super::media::Imports>>,
    tracking_area: RefCell<Option<Retained<NSTrackingArea>>>,
    menu_choice: Cell<Option<usize>>,
    context_controls: RefCell<Vec<shrimply_timeline_skia::ContextMenuControl>>,
    context_error: RefCell<Option<String>>,
    suppress_primary: Cell<bool>,
    secondary_preview_active: Cell<bool>,
    relative_pan_active: Cell<bool>,
    pointer_active: Cell<bool>,
    audio_export: RefCell<Option<context_audio::AudioExport>>,
    video_export: RefCell<Option<context_video::VideoExport>>,
    caption_speech_probe: RefCell<Option<context_menu::CaptionSpeechProbe>>,
    caption_speech_alert: RefCell<Option<Retained<objc2_app_kit::NSAlert>>>,
    transcription_probe: RefCell<Option<context_menu::TranscriptionProbe>>,
    transcription_alert: RefCell<Option<Retained<objc2_app_kit::NSAlert>>>,
    frame_capture: RefCell<Option<context_frame::FrameCapture>>,
    screen_recording: RefCell<Option<screen_recording::ScreenRecording>>,
    drop_source: RefCell<Option<(std::path::PathBuf, super::media::ScopedUrl)>>,
    tools: RefCell<Vec<(super::timeline::Tool, Retained<objc2_app_kit::NSButton>)>>,
    paint_tools: RefCell<Vec<(PaintTool, Retained<objc2_app_kit::NSButton>)>>,
}

#[derive(Clone, Copy)]
pub(super) enum PaintTool {
    Pen,
    Fill,
    Eraser,
    Adjust,
    Transform,
    Smaller,
    Larger,
    Palette,
    OnionPrevious,
    OnionNext,
}

const MIDDLE_MOUSE_BUTTON: isize = 2;
const MASK_PASTEBOARD_TYPE: &str = "com.shrimply.mask-modifier";

enum DropPayload {
    Files(Vec<Retained<NSURL>>),
    Mask(String),
    Image,
    ImageUrl(String),
    Text(String),
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[ivars = CanvasState]
    pub struct CanvasView;

    unsafe impl NSObjectProtocol for CanvasView {}
    unsafe impl NSDraggingDestination for CanvasView {
        #[unsafe(method(draggingExited:))]
        fn dragging_exited(&self, _sender: Option<&ProtocolObject<dyn NSDraggingInfo>>) {
            self.clear_drop_preview();
        }

        #[unsafe(method(concludeDragOperation:))]
        fn conclude_drag(&self, _sender: Option<&ProtocolObject<dyn NSDraggingInfo>>) {
            self.clear_drop_preview();
        }
        #[unsafe(method(draggingEntered:))]
        fn dragging_entered(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
            self.drag_operation(sender)
        }

        #[unsafe(method(draggingUpdated:))]
        fn dragging_updated(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
            self.drag_operation(sender)
        }

        #[unsafe(method(prepareForDragOperation:))]
        fn prepare_drag(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> bool {
            self.drag_operation(sender) == NSDragOperation::Copy
        }

        #[unsafe(method(performDragOperation:))]
        fn perform_drop(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> bool {
            self.perform_file_drop(sender)
        }
    }

    impl CanvasView {
        #[unsafe(method(viewDidChangeEffectiveAppearance))]
        fn appearance_changed(&self) {
            unsafe { let _: () = msg_send![super(self), viewDidChangeEffectiveAppearance]; }
            self.ivars().surface_dirty.set(true);
        }

        #[unsafe(method(viewDidUnhide))]
        fn did_unhide(&self) {
            unsafe { let _: () = msg_send![super(self), viewDidUnhide]; }
            self.ivars().surface_dirty.set(true);
        }

        #[unsafe(method(updateTrackingAreas))]
        fn update_tracking_areas(&self) {
            unsafe { let _: () = msg_send![super(self), updateTrackingAreas]; }
            self.update_tracking();
        }

        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, event: &NSEvent) {
            self.update_pointer(self.point(event));
            self.preview_pointer_event(PointerEvent::Hover(self.preview_input(event)));
        }

        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, event: &NSEvent) {
            self.update_pointer(self.point(event));
            self.preview_pointer_event(PointerEvent::Hover(self.preview_input(event)));
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: &NSEvent) {
            if let Content::Timeline(scene) = &mut *self.ivars().content.borrow_mut() {
                scene.pointer_exited();
                objc2_app_kit::NSCursor::arrowCursor().set();
            }
            if let Content::Preview(state) = &mut *self.ivars().content.borrow_mut() { state.guide_input.pointer_leave(); }
            self.preview_pointer_event(PointerEvent::Leave);
        }

        #[unsafe(method(cancelOperation:))]
        fn cancel_operation(&self, _sender: &objc2_foundation::NSObject) {
            if let Content::Timeline(scene) = &mut *self.ivars().content.borrow_mut() {
                self.release_relative_pan(scene);
                scene.pointer_cancelled();
                Self::set_timeline_cursor(scene.pointer_cursor());
            }
            self.cancel_preview_pointer();
        }

        #[unsafe(method(chooseTrackAdd:))]
        fn choose_track_add(&self, sender: &objc2_app_kit::NSMenuItem) {
            self.ivars().menu_choice.set(Some(sender.tag().try_into().expect("track menu index")));
        }

        #[unsafe(method(chooseCanvasContext:))]
        fn choose_canvas_context(&self, sender: &objc2_app_kit::NSMenuItem) {
            self.ivars().menu_choice.set(Some(sender.tag().try_into().expect("context menu index")));
        }

        #[unsafe(method(changeTimelineContextControl:))]
        fn change_timeline_context_control(&self, sender: &objc2_app_kit::NSSlider) {
            self.change_context_control(sender);
        }

        #[unsafe(method(changeTimelineTool:))]
        fn change_timeline_tool(&self, sender: &objc2_app_kit::NSButton) {
            let tool = self.ivars().tools.borrow().iter()
                .find(|(tool, _)| *tool as isize == sender.tag()).map(|(tool, _)| *tool)
                .expect("registered timeline tool");
            tool.activate(&shrimply_timeline_skia::TimelineTools::new(self.ivars().session.preferences.clone()));
            self.sync_tools();
            self.update_tracking();
            self.window().expect("canvas attached").makeFirstResponder(Some(self));
        }

        #[unsafe(method(togglePreviewGuides:))]
        fn toggle_preview_guides(&self, sender: &objc2_app_kit::NSButton) {
            self.cancel_preview_pointer();
            if let Content::Preview(state) = &mut *self.ivars().content.borrow_mut() {
                state.guides_visible = !state.guides_visible;
                sender.setState(if state.guides_visible { objc2_app_kit::NSControlStateValueOn } else { objc2_app_kit::NSControlStateValueOff });
            }
            shrimply_editor_state::preferences::set_preview_guides_visible(&self.ivars().session.preferences, sender.state() == objc2_app_kit::NSControlStateValueOn);
        }

        #[unsafe(method(changePaintTool:))]
        fn change_paint_tool(&self, sender: &objc2_app_kit::NSButton) {
            let tool = self.ivars().paint_tools.borrow().iter()
                .find(|(tool, _)| *tool as isize == sender.tag())
                .map(|(tool, _)| *tool)
                .expect("registered paint tool");
            let palette_len = {
                let project = self.ivars().session.project.borrow();
                shrimply_timeline_skia::selection_state::focused_video_address(
                    &self.ivars().session.selection_state,
                    &project,
                )
                .and_then(|address| project.video_item(&address))
                .and_then(|item| match &item.content {
                    shrimply_project_document::project::VideoItemContent::Paint(paint) => Some(paint.palette.len()),
                    _ => None,
                })
            };
            if let Content::Preview(state) = &mut *self.ivars().content.borrow_mut() {
                use shrimply_paint_edit_skia::{PAINT_PREVIEW_STATE, PaintPreviewMode, PaintPreviewState};
                state.controller.update_extension(PAINT_PREVIEW_STATE, |paint: &mut PaintPreviewState| match tool {
                    PaintTool::Pen => { paint.set_mode(PaintPreviewMode::Pen); paint.set_eraser(false); paint.adjusting = false; }
                    PaintTool::Fill => { paint.set_mode(PaintPreviewMode::Fill); paint.set_eraser(false); paint.adjusting = false; }
                    PaintTool::Eraser => { paint.set_eraser(!paint.eraser); if paint.eraser { paint.adjusting = false; } }
                    PaintTool::Adjust => { paint.adjusting = !paint.adjusting; if paint.adjusting { paint.set_eraser(false); } }
                    PaintTool::Transform => {
                        paint.set_mode(if paint.mode == PaintPreviewMode::StrokeTransform { PaintPreviewMode::Pen } else { PaintPreviewMode::StrokeTransform });
                        paint.adjusting = false;
                    }
                    PaintTool::Smaller | PaintTool::Larger => {
                        let larger = matches!(tool, PaintTool::Larger);
                        if paint.mode == PaintPreviewMode::Fill {
                            paint.set_fill_tolerance(shrimply_paint_edit_skia::step_fill_tolerance(paint.fill_tolerance, larger));
                        } else {
                            let eraser = paint.eraser;
                            paint.set_brush_scale(eraser, shrimply_paint_edit_skia::step_tool_size(paint.brush_scale(eraser), larger));
                        }
                    }
                    PaintTool::Palette => {
                        if let Some(len) = palette_len.filter(|len| *len > 0) {
                            paint.select_palette((paint.palette_index + 1) % len, len);
                        }
                    }
                    PaintTool::OnionPrevious => paint.set_onion_skin(true, !paint.onion_previous),
                    PaintTool::OnionNext => paint.set_onion_skin(false, !paint.onion_next),
                });
            }
            self.sync_paint_tools();
            self.window().expect("canvas attached").makeFirstResponder(Some(self));
        }

        #[unsafe(method(rightMouseDown:))]
        fn right_mouse_down(&self, event: &NSEvent) {
            if matches!(&*self.ivars().content.borrow(), Content::Preview(state)
                if state.controller.sequence
                    != shrimply_preview_interaction_skia::controller::PointerSequence::Idle)
            {
                return;
            }
            self.ivars().secondary_preview_active.set(false);
            self.window().expect("canvas must be attached").makeFirstResponder(Some(self));
            let mut input = self.preview_input(event);
            input.button = shrimply_preview_provider_skia::PointerButton::Secondary;
            if self.preview_pointer_event(PointerEvent::Begin(input)) {
                self.ivars().secondary_preview_active.set(true);
            } else {
                self.preview_pointer_event(PointerEvent::End(input));
                self.open_context_menu(event);
            }
        }

        #[unsafe(method(rightMouseDragged:))]
        fn right_mouse_dragged(&self, event: &NSEvent) {
            if !self.ivars().secondary_preview_active.get() {
                return;
            }
            let mut input = self.preview_input(event);
            input.button = shrimply_preview_provider_skia::PointerButton::Secondary;
            self.preview_pointer_event(PointerEvent::Samples { input, samples: &[input.sample] });
        }

        #[unsafe(method(rightMouseUp:))]
        fn right_mouse_up(&self, event: &NSEvent) {
            if !self.ivars().secondary_preview_active.replace(false) {
                return;
            }
            let mut input = self.preview_input(event);
            input.button = shrimply_preview_provider_skia::PointerButton::Secondary;
            self.preview_pointer_event(PointerEvent::Samples { input, samples: &[input.sample] });
            self.preview_pointer_event(PointerEvent::End(input));
        }

        #[unsafe(method(otherMouseDown:))]
        fn other_mouse_down(&self, event: &NSEvent) {
            if event.buttonNumber() == MIDDLE_MOUSE_BUTTON && matches!(&*self.ivars().content.borrow(), Content::Preview(_)) {
                if !shrimply_editor_state::preferences::snapshot(&self.ivars().session.preferences).preview_zoom_pan_enabled { return; }
                self.preview_pointer_event(PointerEvent::Begin(self.preview_input(event)));
                return;
            }
            if event.buttonNumber() == MIDDLE_MOUSE_BUTTON && let Content::Timeline(scene) = &mut *self.ivars().content.borrow_mut() {
                self.window().expect("canvas must be attached").makeFirstResponder(Some(self));
                let point = self.point(event);
                scene.begin_pan(point);
                if scene.pointer_state().capture_requested {
                    let result = objc2_core_graphics::CGAssociateMouseAndMouseCursorPosition(false);
                    assert_eq!(result, objc2_core_graphics::CGError::Success, "could not capture the macOS pointer for timeline pan");
                    scene.begin_relative_pointer(point, closed_hand_software_cursor());
                    self.ivars().relative_pan_active.set(true);
                    objc2_app_kit::NSCursor::hide();
                } else {
                    objc2_app_kit::NSCursor::closedHandCursor().set();
                }
            }
        }

        #[unsafe(method(otherMouseDragged:))]
        fn other_mouse_dragged(&self, event: &NSEvent) {
            if event.buttonNumber() == MIDDLE_MOUSE_BUTTON && matches!(&*self.ivars().content.borrow(), Content::Preview(_)) {
                if !shrimply_editor_state::preferences::snapshot(&self.ivars().session.preferences).preview_zoom_pan_enabled { return; }
                let input = self.preview_input(event);
                self.preview_pointer_event(PointerEvent::Samples { input, samples: &[input.sample] });
                return;
            }
            if event.buttonNumber() == MIDDLE_MOUSE_BUTTON && let Content::Timeline(scene) = &mut *self.ivars().content.borrow_mut() {
                if self.ivars().relative_pan_active.get() {
                    scene.event(shrimply_timeline_skia::scene::Event::RelativeMotion {
                        delta: glam::Vec2::new(event.deltaX() as f32, -event.deltaY() as f32),
                    });
                } else {
                    scene.pan_to(self.point(event));
                    objc2_app_kit::NSCursor::closedHandCursor().set();
                }
            }
        }

        #[unsafe(method(otherMouseUp:))]
        fn other_mouse_up(&self, event: &NSEvent) {
            if event.buttonNumber() == MIDDLE_MOUSE_BUTTON && matches!(&*self.ivars().content.borrow(), Content::Preview(_)) {
                if !shrimply_editor_state::preferences::snapshot(&self.ivars().session.preferences).preview_zoom_pan_enabled { return; }
                self.preview_pointer_event(PointerEvent::End(self.preview_input(event)));
                return;
            }
            if event.buttonNumber() == MIDDLE_MOUSE_BUTTON && let Content::Timeline(scene) = &mut *self.ivars().content.borrow_mut() {
                let point = self.release_relative_pan(scene).unwrap_or_else(|| self.point(event));
                scene.end_pan(point);
                Self::set_timeline_cursor(scene.pointer_cursor());
            }
        }

        #[unsafe(method(magnifyWithEvent:))]
        fn magnify(&self, event: &NSEvent) {
            if let Content::Timeline(scene) = &mut *self.ivars().content.borrow_mut() {
                scene.magnify(self.point(event), event.magnification());
            }
        }

        #[unsafe(method(isFlipped))]
        fn flipped(&self) -> bool { true }

        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool { true }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            let control = event.modifierFlags().contains(NSEventModifierFlags::Control);
            self.ivars().suppress_primary.set(control);
            if control {
                if matches!(&*self.ivars().content.borrow(), Content::Preview(state)
                    if state.controller.sequence
                        != shrimply_preview_interaction_skia::controller::PointerSequence::Idle)
                {
                    return;
                }
                self.ivars().secondary_preview_active.set(false);
                let mut input = self.preview_input(event);
                input.button = shrimply_preview_provider_skia::PointerButton::Secondary;
                if self.preview_pointer_event(PointerEvent::Begin(input)) {
                    self.ivars().secondary_preview_active.set(true);
                } else {
                    self.preview_pointer_event(PointerEvent::End(input));
                    self.open_context_menu(event);
                }
                return;
            }
            self.window().expect("canvas must be attached").makeFirstResponder(Some(self));
            if let Content::Timeline(scene) = &mut *self.ivars().content.borrow_mut() {
                let modifiers = event.modifierFlags();
                let toggle = modifiers.contains(NSEventModifierFlags::Command);
                let extend = modifiers.contains(NSEventModifierFlags::Shift);
                if event.clickCount() == 2 { scene.double_click_down(self.point(event), toggle, extend); }
                else { scene.pointer_down(self.point(event), toggle, extend); }
                Self::set_timeline_cursor(scene.pointer_cursor());
            }
            self.preview_pointer_down(self.point(event));
            self.preview_pointer_event(PointerEvent::Begin(self.preview_input(event)));
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            if self.ivars().suppress_primary.get() {
                if self.ivars().secondary_preview_active.get() {
                    let mut input = self.preview_input(event);
                    input.button = shrimply_preview_provider_skia::PointerButton::Secondary;
                    self.preview_pointer_event(PointerEvent::Samples { input, samples: &[input.sample] });
                }
                return;
            }
            if let Content::Timeline(scene) = &mut *self.ivars().content.borrow_mut() {
                let modifiers = event.modifierFlags();
                scene.pointer_dragged(
                    self.point(event),
                    modifiers.contains(NSEventModifierFlags::Command),
                    modifiers.contains(NSEventModifierFlags::Shift),
                );
                Self::set_timeline_cursor(scene.pointer_cursor());
            }
            self.preview_pointer_move(self.point(event));
            let input = self.preview_input(event);
            self.preview_pointer_event(PointerEvent::Samples { input, samples: &[input.sample] });
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            if self.ivars().suppress_primary.replace(false) {
                if self.ivars().secondary_preview_active.replace(false) {
                    let mut input = self.preview_input(event);
                    input.button = shrimply_preview_provider_skia::PointerButton::Secondary;
                    self.preview_pointer_event(PointerEvent::Samples { input, samples: &[input.sample] });
                    self.preview_pointer_event(PointerEvent::End(input));
                }
                return;
            }
            let point = self.point(event);
            let result = {
                let mut content = self.ivars().content.borrow_mut();
                if let Content::Timeline(scene) = &mut *content {
                    let modifiers = event.modifierFlags();
                    scene.pointer_up(
                        point,
                        modifiers.contains(NSEventModifierFlags::Command),
                        modifiers.contains(NSEventModifierFlags::Shift),
                    )
                } else {
                    Ok(None)
                }
            };
            match result.and_then(|action| {
                if let Some(action) = action { self.activate_track_button(action, point) } else { Ok(()) }
            }) {
                Ok(()) => {},
                Err(error) => self.show_error(&error),
            }
            if let Content::Timeline(scene) = &*self.ivars().content.borrow() {
                Self::set_timeline_cursor(scene.pointer_cursor());
            }
            self.update_tracking();
            if let Err(error) = self.preview_pointer_up(point) { self.show_error(&error); }
            let input = self.preview_input(event);
            self.preview_pointer_event(PointerEvent::Samples { input, samples: &[input.sample] });
            self.preview_pointer_event(PointerEvent::End(input));
        }

        #[unsafe(method(scrollWheel:))]
        fn scroll(&self, event: &NSEvent) {
            if let Content::Timeline(scene) = &mut *self.ivars().content.borrow_mut() {
                use shrimply_timeline_skia::{metrics::SCROLL_PIXELS_PER_STEP, view::TimelineScrollInput};
                let (input, step) = if event.hasPreciseScrollingDeltas() {
                    (TimelineScrollInput::Surface, 1.0)
                } else {
                    (TimelineScrollInput::Wheel, SCROLL_PIXELS_PER_STEP)
                };
                scene.scroll(self.point(event), glam::Vec2::new((event.scrollingDeltaX() * step) as f32, (event.scrollingDeltaY() * step) as f32), event.modifierFlags().contains(NSEventModifierFlags::Control), input);
                return;
            }
            let scale = if event.hasPreciseScrollingDeltas() {
                shrimply_preview_provider_skia::math::SCROLL_PIXELS_PER_STEP
            } else { 1.0 };
            let enabled = shrimply_editor_state::preferences::snapshot(&self.ivars().session.preferences).preview_zoom_pan_enabled;
            let scale = if enabled { scale } else { 1.0 };
            self.preview_pointer_event(PointerEvent::Scroll {
                input: self.preview_input(event),
                delta: glam::Vec2::new(-event.scrollingDeltaX() as f32 / scale, -event.scrollingDeltaY() as f32 / scale),
            });
        }

        #[unsafe(method(keyUp:))]
        fn key_up(&self, event: &NSEvent) {
            if !self.preview_keyboard(event, KeyState::Released) {
                unsafe { let _: () = msg_send![super(self), keyUp: event]; }
            }
        }

        #[unsafe(method(flagsChanged:))]
        fn flags_changed(&self, event: &NSEvent) {
            if let Content::Timeline(scene) = &mut *self.ivars().content.borrow_mut() {
                let modifiers = event.modifierFlags();
                scene.event(shrimply_timeline_skia::scene::Event::Modifiers(
                    shrimply_timeline_skia::scene::TimelineModifiers {
                        ctrl: modifiers.contains(NSEventModifierFlags::Command),
                        shift: modifiers.contains(NSEventModifierFlags::Shift),
                    },
                ));
            }
            self.preview_modifiers_changed(event);
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            let key = event.charactersIgnoringModifiers().and_then(|text| text.to_string().chars().next());
            // GTK's playback shortcuts run in capture before preview providers.
            match key {
                Some(' ') => {
                    shrimply_editor_state::player_state::toggle_playing(&self.ivars().session.player_state);
                    return;
                }
                Some('l' | 'L') => {
                    shrimply_editor_state::player_state::step_playback_speed_forward(&self.ivars().session.player_state);
                    return;
                }
                _ => {}
            }
            if self.preview_keyboard(event, KeyState::Pressed) { return; }

            let modifiers = event.modifierFlags();
            if !modifiers.intersects(NSEventModifierFlags::Option | NSEventModifierFlags::Control)
                && let Some(action) = key.and_then(|key| shrimply_timeline_skia::scene::KeyAction::from_key(
                    key, modifiers.contains(NSEventModifierFlags::Command), modifiers.contains(NSEventModifierFlags::Shift)))
            {
                let result = {
                    let mut content = self.ivars().content.borrow_mut();
                    if let Content::Timeline(scene) = &mut *content { Some(scene.key_action(action)) } else { None }
                };
                if let Some(result) = result {
                    if let Err(error) = result.and_then(|request| request.map_or(Ok(()), |request| self.handle_context_request(request))) {
                        self.show_error(&error);
                    }
                    return;
                }
            }
            unsafe { let _: () = msg_send![super(self), keyDown: event]; }
        }
    }
);

impl CanvasView {
    pub(super) fn register_paint_tool(
        &self,
        tool: PaintTool,
        button: Retained<objc2_app_kit::NSButton>,
    ) {
        self.ivars().paint_tools.borrow_mut().push((tool, button));
        self.sync_paint_tools();
    }

    fn sync_paint_tools(&self) {
        use shrimply_paint_edit_skia::{PAINT_PREVIEW_STATE, PaintPreviewMode, PaintPreviewState};
        let tools = self.ivars().paint_tools.borrow();
        if tools.is_empty() {
            return;
        }
        let visible = {
            let project = self.ivars().session.project.borrow();
            shrimply_timeline_skia::selection_state::focused_video_address(
                &self.ivars().session.selection_state,
                &project,
            )
            .and_then(|address| project.video_item(&address))
            .is_some_and(|item| {
                matches!(
                    item.content,
                    shrimply_project_document::project::VideoItemContent::Paint(_)
                )
            })
        };
        let content = self.ivars().content.borrow();
        let state = match &*content {
            Content::Preview(preview) => preview
                .controller
                .extension::<PaintPreviewState>(PAINT_PREVIEW_STATE),
            _ => None,
        };
        for (tool, button) in tools.iter() {
            if button.isHidden() == visible {
                button.setHidden(!visible);
            }
            let selected = state.is_some_and(|state| match tool {
                PaintTool::Pen => {
                    state.mode == PaintPreviewMode::Pen && !state.eraser && !state.adjusting
                }
                PaintTool::Fill => state.mode == PaintPreviewMode::Fill,
                PaintTool::Eraser => state.eraser,
                PaintTool::Adjust => state.adjusting,
                PaintTool::Transform => state.mode == PaintPreviewMode::StrokeTransform,
                PaintTool::OnionPrevious => state.onion_previous,
                PaintTool::OnionNext => state.onion_next,
                PaintTool::Smaller | PaintTool::Larger | PaintTool::Palette => false,
            });
            super::layout::set_toggle_selected(button, selected, super::layout::ToggleStyle::Solid);
        }
    }

    pub(super) fn register_tool(
        &self,
        tool: super::timeline::Tool,
        button: Retained<objc2_app_kit::NSButton>,
    ) {
        self.ivars().tools.borrow_mut().push((tool, button));
        self.sync_tools();
    }

    fn sync_tools(&self) {
        let tools = self.ivars().tools.borrow();
        if tools.is_empty() {
            return;
        }
        let state =
            shrimply_timeline_skia::TimelineTools::new(self.ivars().session.preferences.clone())
                .state();
        for (tool, button) in tools.iter() {
            super::layout::set_toggle_selected(
                button,
                tool.selected(state),
                super::layout::ToggleStyle::Grouped,
            );
        }
    }
    fn update_pointer(&self, point: glam::Vec2) {
        if let Content::Timeline(scene) = &mut *self.ivars().content.borrow_mut() {
            scene.pointer_moved(point);
            Self::set_timeline_cursor(scene.pointer_cursor());
        }
        self.preview_pointer_move(point);
    }

    fn set_timeline_cursor(cursor: shrimply_timeline_skia::view::TimelineCursor) {
        use objc2_app_kit::{NSCursor, NSCursorFrameResizeDirections, NSCursorFrameResizePosition};
        use shrimply_timeline_skia::view::TimelineCursor;
        match cursor {
            TimelineCursor::Default => NSCursor::arrowCursor().set(),
            TimelineCursor::ResizeStart => NSCursor::frameResizeCursorFromPosition_inDirections(
                NSCursorFrameResizePosition::Left,
                NSCursorFrameResizeDirections::All,
            )
            .set(),
            TimelineCursor::ResizeEnd => NSCursor::frameResizeCursorFromPosition_inDirections(
                NSCursorFrameResizePosition::Right,
                NSCursorFrameResizeDirections::All,
            )
            .set(),
            TimelineCursor::ResizeHorizontal => {
                NSCursor::columnResizeCursorInDirections(
                    objc2_app_kit::NSHorizontalDirections::All,
                )
                .set();
            }
            TimelineCursor::Crosshair => NSCursor::crosshairCursor().set(),
        }
    }

    fn update_tracking(&self) {
        if let Some(area) = self.ivars().tracking_area.borrow_mut().take() {
            self.removeTrackingArea(&area);
        }
        if matches!(
            *self.ivars().content.borrow(),
            Content::Timeline(_) | Content::Preview(_)
        ) {
            let area = unsafe {
                NSTrackingArea::initWithRect_options_owner_userInfo(
                    NSTrackingArea::alloc(),
                    NSRect::ZERO,
                    NSTrackingAreaOptions::MouseEnteredAndExited
                        | NSTrackingAreaOptions::MouseMoved
                        | NSTrackingAreaOptions::ActiveInKeyWindow
                        | NSTrackingAreaOptions::InVisibleRect
                        | NSTrackingAreaOptions::EnabledDuringMouseDrag,
                    Some(self),
                    None,
                )
            };
            self.addTrackingArea(&area);
            self.ivars().tracking_area.replace(Some(area));
            if let Some(window) = self.window() {
                window.setAcceptsMouseMovedEvents(true);
                let point =
                    self.convertPoint_fromView(window.mouseLocationOutsideOfEventStream(), None);
                if window.isKeyWindow()
                    && objc2_foundation::NSMouseInRect(point, self.visibleRect(), true)
                {
                    self.update_pointer(glam::Vec2::new(point.x as f32, point.y as f32));
                } else if let Content::Timeline(scene) = &mut *self.ivars().content.borrow_mut() {
                    scene.pointer_exited();
                }
            }
        }
    }

    fn show_error(&self, error: &str) {
        super::error_alert::show(self.mtm(), error);
    }

    fn drop_payload(pasteboard: &NSPasteboard) -> Option<DropPayload> {
        let mask_type = NSString::from_str(MASK_PASTEBOARD_TYPE);
        if let Some(mask) = pasteboard.stringForType(&mask_type) {
            return Some(DropPayload::Mask(mask.to_string()));
        }
        let files = super::media::file_urls(pasteboard);
        if !files.is_empty() {
            return Some(DropPayload::Files(files));
        }
        if pasteboard
            .dataForType(unsafe { NSPasteboardTypePNG })
            .is_some()
            || pasteboard
                .dataForType(unsafe { NSPasteboardTypeTIFF })
                .is_some()
        {
            return Some(DropPayload::Image);
        }
        if let Some(url) = pasteboard.stringForType(unsafe { NSPasteboardTypeURL }) {
            return Some(DropPayload::ImageUrl(url.to_string()));
        }
        pasteboard
            .stringForType(unsafe { NSPasteboardTypeString })
            .map(|text| {
                match shrimply_timeline_skia::external_content::classify_external_text(
                    text.to_string(),
                ) {
                    shrimply_timeline_skia::external_content::ExternalText::Text(text) => {
                        DropPayload::Text(text)
                    }
                    shrimply_timeline_skia::external_content::ExternalText::ImageUrl(url) => {
                        DropPayload::ImageUrl(url)
                    }
                }
            })
    }

    fn drag_operation(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
        if !sender
            .draggingSourceOperationMask()
            .contains(NSDragOperation::Copy)
        {
            self.clear_drop_preview();
            return NSDragOperation::None;
        }
        let pasteboard = sender.draggingPasteboard();
        let Some(payload) = Self::drop_payload(&pasteboard) else {
            self.clear_drop_preview();
            return NSDragOperation::None;
        };
        let point = self.convertPoint_fromView(sender.draggingLocation(), None);
        let point = glam::Vec2::new(point.x as f32, point.y as f32);
        let accepted = {
            let mut content = self.ivars().content.borrow_mut();
            let Content::Timeline(scene) = &mut *content else {
                return NSDragOperation::None;
            };
            match payload {
                DropPayload::Files(files) => {
                    let Ok(paths) = super::media::file_url_paths(&files) else {
                        return NSDragOperation::None;
                    };
                    let path = paths.first().expect("file URL list is not empty").clone();
                    if self
                        .ivars()
                        .drop_source
                        .borrow()
                        .as_ref()
                        .is_none_or(|(current, _)| current != &path)
                    {
                        self.ivars().drop_source.replace(Some((
                            path.clone(),
                            super::media::ScopedUrl::new(files[0].clone()),
                        )));
                    }
                    scene.update_external_files_preview(&paths, point)
                }
                DropPayload::Text(text) => scene.update_text_drop_preview(text, point),
                DropPayload::Mask(mask) => mask
                    .parse()
                    .is_ok_and(|modifier_id| scene.mask_drop_target(modifier_id, point)),
                DropPayload::Image | DropPayload::ImageUrl(_) => {
                    scene.clear_drop_preview();
                    scene.external_drop_target(point)
                }
            }
        };
        if accepted {
            NSDragOperation::Copy
        } else {
            self.clear_drop_preview();
            NSDragOperation::None
        }
    }

    fn clear_drop_preview(&self) {
        if let Content::Timeline(scene) = &mut *self.ivars().content.borrow_mut() {
            scene.clear_drop_preview();
        }
        self.ivars().drop_source.replace(None);
    }

    fn perform_file_drop(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> bool {
        if self.drag_operation(sender) != NSDragOperation::Copy {
            return false;
        }
        let pasteboard = sender.draggingPasteboard();
        let Some(payload) = Self::drop_payload(&pasteboard) else {
            self.clear_drop_preview();
            return false;
        };
        let point = self.convertPoint_fromView(sender.draggingLocation(), None);
        let point = glam::Vec2::new(point.x as f32, point.y as f32);
        let result = (|| match payload {
            DropPayload::Files(urls) => self.perform_external_file_urls(urls, Some(point)),
            payload => {
                let content = match payload {
                    DropPayload::Image => super::media::clipboard_image_path(&pasteboard)?
                        .map(|path| {
                            shrimply_timeline_skia::external_content::ExternalDrop::Files(vec![
                                path,
                            ])
                        })
                        .ok_or_else(|| "dragged image has no readable image data".to_string()),
                    DropPayload::ImageUrl(url) => {
                        Ok(shrimply_timeline_skia::external_content::ExternalDrop::ImageUrl(url))
                    }
                    DropPayload::Text(text) => {
                        Ok(shrimply_timeline_skia::external_content::ExternalDrop::Text(text))
                    }
                    DropPayload::Mask(mask) => {
                        Ok(shrimply_timeline_skia::external_content::ExternalDrop::Mask(mask))
                    }
                    DropPayload::Files(_) => unreachable!("file payload handled separately"),
                };
                content.and_then(|content| self.perform_external_drop(content, Some(point)))
            }
        })();
        self.clear_drop_preview();
        if let Err(error) = result {
            self.show_error(&error);
            return false;
        }
        true
    }

    fn perform_external_drop(
        &self,
        content: shrimply_timeline_skia::external_content::ExternalDrop,
        point: Option<glam::Vec2>,
    ) -> Result<(), String> {
        let mut canvas = self.ivars().content.borrow_mut();
        let Content::Timeline(scene) = &mut *canvas else {
            return Err("timeline drop reached a non-timeline canvas".into());
        };
        let action = scene.perform_external_drop(content, point)?;
        drop(canvas);
        match action {
            shrimply_timeline_skia::external_content::ExternalDropAction::Complete => Ok(()),
            shrimply_timeline_skia::external_content::ExternalDropAction::Importing(_) => Ok(()),
            shrimply_timeline_skia::external_content::ExternalDropAction::ConfirmRemux {
                paths,
                batch,
            } => {
                if !self.confirm_remux_prompt() {
                    return Ok(());
                }
                let mut content = self.ivars().content.borrow_mut();
                let Content::Timeline(scene) = &mut *content else {
                    return Err("timeline closed while confirming media remux".into());
                };
                scene.begin_external_remux(paths, point, batch)
            }
        }
    }

    fn perform_external_file_urls(
        &self,
        urls: Vec<Retained<NSURL>>,
        point: Option<glam::Vec2>,
    ) -> Result<(), String> {
        let (paths, scopes) = super::media::scoped_file_urls(urls)?;
        let action = {
            let mut content = self.ivars().content.borrow_mut();
            let Content::Timeline(scene) = &mut *content else {
                return Err("timeline drop reached a non-timeline canvas".into());
            };
            scene.perform_external_drop(
                shrimply_timeline_skia::external_content::ExternalDrop::Files(paths),
                point,
            )?
        };
        let batch = match action {
            shrimply_timeline_skia::external_content::ExternalDropAction::Importing(batch) => batch,
            shrimply_timeline_skia::external_content::ExternalDropAction::ConfirmRemux {
                paths,
                batch,
            } => {
                if !self.confirm_remux_prompt() {
                    return Ok(());
                }
                let mut content = self.ivars().content.borrow_mut();
                let Content::Timeline(scene) = &mut *content else {
                    return Err("timeline closed while confirming media remux".into());
                };
                scene.begin_external_remux(paths, point, batch)?;
                batch
            }
            shrimply_timeline_skia::external_content::ExternalDropAction::Complete => {
                return Err("file import completed without an asynchronous operation".into());
            }
        };
        self.ivars()
            .imports
            .borrow_mut()
            .retain_pending(batch, scopes);
        Ok(())
    }

    fn confirm_remux_prompt(&self) -> bool {
        let alert = objc2_app_kit::NSAlert::new(self.mtm());
        alert.setMessageText(&NSString::from_str("Remux MKV/WebM to MP4?"));
        alert.setInformativeText(&NSString::from_str(
            "MP4 is the supported timeline format. The source files will be kept.",
        ));
        alert.addButtonWithTitle(&NSString::from_str("Remux"));
        alert.addButtonWithTitle(&NSString::from_str("Cancel"));
        alert.runModal() == objc2_app_kit::NSAlertFirstButtonReturn
    }

    fn point(&self, event: &NSEvent) -> glam::Vec2 {
        let point = self.convertPoint_fromView(event.locationInWindow(), None);
        glam::Vec2::new(point.x as f32, point.y as f32)
    }

    pub fn render(&self) -> Result<(), String> {
        let _timing = shrimply_process_reporting::diagnostics::timing(
            match &*self.ivars().content.borrow() {
                Content::Timeline(_) => "Timeline lifecycle total",
                Content::Preview(_) => "Preview lifecycle total",
                Content::Meter(_) => "Meter lifecycle total",
            },
        );
        self.sync_tools();
        self.sync_paint_tools();
        self.poll_audio_export()?;
        self.poll_video_export()?;
        self.poll_frame_capture()?;
        self.poll_caption_speech_probe()?;
        self.poll_transcription_probe()?;
        self.update_screen_recording()?;
        let size = self.bounds().size;
        let pointer_active = self.window().is_some_and(|window| window.isKeyWindow())
            && !self.isHiddenOrHasHiddenAncestor();
        if !pointer_active && self.ivars().pointer_active.replace(pointer_active) {
            self.teardown_preview_pointer();
            if let Content::Timeline(scene) = &mut *self.ivars().content.borrow_mut() {
                self.release_relative_pan(scene);
                scene.pointer_exited();
                scene.pointer_cancelled();
            }
        }
        self.ivars().pointer_active.set(pointer_active);
        if self.window().is_none()
            || self.isHiddenOrHasHiddenAncestor()
            || size.width <= 0.0
            || size.height <= 0.0
        {
            return Ok(());
        }
        self.refresh_live_preview();
        self.poll_preview()?;
        {
            let _timing =
                shrimply_process_reporting::diagnostics::timing("Preview provider preparation");
            self.prepare_preview()?;
        }
        let scale = self.window().expect("attached canvas").backingScaleFactor();
        let redraw = match &mut *self.ivars().content.borrow_mut() {
            Content::Timeline(scene) => {
                scene.needs_redraw(glam::Vec2::new(size.width as f32, size.height as f32))
            }
            Content::Meter(meter) => meter.update(
                self.ivars().session.audio_levels.take_peaks(),
                std::time::Instant::now(),
            ),
            Content::Preview(preview) => std::mem::take(&mut preview.controller.frame_pending),
        };
        let mut renderer = self.ivars().renderer.borrow_mut();
        let resized = renderer.layer().contentsScale() != scale
            || renderer.layer().drawableSize()
                != NSSize::new((size.width * scale).ceil(), (size.height * scale).ceil());
        if renderer.layer().contentsScale() != scale {
            renderer.layer().setContentsScale(scale);
        }
        let drawable_size = NSSize::new((size.width * scale).ceil(), (size.height * scale).ceil());
        if renderer.layer().drawableSize() != drawable_size {
            renderer.layer().setDrawableSize(drawable_size);
        }
        let mut result = Ok(());
        let surface_label = match &*self.ivars().content.borrow() {
            Content::Timeline(_) => "Timeline",
            Content::Preview(_) => "Preview",
            Content::Meter(_) => "Meter",
        };
        if matches!(&*self.ivars().content.borrow(), Content::Preview(_)) {
            if redraw {
                shrimply_process_reporting::diagnostics::count(
                    "Preview redraw / controller request",
                );
            }
            if resized {
                shrimply_process_reporting::diagnostics::count("Preview redraw / resize or scale");
            }
            if self.ivars().surface_dirty.get() {
                shrimply_process_reporting::diagnostics::count(
                    "Preview redraw / invalidated surface",
                );
            }
        }
        if redraw || resized {
            self.ivars().surface_dirty.set(true);
        }
        if self.ivars().surface_dirty.get() {
            let _timing = shrimply_process_reporting::diagnostics::timing(
                match &*self.ivars().content.borrow() {
                    Content::Timeline(_) => "Timeline UI surface submission",
                    Content::Preview(_) => "Preview UI surface submission",
                    Content::Meter(_) => "Meter UI surface submission",
                },
            );
            renderer.draw(surface_label, |canvas| {
                self.ivars().surface_dirty.set(false);
                canvas.clear(shrimply_cross_ui_theme::current().view_bg);
                canvas.scale((scale as f32, scale as f32));
                match &mut *self.ivars().content.borrow_mut() {
                    Content::Timeline(scene) => scene.draw(
                        canvas,
                        glam::Vec2::new(size.width as f32, size.height as f32),
                    ),
                    Content::Meter(meter) => {
                        meter.draw(canvas, size.width as f32, size.height as f32);
                    }
                    Content::Preview(preview) => {
                        let player = shrimply_editor_state::player_state::snapshot(
                            &self.ivars().session.player_state,
                        );
                        let project = self.ivars().session.project.borrow();
                        let frame = project.canvas_size;
                        let prefs = shrimply_editor_state::preferences::snapshot(
                            &self.ivars().session.preferences,
                        );
                        preview.sync_guides(&project.preview_guides, prefs.preview_guides_visible);
                        let viewport = shrimply_preview_interaction_skia::guides::viewport(
                            glam::IVec2::new(size.width as i32, size.height as i32),
                            frame,
                            prefs.preview_padding_px,
                            preview.guides_visible,
                            preview.fullscreen,
                        );
                        if !prefs.preview_zoom_pan_enabled {
                            if preview.navigation.active() {
                                objc2_app_kit::NSCursor::arrowCursor().set();
                            }
                            preview.navigation.reset();
                        }
                        let bounds = shrimply_preview_interaction_skia::guides::bounds(
                            glam::vec2(size.width as f32, size.height as f32),
                            prefs.preview_padding_px,
                            preview.guides_visible,
                            preview.fullscreen,
                        );
                        let viewport = preview.navigation.viewport(viewport, bounds);
                        preview.viewport = Some(viewport);
                        let content = viewport.content_rect;
                        let clip_rect = preview
                            .navigation
                            .clip_rect(bounds, glam::vec2(size.width as f32, size.height as f32));
                        shrimply_preview_provider_skia::canvas::draw_background(
                            canvas,
                            shrimply_preview_provider_skia::canvas::Appearance {
                                content_rect: shrimply_preview_provider_skia::Rect::from_min_max(
                                    content.min.max(clip_rect.min),
                                    content.max.min(clip_rect.max),
                                ),
                                background: shrimply_cross_ui_theme::current().view_bg,
                                shadow_size: prefs.preview_shadow_size_px,
                                pixel_scale: scale as f32,
                            },
                        );
                        canvas.save();
                        canvas.clip_rect(skia_safe::Rect::from(clip_rect), None, false);
                        canvas.translate((content.min.x, content.min.y));
                        canvas.scale((
                            content.width() / frame.width as f32,
                            content.height() / frame.height as f32,
                        ));
                        canvas.clip_rect(
                            skia_safe::Rect::from_wh(frame.width as f32, frame.height as f32),
                            None,
                            false,
                        );
                        use shrimply_editor_state::preferences::{
                            PreviewDownsampleMethod, PreviewUpsampleMethod,
                        };
                        use skia_safe::{FilterMode, MipmapMode, SamplingOptions};
                        let downsampling = content.width() * (scale as f32) < frame.width as f32
                            || content.height() * (scale as f32) < frame.height as f32;
                        let sampling = if downsampling {
                            match prefs.preview_downsample_method {
                                PreviewDownsampleMethod::Nearest => FilterMode::Nearest.into(),
                                PreviewDownsampleMethod::Bilinear => FilterMode::Linear.into(),
                                PreviewDownsampleMethod::Trilinear => {
                                    SamplingOptions::new(FilterMode::Linear, MipmapMode::Linear)
                                }
                            }
                        } else {
                            match prefs.preview_upsample_method {
                                PreviewUpsampleMethod::Nearest => FilterMode::Nearest.into(),
                                PreviewUpsampleMethod::Bilinear => FilterMode::Linear.into(),
                            }
                        };
                        {
                            let _timing = shrimply_process_reporting::diagnostics::timing(
                                "Preview paint / image and mipmaps",
                            );
                            result = preview.renderer.draw(canvas, sampling);
                        }
                        canvas.restore();
                        let focused_caption =
                            shrimply_timeline_skia::selection_state::focused_item_address(
                                &self.ivars().session.selection_state,
                                &project,
                            )
                            .filter(|address| project.caption_item(address).is_some());
                        let captions_timing = shrimply_process_reporting::diagnostics::timing(
                            "Preview paint / captions and guides",
                        );
                        preview::captions::draw(
                            canvas,
                            preview,
                            &project,
                            player.position,
                            preview::captions::appearance(
                                size,
                                &prefs,
                                preview.caption_bottom_inset,
                            ),
                            focused_caption.as_ref(),
                        );
                        preview::draw_guides(canvas, preview, &project, size);
                        drop(captions_timing);
                        let _timing = shrimply_process_reporting::diagnostics::timing(
                            "Preview paint / interaction overlay",
                        );
                        canvas.save();
                        canvas.clip_rect(skia_safe::Rect::from(clip_rect), None, false);
                        if let Some(retiring) = preview.controller.retiring_provider.as_mut() {
                            retiring.context.viewport = viewport;
                        }
                        preview.controller.draw(canvas, &preview.expressions);
                        canvas.restore();
                    }
                }
            });
        } else {
            shrimply_process_reporting::diagnostics::count(match &*self.ivars().content.borrow() {
                Content::Timeline(_) => "Timeline / skipped unchanged surface",
                Content::Preview(_) => "Preview / skipped unchanged surface",
                Content::Meter(_) => "Meter / skipped unchanged surface",
            });
        }
        drop(renderer);
        self.update_screen_recording()?;
        let (external_error, external_imports, caption_speech_updates, transcription_updates) = {
            let mut content = self.ivars().content.borrow_mut();
            match &mut *content {
                Content::Timeline(scene) => {
                    let mut imports = Vec::new();
                    while let Some(event) = scene.take_external_import_event() {
                        imports.push(event);
                    }
                    let mut transcription = Vec::new();
                    while let Some(update) = scene.take_transcription_update() {
                        transcription.push(update);
                    }
                    let mut caption_speech = Vec::new();
                    while let Some(update) = scene.take_caption_speech_update() {
                        caption_speech.push(update);
                    }
                    (scene.take_error(), imports, caption_speech, transcription)
                }
                Content::Preview(_) | Content::Meter(_) => {
                    (None, Vec::new(), Vec::new(), Vec::new())
                }
            }
        };
        for event in external_imports {
            self.ivars().imports.borrow_mut().finish_external(event);
        }
        if let Some(error) = external_error {
            self.show_error(&error);
        }
        for update in caption_speech_updates {
            if let Err(error) = self.handle_caption_speech_update(update) {
                self.show_error(&error);
            }
        }
        for update in transcription_updates {
            if let Err(error) = self.handle_transcription_update(update) {
                self.show_error(&error);
            }
        }
        result
    }
}

impl CanvasView {
    pub fn suspend_timeline(&self) {
        if let Content::Timeline(scene) = &mut *self.ivars().content.borrow_mut() {
            self.release_relative_pan(scene);
            scene.suspend();
            while let Some(event) = scene.take_external_import_event() {
                self.ivars().imports.borrow_mut().finish_external(event);
            }
        }
        if let Err(error) = self.update_screen_recording() {
            self.show_error(&error);
        }
    }

    fn release_relative_pan(
        &self,
        scene: &mut shrimply_timeline_skia::scene::Scene,
    ) -> Option<glam::Vec2> {
        if !self.ivars().relative_pan_active.replace(false) {
            return None;
        }
        let point = scene.end_relative_pointer();
        let result = objc2_core_graphics::CGAssociateMouseAndMouseCursorPosition(true);
        assert_eq!(
            result,
            objc2_core_graphics::CGError::Success,
            "could not restore the macOS pointer after timeline pan"
        );
        objc2_app_kit::NSCursor::unhide();
        point
    }
}

fn closed_hand_software_cursor() -> shrimply_components_skia::cursor::SoftwareCursor {
    let cursor = objc2_app_kit::NSCursor::closedHandCursor();
    let native = cursor.image();
    let encoded = native
        .TIFFRepresentation()
        .expect("macOS closed-hand cursor must provide an image");
    let image = skia_safe::Image::from_encoded(skia_safe::Data::new_copy(unsafe {
        encoded.as_bytes_unchecked()
    }))
    .expect("macOS closed-hand cursor image must be decodable by Skia");
    let hot_spot = cursor.hotSpot();
    let size = native.size();
    shrimply_components_skia::cursor::SoftwareCursor::from_image(
        image,
        glam::Vec2::new(hot_spot.x as f32, hot_spot.y as f32),
        glam::Vec2::new(size.width as f32, size.height as f32),
    )
}

pub fn new(
    content: Content,
    session: Rc<EditorSession>,
    imports: Rc<RefCell<super::media::Imports>>,
    mtm: MainThreadMarker,
) -> Retained<CanvasView> {
    let view = CanvasView::alloc(mtm).set_ivars(CanvasState {
        surface_dirty: Rc::new(Cell::new(true)),
        tools: RefCell::new(Vec::new()),
        paint_tools: RefCell::new(Vec::new()),
        tracking_area: RefCell::new(None),
        menu_choice: Cell::new(None),
        context_controls: RefCell::new(Vec::new()),
        context_error: RefCell::new(None),
        suppress_primary: Cell::new(false),
        secondary_preview_active: Cell::new(false),
        relative_pan_active: Cell::new(false),
        pointer_active: Cell::new(true),
        audio_export: RefCell::new(None),
        video_export: RefCell::new(None),
        caption_speech_probe: RefCell::new(None),
        caption_speech_alert: RefCell::new(None),
        transcription_probe: RefCell::new(None),
        transcription_alert: RefCell::new(None),
        frame_capture: RefCell::new(None),
        screen_recording: RefCell::new(None),
        drop_source: RefCell::new(None),
        imports,
        renderer: RefCell::new(Renderer::default()),
        content: RefCell::new(content),
        session,
    });
    let view: Retained<CanvasView> = unsafe { msg_send![super(view), initWithFrame: NSRect::ZERO] };
    // Layer-hosting views are presented by the display link; drawRect is not invoked for a CAMetalLayer.
    view.setLayer(Some(view.ivars().renderer.borrow().layer()));
    view.setWantsLayer(true);
    if matches!(*view.ivars().content.borrow(), Content::Preview(_)) {
        view.connect_preview_redraw();
    }
    if matches!(*view.ivars().content.borrow(), Content::Timeline(_)) {
        let mask_type = NSString::from_str(MASK_PASTEBOARD_TYPE);
        view.registerForDraggedTypes(&NSArray::from_slice(&[
            unsafe { NSPasteboardTypeFileURL },
            unsafe { NSPasteboardTypeURL },
            unsafe { NSPasteboardTypeString },
            unsafe { NSPasteboardTypePNG },
            unsafe { NSPasteboardTypeTIFF },
            &mask_type,
        ]));
    }
    view
}
