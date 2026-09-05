use objc2::runtime::ProtocolObject;
use objc2::{AnyThread, DefinedClass, MainThreadOnly, define_class, msg_send, rc::Retained};
use objc2_app_kit::{
    NSDragOperation, NSDraggingDestination, NSDraggingInfo, NSEvent, NSEventModifierFlags,
    NSPasteboardTypeFileURL, NSTrackingArea, NSTrackingAreaOptions, NSView,
};
use objc2_foundation::{MainThreadMarker, NSRect, NSSize};
use objc2_foundation::{NSArray, NSObjectProtocol};
use shrimply_cross_ui_core::editor::EditorSession;
use shrimply_preview_core::{KeyState, PointerEvent};
use shrimply_skia_adw_core::audio_meter::AudioMeter;
use shrimply_skia_metal::Renderer;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

mod context_audio;
mod context_frame;
mod context_menu;
pub(super) mod preview;
mod track_actions;

pub enum Content {
    Timeline(Box<shrimply_timeline_core::scene::Scene>),
    Preview(Box<preview::State>),
    Meter(AudioMeter),
}

pub struct CanvasState {
    renderer: RefCell<Renderer>,
    content: RefCell<Content>,
    session: Rc<EditorSession>,
    imports: Rc<RefCell<super::media::Imports>>,
    tracking_area: RefCell<Option<Retained<NSTrackingArea>>>,
    menu_choice: Cell<Option<usize>>,
    context_controls: RefCell<Vec<shrimply_timeline_core::ContextMenuControl>>,
    context_error: RefCell<Option<String>>,
    suppress_primary: Cell<bool>,
    audio_export: RefCell<Option<context_audio::AudioExport>>,
    frame_capture: RefCell<Option<context_frame::FrameCapture>>,
    drop_source: RefCell<Option<(std::path::PathBuf, super::media::ScopedUrl)>>,
    tools: RefCell<Vec<(super::timeline::Tool, Retained<objc2_app_kit::NSButton>)>>,
}

const MIDDLE_MOUSE_BUTTON: isize = 2;

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
            tool.activate(&shrimply_timeline_core::TimelineTools::new(self.ivars().session.preferences.clone()));
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
            shrimply_state::preferences::set_preview_guides_visible(&self.ivars().session.preferences, sender.state() == objc2_app_kit::NSControlStateValueOn);
        }

        #[unsafe(method(rightMouseDown:))]
        fn right_mouse_down(&self, event: &NSEvent) {
            self.open_context_menu(event);
        }

        #[unsafe(method(otherMouseDown:))]
        fn other_mouse_down(&self, event: &NSEvent) {
            if event.buttonNumber() == MIDDLE_MOUSE_BUTTON && let Content::Timeline(scene) = &mut *self.ivars().content.borrow_mut() {
                self.window().expect("canvas must be attached").makeFirstResponder(Some(self));
                scene.begin_pan(self.point(event));
                objc2_app_kit::NSCursor::closedHandCursor().set();
            }
        }

        #[unsafe(method(otherMouseDragged:))]
        fn other_mouse_dragged(&self, event: &NSEvent) {
            if event.buttonNumber() == MIDDLE_MOUSE_BUTTON && let Content::Timeline(scene) = &mut *self.ivars().content.borrow_mut() {
                scene.pan_to(self.point(event));
                objc2_app_kit::NSCursor::closedHandCursor().set();
            }
        }

        #[unsafe(method(otherMouseUp:))]
        fn other_mouse_up(&self, event: &NSEvent) {
            if event.buttonNumber() == MIDDLE_MOUSE_BUTTON && let Content::Timeline(scene) = &mut *self.ivars().content.borrow_mut() {
                scene.end_pan(self.point(event));
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
                self.open_context_menu(event);
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
            if self.ivars().suppress_primary.get() { return; }
            if let Content::Timeline(scene) = &mut *self.ivars().content.borrow_mut() {
                scene.pointer_dragged(self.point(event));
                Self::set_timeline_cursor(scene.pointer_cursor());
            }
            self.preview_pointer_move(self.point(event));
            let input = self.preview_input(event);
            self.preview_pointer_event(PointerEvent::Samples { input, samples: &[input.sample] });
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            if self.ivars().suppress_primary.replace(false) { return; }
            let point = self.point(event);
            let result = {
                let mut content = self.ivars().content.borrow_mut();
                if let Content::Timeline(scene) = &mut *content { scene.pointer_up(point) } else { Ok(None) }
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
                let step = if event.hasPreciseScrollingDeltas() { 1.0 } else { shrimply_timeline_core::metrics::SCROLL_PIXELS_PER_STEP };
                scene.scroll(self.point(event), glam::Vec2::new((event.scrollingDeltaX() * step) as f32, (event.scrollingDeltaY() * step) as f32), event.modifierFlags().contains(NSEventModifierFlags::Control));
            }
            self.preview_pointer_event(PointerEvent::Scroll {
                input: self.preview_input(event),
                delta: glam::Vec2::new(-event.scrollingDeltaX() as f32, -event.scrollingDeltaY() as f32),
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
            self.preview_modifiers_changed(event);
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            let key = event.charactersIgnoringModifiers().and_then(|text| text.to_string().chars().next());
            // GTK's playback shortcuts run in capture before preview providers.
            match key {
                Some(' ') => {
                    shrimply_state::player_state::toggle_playing(&self.ivars().session.player_state);
                    return;
                }
                Some('l' | 'L') => {
                    shrimply_state::player_state::step_playback_speed_forward(&self.ivars().session.player_state);
                    return;
                }
                _ => {}
            }
            if self.preview_keyboard(event, KeyState::Pressed) { return; }

            let modifiers = event.modifierFlags();
            if !modifiers.intersects(NSEventModifierFlags::Option | NSEventModifierFlags::Control)
                && let Some(action) = key.and_then(|key| shrimply_timeline_core::scene::KeyAction::from_key(
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
    pub(super) fn register_tool(
        &self,
        tool: super::timeline::Tool,
        button: Retained<objc2_app_kit::NSButton>,
    ) {
        self.ivars().tools.borrow_mut().push((tool, button));
        self.sync_tools();
    }

    fn sync_tools(&self) {
        let state =
            shrimply_timeline_core::TimelineTools::new(self.ivars().session.preferences.clone())
                .state();
        for (tool, button) in self.ivars().tools.borrow().iter() {
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

    fn set_timeline_cursor(cursor: shrimply_timeline_core::view::TimelineCursor) {
        use objc2_app_kit::{NSCursor, NSCursorFrameResizeDirections, NSCursorFrameResizePosition};
        use shrimply_timeline_core::view::TimelineCursor;
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

    fn drag_operation(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
        if !sender
            .draggingSourceOperationMask()
            .contains(NSDragOperation::Copy)
        {
            self.clear_drop_preview();
            return NSDragOperation::None;
        }
        let Some(url) = super::media::file_urls(&sender.draggingPasteboard())
            .into_iter()
            .next()
        else {
            self.clear_drop_preview();
            return NSDragOperation::None;
        };
        let Some(path) = url.to_file_path() else {
            self.clear_drop_preview();
            return NSDragOperation::None;
        };
        if self
            .ivars()
            .drop_source
            .borrow()
            .as_ref()
            .is_none_or(|(current, _)| current != &path)
        {
            self.ivars()
                .drop_source
                .replace(Some((path.clone(), super::media::ScopedUrl::new(url))));
        }
        let point = self.convertPoint_fromView(sender.draggingLocation(), None);
        let accepted = {
            let mut content = self.ivars().content.borrow_mut();
            if let Content::Timeline(scene) = &mut *content {
                scene.update_drop_preview(path, glam::Vec2::new(point.x as f32, point.y as f32))
            } else {
                false
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
        let point = self.convertPoint_fromView(sender.draggingLocation(), None);
        let placement = {
            let content = self.ivars().content.borrow();
            let Content::Timeline(scene) = &*content else {
                return false;
            };
            shrimply_timeline_core::import_queue::Placement {
                start: shrimply_timeline_core::math::time_at_x(scene.view(), point.x),
                target: shrimply_timeline_core::items::NewItemTarget::AtY(
                    point.y.max(shrimply_timeline_core::metrics::RULER_HEIGHT)
                        + scene.view().scroll_y,
                ),
                collision: shrimply_timeline_core::TimelineTools::new(
                    self.ivars().session.preferences.clone(),
                )
                .state()
                .drag_collision,
            }
        };
        let result = self.ivars().imports.borrow_mut().enqueue(
            super::media::file_urls(&sender.draggingPasteboard()),
            &self.ivars().session,
            super::media::Destination::Timeline(placement),
        );
        self.clear_drop_preview();
        if let Err(error) = result {
            let alert = objc2_app_kit::NSAlert::new(self.mtm());
            alert.setInformativeText(&objc2_foundation::NSString::from_str(&error));
            alert.runModal();
            return false;
        }
        true
    }

    fn point(&self, event: &NSEvent) -> glam::Vec2 {
        let point = self.convertPoint_fromView(event.locationInWindow(), None);
        glam::Vec2::new(point.x as f32, point.y as f32)
    }

    pub fn render(&self) -> Result<(), String> {
        self.sync_tools();
        self.poll_audio_export()?;
        self.poll_frame_capture()?;
        let size = self.bounds().size;
        if self.window().is_none_or(|window| !window.isKeyWindow())
            || self.isHiddenOrHasHiddenAncestor()
        {
            self.cancel_preview_pointer();
        }
        if (self.window().is_none_or(|window| !window.isKeyWindow())
            || self.isHiddenOrHasHiddenAncestor())
            && let Content::Timeline(scene) = &mut *self.ivars().content.borrow_mut()
        {
            scene.pointer_exited();
            scene.pointer_cancelled();
        }
        if self.window().is_none()
            || self.isHiddenOrHasHiddenAncestor()
            || size.width <= 0.0
            || size.height <= 0.0
        {
            return Ok(());
        }
        self.refresh_live_preview();
        self.prepare_preview()?;
        let scale = self.window().expect("attached canvas").backingScaleFactor();
        let mut renderer = self.ivars().renderer.borrow_mut();
        renderer.layer().setContentsScale(scale);
        renderer.layer().setDrawableSize(NSSize::new(
            (size.width * scale).ceil(),
            (size.height * scale).ceil(),
        ));
        let mut result = Ok(());
        let mut manim_updates = Vec::new();
        renderer.draw(|canvas| {
            canvas.clear(shrimply_cross_ui_theme::current().view_bg);
            canvas.scale((scale as f32, scale as f32));
            match &mut *self.ivars().content.borrow_mut() {
                Content::Timeline(scene) => scene.draw(
                    canvas,
                    glam::Vec2::new(size.width as f32, size.height as f32),
                ),
                Content::Meter(meter) => {
                    meter.update(
                        self.ivars().session.audio_levels.take_peaks(),
                        std::time::Instant::now(),
                    );
                    meter.draw(canvas, size.width as f32, size.height as f32);
                }
                Content::Preview(preview) => {
                    let player =
                        shrimply_state::player_state::snapshot(&self.ivars().session.player_state);
                    preview
                        .renderer
                        .set_interaction(player.playing, player.scrubbing);
                    let project = self.ivars().session.project.borrow();
                    let frame = project.canvas_size;
                    let prefs =
                        shrimply_state::preferences::snapshot(&self.ivars().session.preferences);
                    preview.sync_guides(&project.preview_guides, prefs.preview_guides_visible);
                    let viewport = shrimply_preview_interaction_core::guides::viewport(
                        glam::IVec2::new(size.width as i32, size.height as i32),
                        frame,
                        prefs.preview_padding_px,
                        preview.guides_visible,
                        preview.fullscreen,
                    );
                    preview.viewport = Some(viewport);
                    let content = viewport.content_rect;
                    shrimply_preview_core::canvas::draw_background(
                        canvas,
                        shrimply_preview_core::canvas::Appearance {
                            content_rect: content,
                            background: shrimply_cross_ui_theme::current().view_bg,
                            shadow_size: prefs.preview_shadow_size_px,
                            pixel_scale: scale as f32,
                        },
                    );
                    canvas.save();
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
                    result = preview.renderer.draw(
                        canvas,
                        &project,
                        shrimply_state::player_state::current_time(
                            &self.ivars().session.player_state,
                        ),
                    );
                    manim_updates.extend(preview.renderer.take_manim_updates());
                    canvas.restore();
                    let focused_caption =
                        shrimply_timeline_core::selection_state::focused_item_address(
                            &self.ivars().session.selection_state,
                            &project,
                        )
                        .filter(|address| project.caption_item(address).is_some());
                    preview::captions::draw(
                        canvas,
                        preview,
                        &project,
                        player.position,
                        preview::captions::appearance(size, &prefs, preview.caption_bottom_inset),
                        focused_caption.as_ref(),
                    );
                    preview::draw_guides(canvas, preview, &project, size);
                    if let Some((id, revision, exclusion)) = preview.renderer.presented_frame()
                        && preview.presented_frame != Some(id)
                        && preview.controller.accept_base_frame(revision, exclusion)
                    {
                        preview.presented_frame = Some(id);
                        let (time, audio) = preview
                            .renderer
                            .presented_audio()
                            .expect("accepted frame has audio analysis");
                        preview.audio_analysis = Some((revision, time, audio.clone()));
                        preview.controller.context_invalidated = true;
                    }
                    preview.controller.draw(canvas, &preview.expressions);
                    preview.sync_loading(shrimply_project::project::scaled_time_delta(
                        project.frame_step(),
                        player.playback_speed,
                    ));
                }
            }
        });
        drop(renderer);
        for update in manim_updates {
            shrimply_state::manim_status::apply(
                &self.ivars().session.project,
                &self.ivars().session.player_state,
                update,
            );
        }
        result
    }
}

pub fn new(
    content: Content,
    session: Rc<EditorSession>,
    imports: Rc<RefCell<super::media::Imports>>,
    mtm: MainThreadMarker,
) -> Retained<CanvasView> {
    let view = CanvasView::alloc(mtm).set_ivars(CanvasState {
        tools: RefCell::new(Vec::new()),
        tracking_area: RefCell::new(None),
        menu_choice: Cell::new(None),
        context_controls: RefCell::new(Vec::new()),
        context_error: RefCell::new(None),
        suppress_primary: Cell::new(false),
        audio_export: RefCell::new(None),
        frame_capture: RefCell::new(None),
        drop_source: RefCell::new(None),
        imports,
        renderer: RefCell::new(Renderer::default()),
        content: RefCell::new(content),
        session,
    });
    let view: Retained<CanvasView> = unsafe { msg_send![super(view), initWithFrame: NSRect::ZERO] };
    // Layer-hosting views are presented explicitly by the frame timer; drawRect is not invoked for a CAMetalLayer.
    view.setLayer(Some(view.ivars().renderer.borrow().layer()));
    view.setWantsLayer(true);
    if matches!(*view.ivars().content.borrow(), Content::Timeline(_)) {
        view.registerForDraggedTypes(&NSArray::from_slice(&[unsafe { NSPasteboardTypeFileURL }]));
    }
    view
}
