mod about;
mod audio_meter;
mod layout;
mod menus;
mod timeline;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{AnyThread, DefinedClass, MainThreadOnly, define_class, msg_send};
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
use std::cell::{Cell, OnceCell};
use std::path::Path;

struct EditorIvars {
    window: OnceCell<Retained<NSWindow>>,
    layout: OnceCell<layout::Layout>,
    view_items: OnceCell<Vec<Retained<NSMenuItem>>>,
    inspector_visible: Cell<bool>,
    timeline_visible: Cell<bool>,
    fullscreen_preview: Cell<bool>,
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
            app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
            app.activate();
        }
    }

    unsafe impl NSWindowDelegate for Editor {
        #[unsafe(method(windowWillClose:))]
        fn will_close(&self, _notification: &NSNotification) {
            NSApplication::sharedApplication(self.mtm()).terminate(None);
        }

        #[unsafe(method(windowDidExitFullScreen:))]
        fn did_exit_fullscreen(&self, _notification: &NSNotification) {
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
            let fullscreen = !self.ivars().fullscreen_preview.get();
            self.ivars().fullscreen_preview.set(fullscreen);
            self.sync_panels();
            let window = self.ivars().window.get().expect("editor window must exist");
            if window.styleMask().contains(NSWindowStyleMask::FullScreen) != fullscreen {
                window.toggleFullScreen(None);
            }
        }

        #[unsafe(method(showShortcuts:))]
        fn show_shortcuts(&self, _sender: &NSObject) {
            let alert = NSAlert::new(self.mtm());
            alert.setMessageText(ns_string!("Keyboard Shortcuts"));
            alert.setInformativeText(ns_string!("⌘1  Toggle Inspector\n⌘2  Toggle Timeline\n⌃⌘F  Fullscreen Preview\n⌘W  Close Window\n⌘Q  Quit\n\nEditing, playback, and export are unavailable in this layout preview."));
            alert.runModal();
        }
    }
);

impl Editor {
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
    let title = project.and_then(Path::file_name).map_or_else(
        || "Shrimply — Layout Preview".to_string(),
        |name| format!("{} — Layout Preview", name.to_string_lossy()),
    );
    let editor = Editor::alloc(mtm).set_ivars(EditorIvars {
        window: OnceCell::new(),
        layout: OnceCell::new(),
        view_items: OnceCell::new(),
        inspector_visible: Cell::new(true),
        timeline_visible: Cell::new(true),
        fullscreen_preview: Cell::new(false),
        title,
    });
    let editor: Retained<Editor> = unsafe { msg_send![super(editor), init] };
    app.setDelegate(Some(ProtocolObject::from_ref(&*editor)));
    app.run();
}
