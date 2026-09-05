mod create_project;
mod recents;

use block2::StackBlock;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAlert, NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate,
    NSAutoresizingMaskOptions, NSBackingStoreType, NSBezelStyle, NSButton, NSColor, NSControlSize,
    NSFont, NSGlassEffectView, NSGlassEffectViewStyle, NSImage, NSMenu, NSMenuItem,
    NSModalResponseOK, NSOpenPanel, NSSavePanel, NSSearchField, NSTextField,
    NSTitlebarSeparatorStyle, NSToolbar, NSView, NSWindow, NSWindowDelegate, NSWindowStyleMask,
    NSWindowTitleVisibility, NSWindowToolbarStyle, NSWorkspace,
};
use objc2_foundation::{
    MainThreadMarker, NSArray, NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize,
    NSString, NSURL, ns_string,
};
use shrimply_cross_ui_core::launcher;
use shrimply_support::recent_projects::{self, RecentProject};
use std::cell::{OnceCell, RefCell};
use std::path::PathBuf;

const WINDOW_SIZE: NSSize = NSSize::new(760.0, 560.0);
const MINIMUM_WINDOW_SIZE: NSSize = NSSize::new(560.0, 420.0);
const SIDEBAR_WIDTH: f64 = 200.0;
const CONTENT_MARGIN: f64 = 12.0;
const TITLEBAR_HEIGHT: f64 = 52.0;
const SEARCH_HEIGHT: f64 = 32.0;
const CLEAR_BUTTON_WIDTH: f64 = 36.0;
const SIDEBAR_BUTTON_WIDTH: f64 = 160.0;
const SIDEBAR_BUTTON_HEIGHT: f64 = 32.0;

struct DelegateIvars {
    window: OnceCell<Retained<NSWindow>>,
    search: OnceCell<Retained<NSSearchField>>,
    recent_list: OnceCell<recents::List>,
    recent_projects: RefCell<Vec<RecentProject>>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = DelegateIvars]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl NSApplicationDelegate for Delegate {
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn did_finish_launching(&self, notification: &NSNotification) {
            let mtm = self.mtm();
            let app = notification
                .object()
                .expect("application notification must have an object")
                .downcast::<NSApplication>()
                .expect("application notification object must be NSApplication");

            let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), ns_string!(""));
            let app_item = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm),
                    ns_string!(""),
                    None,
                    ns_string!(""),
                )
            };
            let app_menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), ns_string!(""));
            unsafe {
                app_menu.addItemWithTitle_action_keyEquivalent(
                    ns_string!("Quit Shrimply"),
                    Some(sel!(terminate:)),
                    ns_string!("q"),
                );
            }
            app_item.setSubmenu(Some(&app_menu));
            menu.addItem(&app_item);
            app.setMainMenu(Some(&menu));

            let window = unsafe {
                NSWindow::initWithContentRect_styleMask_backing_defer(
                    NSWindow::alloc(mtm),
                    NSRect::new(NSPoint::new(0.0, 0.0), WINDOW_SIZE),
                    NSWindowStyleMask::Titled
                        | NSWindowStyleMask::Closable
                        | NSWindowStyleMask::Miniaturizable
                        | NSWindowStyleMask::Resizable
                        | NSWindowStyleMask::FullSizeContentView,
                    NSBackingStoreType::Buffered,
                    false,
                )
            };
            unsafe { window.setReleasedWhenClosed(false) };
            window.setTitle(ns_string!("Shrimply"));
            window.setTitleVisibility(NSWindowTitleVisibility::Hidden);
            window.setTitlebarAppearsTransparent(true);
            window.setTitlebarSeparatorStyle(NSTitlebarSeparatorStyle::None);
            window.setToolbar(Some(&NSToolbar::init(NSToolbar::alloc(mtm))));
            window.setToolbarStyle(NSWindowToolbarStyle::UnifiedCompact);
            window.setContentMinSize(MINIMUM_WINDOW_SIZE);
            window.setOpaque(false);
            window.setBackgroundColor(Some(&NSColor::clearColor()));
            window.setMovableByWindowBackground(true);
            window.center();
            window.setDelegate(Some(ProtocolObject::from_ref(self)));

            let root_glass = NSGlassEffectView::initWithFrame(
                NSGlassEffectView::alloc(mtm),
                NSRect::new(NSPoint::new(0.0, 0.0), WINDOW_SIZE),
            );
            root_glass.setStyle(NSGlassEffectViewStyle::Regular);
            root_glass.setTintColor(Some(&NSColor::windowBackgroundColor()));
            root_glass.setAutoresizingMask(
                NSAutoresizingMaskOptions::ViewWidthSizable
                    | NSAutoresizingMaskOptions::ViewHeightSizable,
            );

            let content = NSView::initWithFrame(
                NSView::alloc(mtm),
                NSRect::new(NSPoint::new(0.0, 0.0), WINDOW_SIZE),
            );
            content.setAutoresizingMask(
                NSAutoresizingMaskOptions::ViewWidthSizable
                    | NSAutoresizingMaskOptions::ViewHeightSizable,
            );
            root_glass.setContentView(Some(&content));

            let sidebar = NSGlassEffectView::initWithFrame(
                NSGlassEffectView::alloc(mtm),
                NSRect::new(
                    NSPoint::new(0.0, 0.0),
                    NSSize::new(SIDEBAR_WIDTH, WINDOW_SIZE.height),
                ),
            );
            sidebar.setStyle(NSGlassEffectViewStyle::Regular);
            sidebar.setAutoresizingMask(NSAutoresizingMaskOptions::ViewHeightSizable);
            let sidebar_content = NSView::initWithFrame(
                NSView::alloc(mtm),
                NSRect::new(
                    NSPoint::new(0.0, 0.0),
                    NSSize::new(SIDEBAR_WIDTH, WINDOW_SIZE.height),
                ),
            );
            sidebar_content.setAutoresizingMask(NSAutoresizingMaskOptions::ViewHeightSizable);
            sidebar.setContentView(Some(&sidebar_content));

            let create = unsafe {
                NSButton::buttonWithTitle_target_action(
                    ns_string!("Create Project"),
                    Some(self),
                    Some(sel!(createProject:)),
                    mtm,
                )
            };
            create.setFrame(NSRect::new(
                NSPoint::new(
                    (SIDEBAR_WIDTH - SIDEBAR_BUTTON_WIDTH) / 2.0,
                    WINDOW_SIZE.height - TITLEBAR_HEIGHT - CONTENT_MARGIN - SIDEBAR_BUTTON_HEIGHT,
                ),
                NSSize::new(SIDEBAR_BUTTON_WIDTH, SIDEBAR_BUTTON_HEIGHT),
            ));
            create.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinYMargin);
            create.setControlSize(NSControlSize::Regular);
            create.setBezelStyle(NSBezelStyle::Push);
            create.setKeyEquivalent(ns_string!("\r"));
            sidebar_content.addSubview(&create);

            let open = unsafe {
                NSButton::buttonWithTitle_target_action(
                    ns_string!("Open Project"),
                    Some(self),
                    Some(sel!(openProject:)),
                    mtm,
                )
            };
            open.setFrame(NSRect::new(
                NSPoint::new(
                    (SIDEBAR_WIDTH - SIDEBAR_BUTTON_WIDTH) / 2.0,
                    WINDOW_SIZE.height
                        - TITLEBAR_HEIGHT
                        - CONTENT_MARGIN * 2.0
                        - SIDEBAR_BUTTON_HEIGHT * 2.0,
                ),
                NSSize::new(SIDEBAR_BUTTON_WIDTH, SIDEBAR_BUTTON_HEIGHT),
            ));
            open.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinYMargin);
            open.setControlSize(NSControlSize::Regular);
            open.setBezelStyle(NSBezelStyle::Push);
            sidebar_content.addSubview(&open);
            content.addSubview(&sidebar);

            let right_origin = SIDEBAR_WIDTH + CONTENT_MARGIN;
            let right_width = WINDOW_SIZE.width - right_origin - CONTENT_MARGIN;
            let title = NSTextField::labelWithString(ns_string!("Shrimply"), mtm);
            title.setFont(Some(&NSFont::titleBarFontOfSize(0.0)));
            title.sizeToFit();
            let title_size = title.frame().size;
            title.setFrameOrigin(NSPoint::new(
                right_origin,
                WINDOW_SIZE.height - (TITLEBAR_HEIGHT + title_size.height) / 2.0,
            ));
            title.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinYMargin);
            content.addSubview(&title);

            let search = NSSearchField::initWithFrame(
                NSSearchField::alloc(mtm),
                NSRect::new(
                    NSPoint::new(
                        right_origin,
                        WINDOW_SIZE.height - TITLEBAR_HEIGHT - SEARCH_HEIGHT - CONTENT_MARGIN,
                    ),
                    NSSize::new(right_width - CLEAR_BUTTON_WIDTH - 8.0, SEARCH_HEIGHT),
                ),
            );
            search.setPlaceholderString(Some(ns_string!("Search history")));
            search.setContinuous(true);
            unsafe {
                search.setTarget(Some(self));
                search.setAction(Some(sel!(searchChanged:)));
            }
            search.setAutoresizingMask(
                NSAutoresizingMaskOptions::ViewWidthSizable
                    | NSAutoresizingMaskOptions::ViewMinYMargin,
            );
            content.addSubview(&search);

            let trash_image = NSImage::imageWithSystemSymbolName_accessibilityDescription(
                ns_string!("trash"),
                Some(ns_string!("Clear recent projects")),
            )
            .expect("macOS must provide the trash system symbol");
            let clear = unsafe {
                NSButton::buttonWithImage_target_action(
                    &trash_image,
                    Some(self),
                    Some(sel!(clearHistory:)),
                    mtm,
                )
            };
            clear.setFrame(NSRect::new(
                NSPoint::new(
                    WINDOW_SIZE.width - CONTENT_MARGIN - CLEAR_BUTTON_WIDTH,
                    WINDOW_SIZE.height - TITLEBAR_HEIGHT - SEARCH_HEIGHT - CONTENT_MARGIN,
                ),
                NSSize::new(CLEAR_BUTTON_WIDTH, SEARCH_HEIGHT),
            ));
            clear.setAutoresizingMask(
                NSAutoresizingMaskOptions::ViewMinXMargin
                    | NSAutoresizingMaskOptions::ViewMinYMargin,
            );
            clear.setBezelStyle(NSBezelStyle::Glass);
            clear.setToolTip(Some(ns_string!("Clear recent projects")));
            content.addSubview(&clear);

            let recent_list = recents::List::new(
                NSRect::new(
                    NSPoint::new(right_origin, CONTENT_MARGIN),
                    NSSize::new(
                        right_width,
                        WINDOW_SIZE.height
                            - TITLEBAR_HEIGHT
                            - SEARCH_HEIGHT
                            - CONTENT_MARGIN * 3.0,
                    ),
                ),
                mtm,
            );
            content.addSubview(recent_list.view());
            self.ivars()
                .search
                .set(search)
                .unwrap_or_else(|_| panic!("search field must only be created once"));
            self.ivars()
                .recent_list
                .set(recent_list)
                .unwrap_or_else(|_| panic!("recent list must only be created once"));

            window.setContentView(Some(&root_glass));
            window.makeKeyAndOrderFront(None);
            window.makeFirstResponder(None);
            self.ivars()
                .window
                .set(window)
                .expect("window must only be created once");
            self.refresh_recents("");

            app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
            app.activate();
        }
    }

    unsafe impl NSWindowDelegate for Delegate {
        #[unsafe(method(windowDidResize:))]
        fn window_did_resize(&self, _notification: &NSNotification) {
            if self.ivars().search.get().is_some() && self.ivars().recent_list.get().is_some() {
                self.refresh_current_search();
            }
        }

        #[unsafe(method(windowWillClose:))]
        fn window_will_close(&self, _notification: &NSNotification) {
            NSApplication::sharedApplication(self.mtm()).terminate(None);
        }
    }

    impl Delegate {
        #[unsafe(method(searchChanged:))]
        fn search_changed(&self, sender: &NSSearchField) {
            self.refresh_recents(&sender.stringValue().to_string());
        }

        #[unsafe(method(clearHistory:))]
        fn clear_history(&self, _sender: &NSButton) {
            launcher::clear_recent_projects()
                .unwrap_or_else(|error| panic!("could not clear recent projects: {error}"));
            self.refresh_current_search();
        }

        #[unsafe(method(openRecent:))]
        fn open_recent(&self, sender: &NSButton) {
            let project = self.recent_project(sender.tag());
            panic!(
                "the editor is not available on macOS; cannot open {}",
                project.path.display()
            );
        }

        #[unsafe(method(recentInfo:))]
        fn recent_info(&self, sender: &NSMenuItem) {
            let project = self.recent_project(sender.tag());
            let last_edited = launcher::last_edited(&project.path)
                .unwrap_or_else(|| "Unavailable".to_string());
            let alert = NSAlert::new(self.mtm());
            alert.setMessageText(&NSString::from_str(&project.name));
            alert.setInformativeText(&NSString::from_str(&format!(
                "Last Edited\n{last_edited}\n\nFile Location\n{}",
                project.path.display()
            )));
            alert.addButtonWithTitle(ns_string!("OK"));
            let completion = StackBlock::new({
                let mtm = self.mtm();
                move |response| NSApplication::sharedApplication(mtm).stopModalWithCode(response)
            });
            alert.beginSheetModalForWindow_completionHandler(
                self.ivars().window.get().expect("launcher window must exist"),
                Some(&completion),
            );
            alert.runModal();
        }

        #[unsafe(method(showRecentInFinder:))]
        fn show_recent_in_finder(&self, sender: &NSMenuItem) {
            let project = self.recent_project(sender.tag());
            let url = NSURL::fileURLWithPath(&NSString::from_str(&project.path.to_string_lossy()));
            NSWorkspace::sharedWorkspace()
                .activateFileViewerSelectingURLs(&NSArray::from_retained_slice(&[url]));
        }

        #[unsafe(method(removeRecent:))]
        fn remove_recent(&self, sender: &NSMenuItem) {
            let project = self.recent_project(sender.tag());
            launcher::remove_recent_project(&project.path)
                .unwrap_or_else(|error| panic!("could not remove recent project: {error}"));
            self.refresh_current_search();
        }

        #[unsafe(method(createProject:))]
        fn create_project(&self, _sender: &NSButton) {
            let window = self.ivars().window.get().expect("launcher window must exist");
            let Some(request) = create_project::show(window, self.mtm()) else {
                return;
            };
            let panel = NSSavePanel::savePanel(self.mtm());
            panel.setTitle(Some(ns_string!("Create Project")));
            panel.setPrompt(Some(ns_string!("Create")));
            panel.setNameFieldStringValue(&NSString::from_str(
                &launcher::default_project_filename(&request.name),
            ));
            panel.setCanCreateDirectories(true);
            let completion = StackBlock::new({
                let mtm = self.mtm();
                move |response| NSApplication::sharedApplication(mtm).stopModalWithCode(response)
            });
            panel.beginSheetModalForWindow_completionHandler(window, &completion);
            if panel.runModal() != NSModalResponseOK {
                return;
            }
            let path = panel
                .URL()
                .and_then(|url| url.path())
                .map(|path| PathBuf::from(path.to_string()))
                .expect("the selected save location must be a file path");
            let path = launcher::create_project(
                path,
                &request.name,
                request.canvas_size,
                request.fps,
            )
            .unwrap_or_else(|error| panic!("could not create project: {error}"));
            recent_projects::touch(&path, &request.name)
                .unwrap_or_else(|error| panic!("could not update recent projects: {error}"));
            self.refresh_current_search();
            panic!(
                "the editor is not available on macOS; created {}",
                path.display()
            );
        }

        #[unsafe(method(openProject:))]
        fn open_project(&self, _sender: &NSButton) {
            let panel = NSOpenPanel::openPanel(self.mtm());
            panel.setTitle(Some(ns_string!("Open Project")));
            panel.setPrompt(Some(ns_string!("Open")));
            panel.setCanChooseFiles(true);
            panel.setCanChooseDirectories(false);
            panel.setAllowsMultipleSelection(false);
            let completion = StackBlock::new({
                let mtm = self.mtm();
                move |response| NSApplication::sharedApplication(mtm).stopModalWithCode(response)
            });
            panel.beginSheetModalForWindow_completionHandler(
                self.ivars().window.get().expect("launcher window must exist"),
                &completion,
            );
            if panel.runModal() != NSModalResponseOK {
                return;
            }
            let path = panel
                .URL()
                .and_then(|url| url.path())
                .map(|path| PathBuf::from(path.to_string()))
                .expect("the selected project must be a file path");
            panic!(
                "the editor is not available on macOS; cannot open {}",
                path.display()
            );
        }
    }
);

impl Delegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(DelegateIvars {
            window: OnceCell::new(),
            search: OnceCell::new(),
            recent_list: OnceCell::new(),
            recent_projects: RefCell::new(Vec::new()),
        });
        unsafe { msg_send![super(this), init] }
    }

    fn refresh_current_search(&self) {
        let query = self
            .ivars()
            .search
            .get()
            .expect("search field must exist")
            .stringValue()
            .to_string();
        self.refresh_recents(&query);
    }

    fn refresh_recents(&self, query: &str) {
        let projects = self
            .ivars()
            .recent_list
            .get()
            .expect("recent list must exist")
            .refresh(self, query, self.mtm())
            .unwrap_or_else(|error| panic!("could not load recent projects: {error}"));
        *self.ivars().recent_projects.borrow_mut() = projects;
    }

    fn recent_project(&self, tag: isize) -> RecentProject {
        let index = usize::try_from(tag).expect("recent project index must be valid");
        self.ivars()
            .recent_projects
            .borrow()
            .get(index)
            .cloned()
            .expect("recent project must exist")
    }
}

pub fn run() {
    shrimply_support::diagnostics::init();
    let mtm = MainThreadMarker::new().expect("AppKit must start on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    let delegate = Delegate::new(mtm);
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    app.run();
}
