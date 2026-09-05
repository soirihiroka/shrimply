mod about;
mod canvas;
mod error_alert;
mod fullscreen;
mod layout;
mod media;
mod menus;
mod timeline;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{AnyThread, DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAlert, NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate,
    NSBackingStoreType, NSControlStateValueOff, NSControlStateValueOn, NSMenuItem, NSToolbar,
    NSToolbarDelegate, NSToolbarDisplayMode, NSToolbarItem, NSWindow, NSWindowDelegate,
    NSWindowStyleMask, NSWindowToolbarStyle,
};
use objc2_foundation::{
    MainThreadMarker, NSArray, NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect,
    NSString, ns_string,
};
use shrimply_cross_ui_core::editor::EditorSession;
use shrimply_state::player_state;
use std::cell::{Cell, OnceCell, RefCell};
use std::path::Path;
use std::rc::Rc;

const DISPLAY_RATE: u32 = 60;

struct EditorIvars {
    session: OnceCell<Rc<EditorSession>>,
    imports: Rc<RefCell<media::Imports>>,
    timer: OnceCell<Retained<objc2_foundation::NSTimer>>,
    last_error: RefCell<Option<String>>,
    window: OnceCell<Retained<NSWindow>>,
    layout: OnceCell<layout::Layout>,
    view_items: OnceCell<Vec<Retained<NSMenuItem>>>,
    inspector_visible: Cell<bool>,
    timeline_visible: Cell<bool>,
    fullscreen_preview: Cell<bool>,
    fullscreen: RefCell<fullscreen::State>,
    event_monitor: OnceCell<Retained<objc2::runtime::AnyObject>>,
    title: String,
}

define_class!(
    // AppKit requires an Objective-C object for delegate and target/action callbacks.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = EditorIvars]
    struct Editor;

    unsafe impl NSObjectProtocol for Editor {}

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
            window.setTitle(&NSString::from_str(&self.ivars().title));
            window.setContentMinSize(layout::MINIMUM_WINDOW_SIZE);
            window.setTabbingMode(objc2_app_kit::NSWindowTabbingMode::Disallowed);
            window.setDelegate(Some(ProtocolObject::from_ref(self)));
            let layout = layout::build(self);
            window.setContentViewController(Some(&layout.root));
            self.ivars().layout.set(layout).unwrap_or_else(|_| panic!("layout already installed"));

            let toolbar = NSToolbar::initWithIdentifier(NSToolbar::alloc(mtm), ns_string!("Editor"));
            toolbar.setDisplayMode(NSToolbarDisplayMode::IconOnly);
            toolbar.setAllowsUserCustomization(false);
            toolbar.setDelegate(Some(ProtocolObject::from_ref(self)));
            window.setToolbar(Some(&toolbar));
            window.setToolbarStyle(NSWindowToolbarStyle::UnifiedCompact);
            menus::install(self);
            self.sync_panels();
            window.center();
            window.makeKeyAndOrderFront(None);
            self.ivars().window.set(window).expect("window already installed");
            self.install_fullscreen_events();
            app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
            app.activate();
            let timer = unsafe {
                objc2_foundation::NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
                    1.0 / f64::from(DISPLAY_RATE), self, sel!(renderFrame:), None, true,
                )
            };
            unsafe { objc2_foundation::NSRunLoop::mainRunLoop().addTimer_forMode(&timer, objc2_foundation::NSRunLoopCommonModes); }
            self.ivars().timer.set(timer).expect("frame timer already installed");

        }
    }

    unsafe impl NSWindowDelegate for Editor {
        #[unsafe(method(windowWillClose:))]
        fn will_close(&self, _notification: &NSNotification) {
            if let Some(timer) = self.ivars().timer.get() { timer.invalidate(); }
            if let Some(monitor) = self.ivars().event_monitor.get() {
                unsafe { objc2_app_kit::NSEvent::removeMonitor(monitor); }
            }
            NSApplication::sharedApplication(self.mtm()).terminate(None);
        }

        #[unsafe(method(windowDidExitFullScreen:))]
        fn did_exit_fullscreen(&self, _notification: &NSNotification) {
            self.ivars().fullscreen_preview.set(false);
            self.sync_panels();
        }

        #[unsafe(method(windowDidEnterFullScreen:))]
        fn did_enter_fullscreen(&self, _notification: &NSNotification) {
            self.ivars().fullscreen_preview.set(true);
            self.sync_panels();
        }

        #[unsafe(method(windowDidFailToEnterFullScreen:))]
        fn failed_to_enter_fullscreen(&self, _window: &NSWindow) {
            self.ivars().fullscreen_preview.set(false);
            self.sync_panels();
        }
    }

    unsafe impl NSToolbarDelegate for Editor {
        #[unsafe(method_id(toolbarDefaultItemIdentifiers:))]
        fn default_items(&self, _toolbar: &NSToolbar) -> Retained<NSArray<NSString>> {
            menus::toolbar_identifiers()
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
        #[unsafe(method(renderFrame:))]
        fn render_frame(&self, _timer: &objc2_foundation::NSTimer) {
            let session = self.ivars().session.get().expect("project loaded");
            let imported = self.ivars().imports.borrow_mut().poll(session);
            if let Err(error) = imported { self.show_error(&error); }
            let update = session.poll();
            if let Some(error) = update.audio_playback_stopped { self.show_error(&error); }
            if let Some(title) = update.title { self.ivars().window.get().expect("window installed").setTitle(&NSString::from_str(&title.text)); }
            let layout = self.ivars().layout.get().expect("layout installed");
            let player = player_state::snapshot(&session.player_state);
            self.tick_fullscreen(player.playing);
            layout.progress.setDoubleValue(shrimply_math_core::time_ratio_f64(player.position, player.duration));
            layout.time.setStringValue(&NSString::from_str(&format!("{} / {}", shrimply_project::time_format::playback_time(player.position), shrimply_project::time_format::playback_time(player.duration))));
            let speed = shrimply_preview_core::playback::playback_speed_label(player.playback_speed);
            layout.speed.setStringValue(&NSString::from_str(&speed));
            layout.speed.setToolTip(Some(&NSString::from_str(&format!("Playback speed {speed}"))));
            layout.play.setToolTip(Some(&NSString::from_str(if player.playing { "Pause" } else { "Play" })));
            layout.play.setImage
(Some(&layout::symbol(if player.playing { "pause.fill" } else { "play.fill" }, if player.playing { "Pause" } else { "Play" })));
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

        #[unsafe(method(saveProject:))]

        fn save_project(&self, _sender: &NSObject) {
            if let Err(error) = self.ivars().session.get().expect("project loaded").save() { self.show_error(&error); }
        }

        #[unsafe(method(showAbout:))]
        fn show_about(&self, _sender: &NSObject) {
            about::show(self.mtm());
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

    fn sync_panels(&self) {
        let ivars = self.ivars();
        let layout = ivars.layout.get().expect("layout must exist");
        let fullscreen = ivars.fullscreen_preview.get();
        layout
            .inspector
            .setCollapsed(fullscreen || !ivars.inspector_visible.get());
        layout
            .timeline
            .setCollapsed(fullscreen || !ivars.timeline_visible.get());
        self.sync_fullscreen_layout();
        layout.root.view().setNeedsLayout(true);
        layout.root.view().layoutSubtreeIfNeeded();
        for canvas in &layout.canvases {
            canvas.set_preview_fullscreen(fullscreen);
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

pub fn run(project: Option<&Path>) {
    let mtm = MainThreadMarker::new().expect("AppKit must start on the main thread");
    objc2_foundation::NSProcessInfo::processInfo().setProcessName(ns_string!("Shrimply"));
    let app = NSApplication::sharedApplication(mtm);
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
            return;
        }
        chosen = panel
            .URL()
            .expect("selected project URL")
            .to_file_path()
            .expect("local project file");
        &chosen
    };
    let prepared = match shrimply_project::project::prepare_project(path) {
        Ok(prepared) => prepared,
        Err(error) => {
            error_alert::show(mtm, &format!("Could not open project: {error:?}"));
            return;
        }
    };
    let session = Rc::new(
        EditorSession::new(shrimply_project::project::activate_project(prepared))
            .expect("initialize editor playback"),
    );
    let title = session.title().text;
    let editor = Editor::alloc(mtm).set_ivars(EditorIvars {
        session: OnceCell::from(session),
        imports: Rc::new(RefCell::new(media::Imports::default())),
        timer: OnceCell::new(),
        last_error: RefCell::new(None),
        window: OnceCell::new(),
        layout: OnceCell::new(),
        view_items: OnceCell::new(),
        inspector_visible: Cell::new(true),
        timeline_visible: Cell::new(true),
        fullscreen_preview: Cell::new(false),
        fullscreen: RefCell::new(fullscreen::State::default()),
        event_monitor: OnceCell::new(),
        title,
    });
    let editor: Retained<Editor> = unsafe { msg_send![super(editor), init] };
    app.setDelegate(Some(ProtocolObject::from_ref(&*editor)));
    app.run();
}
