mod about;
mod canvas;
mod error_alert;
mod fullscreen;
mod inspector_split;
mod layout;
mod loading;
mod media;
mod menus;
mod save;
mod settings;
mod timeline;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{AnyThread, DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSAlertSecondButtonReturn, NSAlertThirdButtonReturn,
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSBackingStoreType,
    NSButton, NSColorWell, NSControl, NSControlStateValueOff, NSControlStateValueOn, NSMenuItem,
    NSMenuItemValidation, NSPopUpButton, NSTextField, NSToolbar, NSToolbarDelegate,
    NSToolbarDisplayMode, NSToolbarItem, NSWindow, NSWindowDelegate, NSWindowStyleMask,
    NSWindowToolbarStyle,
};
use objc2_foundation::{
    MainThreadMarker, NSArray, NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect,
    NSString, ns_string,
};
use shrimply_cross_ui_core::editor::EditorSession;
use shrimply_editor_state::player_state;
use std::cell::{Cell, OnceCell, RefCell};
use std::path::Path;
use std::rc::Rc;

struct EditorIvars {
    session: OnceCell<Rc<EditorSession>>,
    imports: Rc<RefCell<media::Imports>>,
    display_link: OnceCell<Retained<objc2_quartz_core::CADisplayLink>>,
    last_error: RefCell<Option<String>>,
    playback_display: Cell<Option<player_state::Snapshot>>,
    last_callback: Cell<Option<std::time::Instant>>,
    window: OnceCell<Retained<NSWindow>>,
    layout: OnceCell<layout::Layout>,
    view_items: OnceCell<Vec<Retained<NSMenuItem>>>,
    inspector_visible: Cell<bool>,
    timeline_visible: Cell<bool>,
    fullscreen_preview: Cell<bool>,
    fullscreen: RefCell<fullscreen::State>,
    settings_window: RefCell<Option<Retained<NSWindow>>>,
    settings_blender_probe:
        RefCell<Option<std::sync::mpsc::Receiver<Result<std::path::PathBuf, String>>>>,
    settings_server_probe: RefCell<Option<std::sync::mpsc::Receiver<settings::ServerProbe>>>,
    settings_device_probe: RefCell<Option<(u64, std::sync::mpsc::Receiver<settings::ServerProbe>)>>,
    settings_device_revision: Cell<u64>,
    settings_device_error: RefCell<Option<String>>,
    settings_server_statuses: RefCell<
        std::collections::BTreeMap<
            String,
            Result<shrimply_editor_state::preferences::ComputeServerPresentation, String>,
        >,
    >,
    event_monitor: OnceCell<Retained<objc2::runtime::AnyObject>>,
    project_path: std::path::PathBuf,
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
            match item.action() {
                Some(action) if action == sel!(undo:) => {
                    text_undo_manager(self, false).is_some()
                        || shrimply_project_document::project::can_undo()
                }
                Some(action) if action == sel!(redo:) => {
                    text_undo_manager(self, true).is_some()
                        || shrimply_project_document::project::can_redo()
                }
                _ => true,
            }
        }
    }

    unsafe impl NSApplicationDelegate for Editor {
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
            self.begin_project_load();

        }
    }

    unsafe impl NSWindowDelegate for Editor {
        #[unsafe(method(windowWillClose:))]
        fn will_close(&self, _notification: &NSNotification) {
            if let Some(display_link) = self.ivars().display_link.get() { display_link.invalidate(); }
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
                self.poll_project_load();
                return;
            }
            self.poll_blender_probe();
            self.poll_compute_server_probes();
            let session = self.ivars().session.get().expect("project loaded");
            let imported = {
                let _timing = shrimply_process_reporting::diagnostics::timing("Import polling");
                self.ivars().imports.borrow_mut().poll(session)
            };
            if let Err(error) = imported { self.show_error(&error); }
            let update = {
                let _timing = shrimply_process_reporting::diagnostics::timing("Session polling and view-state queue");
                session.poll()
            };
            if let Some(error) = update.audio_playback_stopped { self.show_error(&error); }
            if let Some(title) = update.title { self.ivars().window.get().expect("window installed").setTitle(&NSString::from_str(&title.text)); }
            if self.ivars().loading.borrow().is_some() {
                self.poll_preview_startup();
                return;
            }
            let layout = self.ivars().layout.get().expect("layout installed");
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

        #[unsafe(method(importMedia:))]
        fn import_media(&self, _sender: &NSObject) {
            if let Err(error) = media::choose_files(&self.ivars().imports, self.ivars().session.get().expect("project loaded"), &[], self.mtm()) { self.show_error(&error); }
        }

        #[unsafe(method(undo:))]
        fn undo(&self, _sender: &NSObject) {
            if let Some(manager) = text_undo_manager(self, false) {
                manager.undo();
                return;
            }
            // Finish an active field edit before restoring the project snapshot.
            if !self.ivars().window.get().expect("window created").makeFirstResponder(None) {
                return;
            }
            let session = self.ivars().session.get().expect("project loaded");
            shrimply_cross_ui_core::editor::change_history(
                &session.project, &session.player_state, shrimply_project_document::project::undo,
            );
        }

        #[unsafe(method(redo:))]
        fn redo(&self, _sender: &NSObject) {
            if let Some(manager) = text_undo_manager(self, true) {
                manager.redo();
                return;
            }
            // Finish an active field edit before restoring the project snapshot.
            if !self.ivars().window.get().expect("window created").makeFirstResponder(None) {
                return;
            }
            let session = self.ivars().session.get().expect("project loaded");
            shrimply_cross_ui_core::editor::change_history(
                &session.project, &session.player_state, shrimply_project_document::project::redo,
            );
        }

        #[unsafe(method(saveProject:))]

        fn save_project(&self, _sender: &NSObject) {
            if let Err(error) = self.ivars().session.get().expect("project loaded").save() { self.show_error(&error); }
        }

        #[unsafe(method(saveProjectAs:))]
        fn save_project_as(&self, _sender: &NSObject) {
            let window = self.ivars().window.get().expect("window created");
            if !window.makeFirstResponder(None) { return; }
            if let Err(error) = save::show(window, self.ivars().session.get().expect("project loaded")) {
                self.show_error(&error);
            }
        }

        #[unsafe(method(showAbout:))]
        fn show_about(&self, _sender: &NSObject) {
            about::show(self.mtm());
        }

        #[unsafe(method(showSettings:))]
        fn show_settings(&self, _sender: &NSObject) {
            if let Some(window) = self.ivars().settings_window.borrow().as_ref() {
                window.makeKeyAndOrderFront(None);
                self.refresh_compute_servers();
                return;
            }
            self.ivars().settings_window.replace(Some(settings::show(self)));
            self.refresh_compute_servers();
        }

        #[unsafe(method(changeNumericPreference:))]
        fn change_numeric_preference(&self, sender: &NSControl) {
            let store = &self.ivars().session.get().expect("project loaded").preferences;
            if let Err(error) = settings::change_numeric(store, sender) {
                self.show_error(error);
            }
        }

        #[unsafe(method(changePreviewFilter:))]
        fn change_preview_filter(&self, sender: &NSPopUpButton) {
            let store = &self.ivars().session.get().expect("project loaded").preferences;
            if let Err(error) = settings::change_filter(store, sender) {
                self.show_error(error);
            }
        }

        #[unsafe(method(changeDefaultFont:))]
        fn change_default_font(&self, sender: &NSPopUpButton) {
            let Some(name) = sender.titleOfSelectedItem() else { return };
            if let Err(error) = shrimply_editor_state::preferences::set_value(
                &self.ivars().session.get().expect("project loaded").preferences,
                shrimply_editor_state::preferences::PreferenceId::DefaultTextFontFamily,
                shrimply_editor_state::preferences::PreferenceValue::FontFamily(
                    shrimply_editor_state::preferences::FontFamily::Local { name: name.to_string() },
                ),
            ) {
                self.show_error(error);
            }
        }

        #[unsafe(method(changeCaptionColor:))]
        fn change_caption_color(&self, sender: &NSColorWell) {
            settings::set_caption_color(
                &self.ivars().session.get().expect("project loaded").preferences,
                sender,
            );
        }

        #[unsafe(method(changeComputeServer:))]
        fn change_compute_server(&self, sender: &NSTextField) {
            let store = &self.ivars().session.get().expect("project loaded").preferences;
            let previous = shrimply_editor_state::preferences::snapshot(store).compute_server_url;
            if let Err(error) = shrimply_editor_state::preferences::edit_compute_server(
                store,
                &previous,
                &sender.stringValue().to_string(),
            ) {
                self.show_error(error);
            }
            self.refresh_compute_servers();
        }

        #[unsafe(method(addComputeServer:))]
        fn add_compute_server(&self, _sender: &NSButton) {
            let Some(url) = settings::prompt_compute_server_url(self.mtm()) else { return };
            let store = &self.ivars().session.get().expect("project loaded").preferences;
            if let Err(error) = shrimply_editor_state::preferences::add_compute_server(store, &url) {
                self.show_error(error);
            }
            self.refresh_compute_servers();
        }

        #[unsafe(method(removeComputeServer:))]
        fn remove_compute_server(&self, _sender: &NSButton) {
            let store = &self.ivars().session.get().expect("project loaded").preferences;
            let selected = shrimply_editor_state::preferences::snapshot(store).compute_server_url;
            shrimply_editor_state::preferences::remove_compute_server(store, &selected);
            self.refresh_compute_servers();
        }

        #[unsafe(method(selectComputeServer:))]
        fn select_compute_server(&self, sender: &NSPopUpButton) {
            let (urls, _) = shrimply_editor_state::preferences::compute_servers(
                &self.ivars().session.get().expect("project loaded").preferences,
            );
            let index = sender.indexOfSelectedItem();
            let Some(url) = usize::try_from(index).ok().and_then(|index| urls.get(index)) else { return };
            assert!(
                shrimply_editor_state::preferences::select_compute_server(
                    &self.ivars().session.get().expect("project loaded").preferences,
                    url,
                ),
                "AppKit selected a compute server outside the shared preference list"
            );
            self.cancel_compute_device_probe();
            self.sync_compute_server_settings();
        }

        #[unsafe(method(selectComputeDevice:))]
        fn select_compute_device(&self, sender: &NSPopUpButton) {
            if self.ivars().settings_device_probe.borrow().is_some() {
                self.sync_compute_server_settings();
                return;
            }
            let selected_url = shrimply_editor_state::preferences::snapshot(
                &self.ivars().session.get().expect("project loaded").preferences,
            ).compute_server_url;
            let index = sender.indexOfSelectedItem();
            let device = self.ivars().settings_server_statuses.borrow()
                .get(&selected_url)
                .and_then(|result| result.as_ref().ok())
                .and_then(|status| usize::try_from(index).ok().and_then(|index| status.devices.get(index)))
                .map(|device| device.id.clone());
            let Some(device) = device else {
                self.sync_compute_server_settings();
                return;
            };
            sender.setEnabled(false);
            self.ivars().settings_device_error.borrow_mut().take();
            let revision = self.ivars().settings_device_revision.get().wrapping_add(1);
            self.ivars().settings_device_revision.set(revision);
            let (result_sender, receiver) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let result = shrimply_editor_state::preferences::select_compute_device(&selected_url, &device)
                    .map(|status| shrimply_editor_state::preferences::present_compute_server(&status));
                let _ = result_sender.send((selected_url, result));
            });
            self.ivars()
                .settings_device_probe
                .replace(Some((revision, receiver)));
        }

        #[unsafe(method(chooseBlender:))]
        fn choose_blender(&self, _sender: &NSButton) {
            if self.ivars().settings_blender_probe.borrow().is_some() {
                return;
            }
            match settings::choose_blender_path(self.mtm()) {
                Ok(Some(path)) => {
                    let (sender, receiver) = std::sync::mpsc::channel();
                    std::thread::spawn(move || {
                        let _ = sender.send(shrimply_editor_state::preferences::validate_blender_binary(&path));
                    });
                    self.ivars().settings_blender_probe.replace(Some(receiver));
                    if let Some(window) = self.ivars().settings_window.borrow().as_ref() {
                        let current = shrimply_editor_state::preferences::snapshot(
                            &self.ivars().session.get().expect("project loaded").preferences,
                        );
                        settings::sync_blender_window(
                            window,
                            current.blender_binary.as_deref(),
                            true,
                            self.mtm(),
                        );
                    }
                }
                Ok(None) => {}
                Err(error) => self.show_error(&error),
            }
        }

        #[unsafe(method(clearBlender:))]
        fn clear_blender(&self, sender: &NSButton) {
            let store = &self.ivars().session.get().expect("project loaded").preferences;
            shrimply_editor_state::preferences::apply_blender_binary(store, None);
            let snapshot = shrimply_editor_state::preferences::snapshot(store);
            let window = sender.window().expect("Blender button has a settings window");
            settings::sync_blender_window(
                &window,
                snapshot.blender_binary.as_deref(),
                false,
                self.mtm(),
            );
        }

        #[unsafe(method(toggleInspector:))]
        fn toggle_inspector(&self, _sender: &NSObject) {
            self.ivars().inspector_visible.set(!self.ivars().inspector_visible.get());
            self.sync_panels();
        }

        #[unsafe(method(toggleTimeline:))]
        fn toggle_timeline(&self, _sender: &NSObject) {
            self.ivars().timeline_visible.set(!self.ivars().timeline_visible.get());
            self.sync_panels();
        }

        #[unsafe(method(togglePreviewFullscreen:))]
        fn toggle_fullscreen(&self, _sender: &NSObject) {
            self.toggle_preview_fullscreen();
        }

        #[unsafe(method(showShortcuts:))]
        fn show_shortcuts(&self, _sender: &NSObject) {
            let alert = NSAlert::new(self.mtm());
            alert.setMessageText(ns_string!("Keyboard Shortcuts"));
            alert.setInformativeText(ns_string!("⌘1  Toggle Inspector\n⌘2  Toggle Timeline\n⌃⌘F  Fullscreen Preview\n⌘W  Close Window\n⌘Q  Quit\n\nSpace  Play / Pause"));
            alert.runModal();
        }
    }
);

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

    fn poll_blender_probe(&self) {
        let result = {
            let mut pending = self.ivars().settings_blender_probe.borrow_mut();
            let Some(receiver) = pending.as_ref() else {
                return;
            };
            match receiver.try_recv() {
                Ok(result) => {
                    pending.take();
                    Some(result)
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    pending.take();
                    Some(Err("Blender validation worker stopped unexpectedly".into()))
                }
            }
        };
        let Some(result) = result else {
            return;
        };
        let store = &self
            .ivars()
            .session
            .get()
            .expect("project loaded")
            .preferences;
        if let Ok(path) = &result {
            shrimply_editor_state::preferences::apply_blender_binary(store, Some(path.clone()));
        }
        let snapshot = shrimply_editor_state::preferences::snapshot(store);
        if let Some(window) = self.ivars().settings_window.borrow().as_ref() {
            settings::sync_blender_window(
                window,
                snapshot.blender_binary.as_deref(),
                false,
                self.mtm(),
            );
        }
        if let Err(error) = result {
            self.show_error(&error);
        }
    }

    fn refresh_compute_servers(&self) {
        self.cancel_compute_device_probe();
        let store = &self
            .ivars()
            .session
            .get()
            .expect("project loaded")
            .preferences;
        let (urls, _) = shrimply_editor_state::preferences::compute_servers(store);
        self.ivars()
            .settings_server_statuses
            .borrow_mut()
            .retain(|url, _| urls.contains(url));
        let (result_sender, receiver) = std::sync::mpsc::channel();
        for url in urls {
            let result_sender = result_sender.clone();
            std::thread::spawn(move || {
                let result =
                    shrimply_editor_state::preferences::compute_server_status(&url).map(|status| {
                        shrimply_editor_state::preferences::present_compute_server(&status)
                    });
                let _ = result_sender.send((url, result));
            });
        }
        drop(result_sender);
        self.ivars().settings_server_probe.replace(Some(receiver));
        self.sync_compute_server_settings();
    }

    fn poll_compute_server_probes(&self) {
        let mut server_results = Vec::new();
        {
            let mut pending = self.ivars().settings_server_probe.borrow_mut();
            if let Some(receiver) = pending.as_ref() {
                loop {
                    match receiver.try_recv() {
                        Ok(result) => server_results.push(result),
                        Err(std::sync::mpsc::TryRecvError::Empty) => break,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            pending.take();
                            break;
                        }
                    }
                }
            }
        }
        for (url, result) in &server_results {
            self.ivars()
                .settings_server_statuses
                .borrow_mut()
                .insert(url.clone(), result.clone());
        }
        let device_result = {
            let mut pending = self.ivars().settings_device_probe.borrow_mut();
            let received = pending
                .as_ref()
                .map(|(revision, receiver)| (*revision, receiver.try_recv()));
            match received {
                None => None,
                Some((revision, Ok(result))) => {
                    pending.take();
                    (revision == self.ivars().settings_device_revision.get()).then_some(result)
                }
                Some((_, Err(std::sync::mpsc::TryRecvError::Empty))) => None,
                Some((revision, Err(std::sync::mpsc::TryRecvError::Disconnected))) => {
                    pending.take();
                    (revision == self.ivars().settings_device_revision.get()).then(|| {
                        (
                            shrimply_editor_state::preferences::snapshot(
                                &self
                                    .ivars()
                                    .session
                                    .get()
                                    .expect("project loaded")
                                    .preferences,
                            )
                            .compute_server_url,
                            Err("Device selection stopped unexpectedly".into()),
                        )
                    })
                }
            }
        };
        if let Some((url, result)) = &device_result {
            match result {
                Ok(status) => {
                    self.ivars().settings_device_error.borrow_mut().take();
                    self.ivars()
                        .settings_server_statuses
                        .borrow_mut()
                        .insert(url.clone(), Ok(status.clone()));
                }
                Err(error) => {
                    self.ivars()
                        .settings_device_error
                        .replace(Some(error.clone()));
                }
            }
        }
        if server_results.is_empty() && device_result.is_none() {
            return;
        }
        self.sync_compute_server_settings();
    }

    fn sync_compute_server_settings(&self) {
        let Some(window) = self.ivars().settings_window.borrow().as_ref().cloned() else {
            return;
        };
        settings::sync_compute_servers_window(
            &window,
            &self
                .ivars()
                .session
                .get()
                .expect("project loaded")
                .preferences,
            &self.ivars().settings_server_statuses.borrow(),
            self.ivars().settings_device_error.borrow().as_deref(),
            self.ivars().settings_device_probe.borrow().is_some(),
            self.mtm(),
        );
    }

    fn cancel_compute_device_probe(&self) {
        self.ivars()
            .settings_device_revision
            .set(self.ivars().settings_device_revision.get().wrapping_add(1));
        self.ivars().settings_device_probe.borrow_mut().take();
        self.ivars().settings_device_error.borrow_mut().take();
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

    fn sync_panels(&self) {
        let ivars = self.ivars();
        let layout = ivars.layout.get().expect("layout must exist");
        let fullscreen = ivars.fullscreen_preview.get();
        layout
            .inspector
            .set_collapsed(fullscreen || !ivars.inspector_visible.get());
        layout
            .timeline
            .setCollapsed(fullscreen || !ivars.timeline_visible.get());
        self.sync_fullscreen_layout();
        layout.root.view().setNeedsLayout(true);
        layout.root.view().layoutSubtreeIfNeeded();
        let timeline_active = !fullscreen && ivars.timeline_visible.get();
        for canvas in &layout.canvases {
            canvas.set_preview_fullscreen(fullscreen);
            if !timeline_active {
                canvas.suspend_timeline();
            }
        }
        if let Some(items) = ivars.view_items.get() {
            for (item, checked) in items.iter().zip([
                ivars.inspector_visible.get(),
                ivars.timeline_visible.get(),
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

pub fn run(project: Option<&Path>) -> Result<bool, ()> {
    let mtm = MainThreadMarker::new().expect("AppKit must start on the main thread");
    objc2_foundation::NSProcessInfo::processInfo().setProcessName(ns_string!("Shrimply"));
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    let icon = objc2_app_kit::NSImage::initWithData(
        objc2_app_kit::NSImage::alloc(),
        &objc2_foundation::NSData::with_bytes(include_bytes!(
            "../../../../assets/icons/dev.shrimply.Shrimply.png"
        )),
    )
    .expect("the embedded Shrimply icon must be valid");
    unsafe { app.setApplicationIconImage(Some(&icon)) };
    NSWindow::setAllowsAutomaticWindowTabbing(false, mtm);
    let chosen;
    let path = if let Some(path) = project {
        path
    } else {
        let panel = objc2_app_kit::NSOpenPanel::openPanel(mtm);
        panel.setCanChooseDirectories(false);
        if panel.runModal() != objc2_app_kit::NSModalResponseOK {
            return Ok(false);
        }
        chosen = panel
            .URL()
            .expect("selected project URL")
            .to_file_path()
            .expect("local project file");
        &chosen
    };
    let editor = Editor::alloc(mtm).set_ivars(EditorIvars {
        session: OnceCell::new(),
        imports: Rc::new(RefCell::new(media::Imports::default())),
        display_link: OnceCell::new(),
        last_callback: Cell::new(None),
        last_error: RefCell::new(None),
        playback_display: Cell::new(None),
        window: OnceCell::new(),
        layout: OnceCell::new(),
        view_items: OnceCell::new(),
        inspector_visible: Cell::new(true),
        timeline_visible: Cell::new(true),
        fullscreen_preview: Cell::new(false),
        fullscreen: RefCell::new(fullscreen::State::default()),
        settings_window: RefCell::new(None),
        settings_blender_probe: RefCell::new(None),
        settings_server_probe: RefCell::new(None),
        settings_device_probe: RefCell::new(None),
        settings_device_revision: Cell::new(0),
        settings_device_error: RefCell::new(None),
        settings_server_statuses: RefCell::new(std::collections::BTreeMap::new()),
        event_monitor: OnceCell::new(),
        project_path: path.to_path_buf(),
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
