pub mod about;
mod canvas;
mod error_alert;
mod fullscreen;
mod inspector_split;
mod layout;
mod layout_split;
mod loading;
mod media;
mod menu_sheet;
mod menus;
mod save;
pub mod settings;
mod timeline;
mod transport;
mod trust;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{AnyThread, DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAlert, NSAlertSecondButtonReturn, NSAlertThirdButtonReturn, NSApplication,
    NSApplicationActivationPolicy, NSApplicationDelegate, NSBackingStoreType,
    NSControlStateValueOff, NSControlStateValueOn, NSMenuItem, NSMenuItemValidation, NSToolbar,
    NSToolbarDelegate, NSToolbarDisplayMode, NSToolbarItem, NSUserInterfaceItemIdentification,
    NSWindow, NSWindowDelegate, NSWindowStyleMask, NSWindowToolbarStyle,
};
use objc2_foundation::{
    MainThreadMarker, NSArray, NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect,
    NSString, NSTimer, ns_string,
};
use shrimply_cross_ui_core::editor::EditorSession;
use shrimply_editor_state::player_state;
use std::cell::{Cell, OnceCell, RefCell};
use std::path::Path;
use std::rc::Rc;

const SESSION_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(16);

struct EditorIvars {
    session: OnceCell<Rc<EditorSession>>,
    imports: Rc<RefCell<media::Imports>>,
    display_link: OnceCell<Retained<objc2_quartz_core::CADisplayLink>>,
    session_timer: OnceCell<Retained<NSTimer>>,
    last_error: RefCell<Option<String>>,
    playback_display: Cell<Option<player_state::Snapshot>>,
    last_callback: Cell<Option<std::time::Instant>>,
    window: OnceCell<Retained<NSWindow>>,
    layout: OnceCell<layout::Layout>,
    view_items: OnceCell<Vec<Retained<NSMenuItem>>>,
    editor_layout: Cell<Option<shrimply_editor_state::preferences::EditorLayout>>,
    fullscreen_preview: Cell<bool>,
    fullscreen: RefCell<fullscreen::State>,
    settings: OnceCell<Retained<settings::PreferencesWindow>>,
    event_monitor: OnceCell<Retained<objc2::runtime::AnyObject>>,
    project_path: std::path::PathBuf,
    trust_requests: std::sync::mpsc::Receiver<shrimply_trust_core::Request>,
    trust_dialog_open: Cell<bool>,
    preparation: RefCell<Option<loading::Preparation>>,
    loading: RefCell<Option<loading::View>>,
    outcome: Cell<Result<bool, ()>>,
}

define_class!(
    // AppKit requires an Objective-C object for delegate and target/action callbacks.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = EditorIvars]
    struct Editor;

    unsafe impl NSObjectProtocol for Editor {}

    unsafe impl NSMenuItemValidation for Editor {
        #[unsafe(method(validateMenuItem:))]
        fn validate_menu_item(&self, item: &NSMenuItem) -> bool {
            if self.ivars().loading.borrow().is_some() { return false.into(); }
            let redo = match item.action() {
                Some(action) if action == sel!(undo:) => Some(false),
                Some(action) if action == sel!(redo:) => Some(true),
                Some(action) if action == sel!(activateProjectMenu:) => menus::selected_action(item).history(),
                _ => None,
            };
            redo.is_none_or(|redo| {
                text_undo_manager(self, redo).is_some() || if redo {
                    shrimply_project_document::project::can_redo()
                } else {
                    shrimply_project_document::project::can_undo()
                }
            })
        }
    }

    unsafe impl NSApplicationDelegate for Editor {
        #[unsafe(method(applicationWillTerminate:))]
        fn will_terminate(&self, _notification: &NSNotification) {
            if let Err(error) = shrimply_project_document::project::shutdown_history() {
                tracing::error!(%error, "Could not save project during shutdown");
            }
        }

        #[unsafe(method(applicationDidFinishLaunching:))]
        fn did_finish_launching(&self, _notification: &NSNotification) {
            let mtm = self.mtm();
            let app = NSApplication::sharedApplication(mtm);
            let window = unsafe {
                NSWindow::initWithContentRect_styleMask_backing_defer(
                    NSWindow::alloc(mtm),
                    NSRect::new(NSPoint::ZERO, layout::WINDOW_SIZE),
                    NSWindowStyleMask::Titled | NSWindowStyleMask::Closable
                        | NSWindowStyleMask::Miniaturizable | NSWindowStyleMask::Resizable,
                    NSBackingStoreType::Buffered,
                    false,
                )
            };
            unsafe { window.setReleasedWhenClosed(false) };
            window.setTitle(ns_string!("Shrimply"));
            window.setContentMinSize(layout::MINIMUM_WINDOW_SIZE);
            window.setTabbingMode(objc2_app_kit::NSWindowTabbingMode::Disallowed);
            window.setDelegate(Some(ProtocolObject::from_ref(self)));
            let loading = loading::View::new(&self.ivars().project_path, mtm);
            window.setContentView(Some(&loading.root));
            self.ivars().loading.replace(Some(loading));
            let toolbar = NSToolbar::initWithIdentifier(NSToolbar::alloc(mtm), ns_string!("Editor"));
            toolbar.setDisplayMode(NSToolbarDisplayMode::IconOnly);
            toolbar.setAllowsUserCustomization(false);
            toolbar.setDelegate(Some(ProtocolObject::from_ref(self)));
            window.setToolbar(Some(&toolbar));
            window.setToolbarStyle(NSWindowToolbarStyle::UnifiedCompact);
            window.center();
            window.makeKeyAndOrderFront(None);
            self.ivars().window.set(window).expect("window already installed");
            app.activate();
            let display_link = unsafe {
                self.ivars().window.get().expect("window installed")
                    .displayLinkWithTarget_selector(self, sel!(renderFrame:))
            };
            unsafe { display_link.addToRunLoop_forMode(&objc2_foundation::NSRunLoop::mainRunLoop(), objc2_foundation::NSRunLoopCommonModes); }
            self.ivars().display_link.set(display_link).expect("display link already installed");
            // Agent requests must keep running when the window's display link is suspended.
            let session_timer = unsafe {
                NSTimer::timerWithTimeInterval_target_selector_userInfo_repeats(
                    SESSION_POLL_INTERVAL.as_secs_f64(), self, sel!(pollSession:), None, true,
                )
            };
            unsafe { objc2_foundation::NSRunLoop::mainRunLoop().addTimer_forMode(&session_timer, objc2_foundation::NSRunLoopCommonModes); }
            self.ivars().session_timer.set(session_timer).expect("session timer already installed");
            self.begin_project_load();

        }
    }

    unsafe impl NSWindowDelegate for Editor {
        #[unsafe(method(window:willPositionSheet:usingRect:))]
        fn position_sheet(&self, window: &NSWindow, sheet: &NSWindow, rect: NSRect) -> NSRect {
            if sheet.identifier().is_none_or(|identifier| identifier.to_string() != menu_sheet::IDENTIFIER) {
                return rect;
            }
            let bounds = window.contentView().expect("sheet parent has content").bounds();
            let content = sheet.contentView().expect("menu sheet has content");
            let chrome = sheet.frame().size.height - content.frame().size.height;
            let document_height = content.downcast_ref::<objc2_app_kit::NSScrollView>()
                .and_then(|scroll| scroll.documentView())
                .expect("menu sheet has a scrollable document").frame().size.height;
            let size = objc2_foundation::NSSize::new(
                bounds.size.width, document_height.min((bounds.size.height - chrome).max(0.0)),
            );
            if content.frame().size != size {
                sheet.setContentSize(size);
            }
            // AppKit's attachment rectangle describes the sheet's top edge.
            NSRect::new(NSPoint::new(0.0, sheet.frame().size.height.min(bounds.size.height)),
                objc2_foundation::NSSize::new(bounds.size.width, 0.0))
        }

        #[unsafe(method(windowWillClose:))]
        fn will_close(&self, _notification: &NSNotification) {
            if let Some(display_link) = self.ivars().display_link.get() { display_link.invalidate(); }
            if let Some(timer) = self.ivars().session_timer.get() { timer.invalidate(); }
            if let Some(monitor) = self.ivars().event_monitor.get() {
                unsafe { objc2_app_kit::NSEvent::removeMonitor(monitor); }
            }
            if let Some(layout) = self.ivars().layout.get() {
                for canvas in &layout.canvases {
                    canvas.suspend_timeline();
                }
            }
            if self.ivars().loading.borrow().is_some() {
                self.stop_loading(Ok(false));
            } else {
                NSApplication::sharedApplication(self.mtm()).terminate(None);
            }
        }

        #[unsafe(method(windowDidExitFullScreen:))]
        fn did_exit_fullscreen(&self, _notification: &NSNotification) {
            if self.ivars().loading.borrow().is_some() { return; }
            self.ivars().fullscreen_preview.set(false);
            self.sync_panels();
        }

        #[unsafe(method(windowDidEnterFullScreen:))]
        fn did_enter_fullscreen(&self, _notification: &NSNotification) {
            if self.ivars().loading.borrow().is_some() { return; }
            self.ivars().fullscreen_preview.set(true);
            self.sync_panels();
        }

        #[unsafe(method(windowDidFailToEnterFullScreen:))]
        fn failed_to_enter_fullscreen(&self, _window: &NSWindow) {
            if self.ivars().loading.borrow().is_some() { return; }
            self.ivars().fullscreen_preview.set(false);
            self.sync_panels();
        }
    }

    unsafe impl NSToolbarDelegate for Editor {
        #[unsafe(method_id(toolbarDefaultItemIdentifiers:))]
        fn default_items(&self, _toolbar: &NSToolbar) -> Retained<NSArray<NSString>> {
            if self.ivars().loading.borrow().is_some() {
                NSArray::new()
            } else {
                menus::toolbar_identifiers()
            }
        }

        #[unsafe(method_id(toolbarAllowedItemIdentifiers:))]
        fn allowed_items(&self, _toolbar: &NSToolbar) -> Retained<NSArray<NSString>> {
            menus::toolbar_identifiers()
        }

        #[unsafe(method_id(toolbar:itemForItemIdentifier:willBeInsertedIntoToolbar:))]
        fn toolbar_item(&self, _toolbar: &NSToolbar, identifier: &NSString, _inserted: bool) -> Option<Retained<NSToolbarItem>> {
            menus::toolbar_item(self, identifier)
        }
    }

    impl Editor {
        #[unsafe(method(exportCaptions:))]
        fn export_captions(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            let result = self.ivars().layout.get()
                .ok_or_else(|| "The editor layout is not ready.".to_string())
                .and_then(|layout| layout.canvases.first()
                    .ok_or_else(|| "The editor has no canvas.".to_string())?
                    .export_captions());
            if let Err(error) = result { self.show_error(&error); }
        }

        #[unsafe(method(exportVideo:))]
        fn export_video(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            let result = self
                .ivars()
                .layout
                .get()
                .ok_or_else(|| "The editor layout is not ready.".to_string())
                .and_then(|layout| {
                    layout
                        .canvases
                        .first()
                        .ok_or_else(|| "The editor has no canvas.".to_string())?
                        .export_video()
                });
            if let Err(error) = result {
                self.show_error(&error);
            }
        }

        #[unsafe(method(pollSession:))]
        fn poll_session(&self, _timer: &NSTimer) {
            let Some(session) = self.ivars().session.get() else {
                self.poll_project_load();
                return;
            };
            if !self.ivars().trust_dialog_open.get() && let Ok(request) = self.ivars().trust_requests.try_recv() {
                self.ivars().trust_dialog_open.set(true);
                let result = trust::confirm(&request.review, self.mtm()).ok_or_else(|| "Trust approval was canceled".to_string())
                    .and_then(|kind| request.review.approve(kind));
                let _ = request.response.send(result);
                self.ivars().trust_dialog_open.set(false);
            }
            for error in shrimply_trust_core::poll_edits() { self.show_error(&error); }
            let update = {
                let _timing = shrimply_process_reporting::diagnostics::timing("Session polling and view-state queue");
                session.poll()
            };
            if let Some(error) = update.audio_playback_stopped { self.show_error(&error); }
            if let Some(notice) = update.version_notice { self.show_error(&notice); }
            if let Some(title) = update.title { self.ivars().window.get().expect("window installed").setTitle(&NSString::from_str(&title.text)); }
        }

        #[unsafe(method(renderFrame:))]
        fn render_frame(&self, _display_link: &objc2_quartz_core::CADisplayLink) {
            shrimply_process_reporting::diagnostics::flush_timings();
            let now = std::time::Instant::now();
            if let Some(previous) = self.ivars().last_callback.replace(Some(now)) {
                shrimply_process_reporting::diagnostics::record_timing("UI callback start-to-start interval", now.duration_since(previous));
            }
            shrimply_process_reporting::diagnostics::count("Display-link callback");
            let _timing = shrimply_process_reporting::diagnostics::timing("UI callback total");
            if self.ivars().session.get().is_none() {
                return;
            }
            let session = self.ivars().session.get().expect("project loaded");
            let imported = {
                let _timing = shrimply_process_reporting::diagnostics::timing("Import polling");
                self.ivars().imports.borrow_mut().poll(session)
            };
            if let Err(error) = imported { self.show_error(&error); }
            let source_deletion = self.ivars().imports.borrow_mut().take_source_deletion();
            if let Some(request) = source_deletion
                && let Err(error) = media::confirm_source_deletion(request, &self.ivars().imports, self.mtm())
            {
                self.show_error(&error);
            }
            if self.ivars().loading.borrow().is_some() {
                self.poll_preview_startup();
                return;
            }
            let layout = self.ivars().layout.get().expect("layout installed");
            let preferences = shrimply_editor_state::preferences::snapshot(&session.preferences);
            if self.ivars().editor_layout.get() != Some(preferences.editor_layout) {
                self.sync_panels();
            }
            let tools_hidden = self.ivars().fullscreen_preview.get()
                || !preferences.preview_controls_visible;
            if layout.preview_tools.isHidden() != tools_hidden {
                layout.set_preview_tools_hidden(tools_hidden);
            }
            let inspector_errors = {
                let _timing = shrimply_process_reporting::diagnostics::timing("Inspector polling and rebuild");
                layout.inspector_controller.poll(self.mtm())
            };
            for error in inspector_errors {
                self.show_error(&format!("Could not inspect Manim scenes:\n{error}"));
            }
            let player = player_state::snapshot(&session.player_state);
            shrimply_process_reporting::diagnostics::count(if player.playing { "Callback / playing" } else { "Callback / paused" });
            self.tick_fullscreen(player.playing);
            let previous = self.ivars().playback_display.replace(Some(player));
            if previous.is_none_or(|old| old.position != player.position || old.duration != player.duration) {
                layout.progress.setDoubleValue(shrimply_math_core::time_ratio_f64(player.position, player.duration));
                let time = NSString::from_str(&format!("{} / {}", shrimply_project_document::time_format::playback_time(player.position), shrimply_project_document::time_format::playback_time(player.duration)));
                if layout.time.stringValue() != time {
                    layout.time.setStringValue(&time);
                    layout.playbar.setNeedsLayout(true);
                }
            }
            if previous.is_none_or(|old| old.playback_speed != player.playback_speed) {
                let speed = shrimply_preview_provider_skia::playback::playback_speed_label(player.playback_speed);
                layout.speed.setStringValue(&NSString::from_str(&speed));
                layout.speed.setToolTip(Some(&NSString::from_str(&format!("Playback speed {speed}"))));
            }
            if previous.is_none_or(|old| old.playing != player.playing) {
                let label = if player.playing { "Pause" } else { "Play" };
                layout.play.setToolTip(Some(&NSString::from_str(label)));
                layout.play.setImage(Some(&layout::symbol(if player.playing { "pause.fill" } else { "play.fill" }, label)));
            }
            for canvas in &layout.canvases {
                if let Err(error) = canvas.render() {
                    player_state::set_playing(&session.player_state, false);
                    if self.ivars().last_error.borrow().as_ref() != Some(&error) {
                        self.ivars().last_error.replace(Some(error.clone()));
                        self.show_error(&error);
                    }
                }
            }
        }

        #[unsafe(method(togglePlayback:))]
        fn toggle_playback(&self, _sender: &NSObject) {
            player_state::toggle_playing(&self.ivars().session.get().expect("project loaded").player_state);
        }

        #[unsafe(method(stepBackward:))]
        fn step_backward(&self, _sender: &NSObject) { self.step(false); }

        #[unsafe(method(stepForward:))]
        fn step_forward(&self, _sender: &NSObject) { self.step(true); }

        #[unsafe(method(seek:))]
        fn seek(&self, sender: &objc2_app_kit::NSSlider) {
            let session = self.ivars().session.get().expect("project loaded");
            let scrubbing = NSApplication::sharedApplication(self.mtm()).currentEvent().is_some_and(|event| {
                matches!(event.r#type(), objc2_app_kit::NSEventType::LeftMouseDown | objc2_app_kit::NSEventType::LeftMouseDragged)
            });
            player_state::set_scrubbing(&session.player_state, scrubbing);
            let player = player_state::snapshot(&session.player_state);
            player_state::seek_time(&session.player_state, player.duration.scaled(shrimply_math_core::fraction_from_f64(sender.doubleValue())));
        }

        #[unsafe(method(activateProjectMenu:))]
        fn activate_project_menu(&self, sender: &NSMenuItem) {
            let action = menus::selected_action(sender);
            if let Some(redo) = action.history() {
                self.project_history(redo);
                return;
            }
            let session = self.ivars().session.get().expect("project loaded");
            match session.activate_project_menu(action) {
                Ok(Some(action)) => Self::project_host_action(action).expect("advertised window callback")(self),
                Ok(None) => {}
                Err(error) => self.show_error(&error),
            }
        }

        #[unsafe(method(undo:))]
        fn undo(&self, _sender: &NSObject) {
            self.project_history(false);
        }

        #[unsafe(method(redo:))]
        fn redo(&self, _sender: &NSObject) {
            self.project_history(true);
        }

        #[unsafe(method(toggleInspector:))]
        fn toggle_inspector(&self, _sender: &NSObject) {
            self.update_editor_layout(|layout| layout.inspector_visible = !layout.inspector_visible);
        }

        #[unsafe(method(toggleTimeline:))]
        fn toggle_timeline(&self, _sender: &NSObject) {
            self.update_editor_layout(|layout| layout.timeline_visible = !layout.timeline_visible);
        }

        #[unsafe(method(togglePreviewFullscreen:))]
        fn toggle_fullscreen(&self, _sender: &NSObject) {
            self.toggle_preview_fullscreen();
        }
    }
);

impl Editor {
    fn project_host_action(
        action: shrimply_cross_ui_core::editor::project_menu::Action,
    ) -> Option<fn(&Self)> {
        use shrimply_cross_ui_core::editor::project_menu::Action;
        Some(match action {
            Action::Import => Self::import_media,
            Action::SaveAs => Self::save_project_as,
            Action::Preferences => Self::show_settings,
            Action::Shortcuts => Self::show_shortcuts,
            Action::RestartService => |_| {
                let path = shrimply_project_document::project::active_project_path();
                std::thread::spawn(move || {
                    if let Err(error) =
                        shrimply_cross_ui_core::launcher::restart_local_service(&path)
                    {
                        let completed = block2::RcBlock::new(move || {
                            error_alert::show(
                                MainThreadMarker::new().expect("main thread"),
                                &error,
                            );
                        });
                        unsafe {
                            objc2_foundation::NSOperationQueue::mainQueue()
                                .addOperationWithBlock(&completed)
                        };
                    }
                });
            },
            Action::About => |editor| about::show(editor.mtm()),
            _ => return None,
        })
    }

    fn import_media(&self) {
        let session = self.ivars().session.get().expect("project loaded");
        if let Err(error) = media::choose_files(
            &self.ivars().imports,
            session,
            &[],
            player_state::current_time(&session.player_state),
            self.mtm(),
        ) {
            self.show_error(&error);
        }
    }

    fn save_project_as(&self) {
        let window = self.ivars().window.get().expect("window created");
        if !window.makeFirstResponder(None) {
            return;
        }
        if let Err(error) = save::show(window, self.ivars().session.get().expect("project loaded"))
        {
            self.show_error(&error);
        }
    }

    fn show_settings(&self) {
        self.ivars()
            .settings
            .get_or_init(|| {
                let store = &self
                    .ivars()
                    .session
                    .get()
                    .expect("project loaded")
                    .preferences;
                settings::PreferencesWindow::new(self.mtm(), Some(store.clone()))
            })
            .present();
    }

    fn show_shortcuts(&self) {
        let alert = NSAlert::new(self.mtm());
        alert.setMessageText(ns_string!("Keyboard Shortcuts"));
        alert.setInformativeText(ns_string!("⌘1  Toggle Inspector\n⌘2  Toggle Timeline\n⌃⌘F  Fullscreen Preview\n⌘W  Close Window\n⌘Q  Quit\n\nSpace  Play / Pause"));
        alert.runModal();
    }

    fn project_history(&self, redo: bool) {
        use shrimply_cross_ui_core::editor::project_menu::Action;

        if let Some(manager) = text_undo_manager(self, redo) {
            if redo {
                manager.redo();
            } else {
                manager.undo();
            }
            return;
        }
        if !self
            .ivars()
            .window
            .get()
            .expect("window created")
            .makeFirstResponder(None)
        {
            return;
        }
        let session = self.ivars().session.get().expect("project loaded");
        if let Err(error) =
            session.activate_project_menu(if redo { Action::Redo } else { Action::Undo })
        {
            self.show_error(&error);
        }
    }
}

fn text_undo_manager(
    editor: &Editor,
    redo: bool,
) -> Option<Retained<objc2_foundation::NSUndoManager>> {
    let responder = editor.ivars().window.get()?.firstResponder()?;
    let text = responder.downcast_ref::<objc2_app_kit::NSTextView>()?;
    if !text.allowsUndo() {
        return None;
    }
    let manager = text.undoManager()?;
    (if redo {
        manager.canRedo()
    } else {
        manager.canUndo()
    })
    .then_some(manager)
}

impl Editor {
    fn show_error(&self, error: &str) {
        error_alert::show(self.mtm(), error);
    }

    fn step(&self, forward: bool) {
        let session = self.ivars().session.get().expect("project loaded");
        player_state::set_playing(&session.player_state, false);
        let time = player_state::current_time(&session.player_state);
        let step = session.project.borrow().frame_step();
        player_state::seek_time(
            &session.player_state,
            if forward {
                time.saturating_add(step)
            } else {
                time.saturating_sub(step)
            },
        );
    }

    fn update_editor_layout(
        &self,
        update: impl FnOnce(&mut shrimply_editor_state::preferences::EditorLayout),
    ) {
        let session = self.ivars().session.get().expect("project loaded");
        if let Err(error) =
            shrimply_editor_state::preferences::update_editor_layout(&session.preferences, update)
        {
            self.show_error(&error);
        }
        self.sync_panels();
    }

    fn sync_panels(&self) {
        use objc2_app_kit::{NSAnimatablePropertyContainer, NSAnimationContext};

        let ivars = self.ivars();
        let layout = ivars.layout.get().expect("layout must exist");
        let fullscreen = ivars.fullscreen_preview.get();
        let desired = shrimply_editor_state::preferences::snapshot(
            &ivars.session.get().expect("project loaded").preferences,
        )
        .editor_layout;
        let previous = ivars.editor_layout.replace(Some(desired));
        let animate = previous.is_some() && !fullscreen && !ivars.fullscreen.borrow().is_active();
        let inspector_collapsed = fullscreen || !desired.inspector_visible;
        let timeline_collapsed = fullscreen || !desired.timeline_visible;
        let inspector_changed =
            previous.is_some_and(|prev| prev.inspector_visible != desired.inspector_visible);
        let timeline_changed = layout.timeline.isCollapsed() != timeline_collapsed;

        layout
            .inspector
            .set_collapsed(inspector_collapsed, animate && inspector_changed);
        if animate && timeline_changed {
            if !timeline_collapsed {
                layout.root_split.sync_preferred_thickness(&layout.timeline);
            }
            let generation = layout.root_split.begin_animation();
            let timeline = layout.timeline.clone();
            let changes =
                block2::RcBlock::new(move |context: std::ptr::NonNull<NSAnimationContext>| {
                    unsafe { context.as_ref() }.setDuration(layout_split::PANEL_ANIMATION_SECONDS);
                    unsafe { context.as_ref() }.setAllowsImplicitAnimation(true);
                    timeline.animator().setCollapsed(timeline_collapsed);
                });
            let split = layout.root_split.clone();
            let completion = block2::RcBlock::new(move || {
                if split.finish_animation(generation) {
                    split.restore();
                }
            });
            NSAnimationContext::runAnimationGroup_completionHandler(&changes, Some(&completion));
        } else {
            layout.root_split.cancel_animation();
            layout.timeline.setCollapsed(timeline_collapsed);
            self.sync_fullscreen_layout();
            layout.root.view().setNeedsLayout(true);
            layout.root.view().layoutSubtreeIfNeeded();
            layout.root_split.restore();
        }
        let timeline_active = !fullscreen && desired.timeline_visible;
        for canvas in &layout.canvases {
            canvas.set_preview_fullscreen(fullscreen);
            if !timeline_active {
                canvas.suspend_timeline();
            }
        }
        if let Some(items) = ivars.view_items.get() {
            for (item, checked) in items.iter().zip([
                desired.inspector_visible,
                desired.timeline_visible,
                fullscreen,
            ]) {
                item.setState(if checked {
                    NSControlStateValueOn
                } else {
                    NSControlStateValueOff
                });
            }
        }
    }
}

pub fn run(path: &Path) -> Result<bool, ()> {
    let mtm = MainThreadMarker::new().expect("AppKit must start on the main thread");
    objc2_foundation::NSProcessInfo::processInfo().setProcessName(ns_string!("Shrimply"));
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    NSWindow::setAllowsAutomaticWindowTabbing(false, mtm);
    let editor = Editor::alloc(mtm).set_ivars(EditorIvars {
        session: OnceCell::new(),
        imports: Rc::new(RefCell::new(media::Imports::default())),
        display_link: OnceCell::new(),
        session_timer: OnceCell::new(),
        last_callback: Cell::new(None),
        last_error: RefCell::new(None),
        playback_display: Cell::new(None),
        window: OnceCell::new(),
        layout: OnceCell::new(),
        view_items: OnceCell::new(),
        editor_layout: Cell::new(None),
        fullscreen_preview: Cell::new(false),
        fullscreen: RefCell::new(fullscreen::State::default()),
        settings: OnceCell::new(),
        event_monitor: OnceCell::new(),
        project_path: path.to_path_buf(),
        trust_requests: shrimply_trust_core::interactive_requests(),
        trust_dialog_open: Cell::new(false),
        preparation: RefCell::new(None),
        loading: RefCell::new(None),
        outcome: Cell::new(Ok(false)),
    });
    let editor: Retained<Editor> = unsafe { msg_send![super(editor), init] };
    app.setDelegate(Some(ProtocolObject::from_ref(&*editor)));
    app.run();
    shrimply_project_document::project::clear_project_file_locks();
    editor.ivars().outcome.get()
}
