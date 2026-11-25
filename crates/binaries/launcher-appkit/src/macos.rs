mod create_project;
mod recents;

use block2::StackBlock;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{DefinedClass, MainThreadOnly, Message, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSAlertSecondButtonReturn, NSAlertThirdButtonReturn,
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSAutoresizingMaskOptions,
    NSBackingStoreType, NSBezelStyle, NSButton, NSColor, NSControlSize, NSFont, NSImage, NSMenu,
    NSMenuItem, NSModalResponseOK, NSOpenPanel, NSOpenSavePanelDelegate, NSPasteboard,
    NSPasteboardTypeString, NSSavePanel, NSSearchField, NSTextField, NSTitlebarSeparatorStyle,
    NSToolbar, NSView, NSWindow, NSWindowDelegate, NSWindowStyleMask, NSWindowTitleVisibility,
    NSWindowToolbarStyle, NSWorkspace,
};
use objc2_foundation::{
    MainThreadMarker, NSArray, NSNotification, NSObject, NSObjectProtocol, NSPoint, NSProcessInfo,
    NSRect, NSSize, NSString, NSTimer, NSURL, ns_string,
};
use shrimply_components_appkit::{EffectRole, EffectView};
use shrimply_cross_ui_core::launcher;
use shrimply_recent_projects::{self as recent_projects, RecentProject};
use std::cell::{OnceCell, RefCell};
use std::path::{Path, PathBuf};

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

            let root_effect = EffectView::new(
                NSRect::new(NSPoint::new(0.0, 0.0), WINDOW_SIZE),
                EffectRole::Window,
                mtm,
            );
            let root_glass = root_effect.view();
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
            root_effect.set_content_view(Some(&content));

            let sidebar_effect = EffectView::new(
                NSRect::new(
                    NSPoint::new(0.0, 0.0),
                    NSSize::new(SIDEBAR_WIDTH, WINDOW_SIZE.height),
                ),
                EffectRole::Sidebar,
                mtm,
            );
            let sidebar = sidebar_effect.view();
            sidebar.setAutoresizingMask(NSAutoresizingMaskOptions::ViewHeightSizable);
            let sidebar_content = NSView::initWithFrame(
                NSView::alloc(mtm),
                NSRect::new(
                    NSPoint::new(0.0, 0.0),
                    NSSize::new(SIDEBAR_WIDTH, WINDOW_SIZE.height),
                ),
            );
            sidebar_content.setAutoresizingMask(NSAutoresizingMaskOptions::ViewHeightSizable);
            sidebar_effect.set_content_view(Some(&sidebar_content));

            for (index, (label, action)) in [
                ("Create Project", sel!(createProject:)),
                ("Open Project", sel!(openProject:)),
                ("Open Shared Project", sel!(openSharedProject:)),
                ("Project Servers", sel!(projectServers:)),
            ].into_iter().enumerate() {
                let button = unsafe {
                    NSButton::buttonWithTitle_target_action(
                        &NSString::from_str(label), Some(self), Some(action), mtm,
                    )
                };
                button.setFrame(NSRect::new(
                    NSPoint::new(
                        (SIDEBAR_WIDTH - SIDEBAR_BUTTON_WIDTH) / 2.0,
                        WINDOW_SIZE.height - TITLEBAR_HEIGHT
                            - (index + 1) as f64 * (CONTENT_MARGIN + SIDEBAR_BUTTON_HEIGHT),
                    ),
                    NSSize::new(SIDEBAR_BUTTON_WIDTH, SIDEBAR_BUTTON_HEIGHT),
                ));
                button.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinYMargin);
                button.setControlSize(NSControlSize::Regular);
                button.setBezelStyle(NSBezelStyle::Push);
                if index == 0 {
                    button.setKeyEquivalent(ns_string!("\r"));
                }
                sidebar_content.addSubview(&button);
            }
            content.addSubview(sidebar);

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

            window.setContentView(Some(root_glass));
            window.makeKeyAndOrderFront(None);
            window.makeFirstResponder(None);
            self.ivars()
                .window
                .set(window)
                .expect("window must only be created once");
            self.refresh_recents("");

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

    unsafe impl NSOpenSavePanelDelegate for Delegate {
        #[unsafe(method(panel:shouldEnableURL:))]
        fn should_enable_project(&self, _sender: &objc2::runtime::AnyObject, url: &NSURL) -> bool {
            url.hasDirectoryPath() || url.lastPathComponent().is_some_and(|name| {
                let name = name.to_string().to_lowercase();
                launcher::PROJECT_FILE_PATTERNS.iter()
                    .any(|pattern| name.ends_with(pattern.trim_start_matches('*')))
            })
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
            self.open_in_editor(&project.path);
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
            let link = launcher::shared_project_link(&project.path).ok().flatten();
            if link.is_some() {
                alert.addButtonWithTitle(ns_string!("Copy Project Link"));
            }
            let completion = StackBlock::new({
                let mtm = self.mtm();
                move |response| NSApplication::sharedApplication(mtm).stopModalWithCode(response)
            });
            alert.beginSheetModalForWindow_completionHandler(
                self.ivars().window.get().expect("launcher window must exist"),
                Some(&completion),
            );
            if alert.runModal() == NSAlertSecondButtonReturn
                && let Some(link) = link
            {
                let clipboard = NSPasteboard::generalPasteboard();
                clipboard.clearContents();
                clipboard.setString_forType(&NSString::from_str(&link), unsafe { NSPasteboardTypeString });
            }
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
            if let Some(server) = request.server {
                self.open_remote("Could not create project", move || {
                    launcher::create_shared_project(
                        &server, &request.name, request.canvas_size, request.fps, &[],
                    )
                });
                return;
            }
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
            let path = match launcher::create_project(
                path, &request.name, request.canvas_size, request.fps, &[],
            ) {
                Ok(path) => path,
                Err(error) => {
                    self.show_error("Could not create project", &error);
                    return;
                }
            };
            recent_projects::touch(&path, &request.name, &[])
                .unwrap_or_else(|error| panic!("could not update recent projects: {error}"));
            self.refresh_current_search();
            self.open_in_editor(&path);
        }

        #[unsafe(method(openSharedProject:))]
        fn open_shared_project(&self, _sender: &NSButton) {
            if let Some(link) = self.prompt_remote("Open Shared Project", "Project Link", "") {
                self.open_remote("Could not open project", move || launcher::open_shared_project(&link));
            }
        }

        #[unsafe(method(projectServers:))]
        fn project_servers(&self, _sender: &NSButton) {
            let mtm = self.mtm();
            loop {
                let servers = launcher::project_servers();
                let alert = NSAlert::new(mtm);
                alert.setMessageText(ns_string!("Project Servers"));
                alert.addButtonWithTitle(ns_string!("Add"));
                let edit = alert.addButtonWithTitle(ns_string!("Edit"));
                let remove = alert.addButtonWithTitle(ns_string!("Remove"));
                alert.addButtonWithTitle(ns_string!("Done"));
                edit.setEnabled(!servers.is_empty());
                remove.setEnabled(!servers.is_empty());
                let selection = shrimply_components_appkit::menu::popup(
                    NSRect::new(NSPoint::ZERO, NSSize::new(360.0, 30.0)), false, mtm,
                );
                for server in &servers {
                    selection.addItemWithTitle(&NSString::from_str(server));
                }
                if !servers.is_empty() {
                    alert.setAccessoryView(Some(&selection));
                }
                let response = alert.runModal();
                let selected = usize::try_from(selection.indexOfSelectedItem()).ok()
                    .and_then(|index| servers.get(index)).map(String::as_str);
                let result = match response {
                    value if value == NSAlertFirstButtonReturn => self.prompt_remote("Add Project Server", "Server URL", "")
                        .map(|url| launcher::update_project_servers(None, Some(&url))),
                    value if value == NSAlertSecondButtonReturn => selected.and_then(|current| {
                        self.prompt_remote("Edit Project Server", "Server URL", current)
                            .map(|url| launcher::update_project_servers(Some(current), Some(&url)))
                    }),
                    value if value == NSAlertThirdButtonReturn => selected.map(|current| launcher::update_project_servers(Some(current), None)),
                    _ => return,
                };
                if let Some(Err(error)) = result {
                    self.show_error("Could not update project server", &error);
                }
            }
        }

        #[unsafe(method(openProject:))]
        fn open_project(&self, _sender: &NSButton) {
            let panel = NSOpenPanel::openPanel(self.mtm());
            unsafe { panel.setDelegate(Some(ProtocolObject::from_ref(self))) };
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
            self.open_in_editor(&path);
        }
    }
);

impl Delegate {
    fn open_in_editor(&self, path: &Path) {
        let mut child = match launcher::launch_appkit_editor(path) {
            Ok(child) => child,
            Err(error) => {
                self.show_error("Could not start editor", &error);
                return;
            }
        };
        let window_number = self
            .ivars()
            .window
            .get()
            .expect("launcher window must exist")
            .windowNumber();
        self.ivars()
            .window
            .get()
            .expect("launcher window must exist")
            .orderOut(None);
        let app = NSApplication::sharedApplication(self.mtm());
        app.hide(None);
        // Keep the launcher available for cancellation without a second Dock icon.
        assert!(app.setActivationPolicy(NSApplicationActivationPolicy::Accessory));
        std::thread::spawn(move || {
            let result = child.wait();
            let completed = block2::RcBlock::new(move || match &result {
                Ok(status)
                    if status.code()
                        == Some(i32::from(
                            shrimply_cross_ui_core::launcher::EDITOR_OPEN_CANCELED_EXIT_CODE,
                        )) =>
                {
                    let mtm =
                        MainThreadMarker::new().expect("completion must run on the main thread");
                    let app = NSApplication::sharedApplication(mtm);
                    assert!(app.setActivationPolicy(NSApplicationActivationPolicy::Regular));
                    app.unhide(None);
                    app.windowWithWindowNumber(window_number)
                        .expect("launcher window must still exist")
                        .makeKeyAndOrderFront(None);
                    app.activate();
                }
                Ok(status) if status.success() => {
                    let mtm =
                        MainThreadMarker::new().expect("completion must run on the main thread");
                    NSApplication::sharedApplication(mtm).terminate(None);
                }
                Ok(status) => {
                    eprintln!("editor exited with {status}");
                    std::process::exit(status.code().unwrap_or(1));
                }
                Err(error) => {
                    eprintln!("could not wait for editor: {error}");
                    std::process::exit(1);
                }
            });
            unsafe {
                objc2_foundation::NSOperationQueue::mainQueue().addOperationWithBlock(&completed)
            };
        });
    }

    fn prompt_remote(&self, heading: &str, placeholder: &str, value: &str) -> Option<String> {
        let mtm = self.mtm();
        let alert = NSAlert::new(mtm);
        alert.setMessageText(&NSString::from_str(heading));
        alert.addButtonWithTitle(ns_string!("Continue"));
        alert.addButtonWithTitle(ns_string!("Cancel"));
        let field = NSTextField::initWithFrame(
            NSTextField::alloc(mtm),
            NSRect::new(NSPoint::ZERO, NSSize::new(360.0, 28.0)),
        );
        field.setPlaceholderString(Some(&NSString::from_str(placeholder)));
        field.setStringValue(&NSString::from_str(value));
        alert.setAccessoryView(Some(&field));
        (alert.runModal() == NSAlertFirstButtonReturn).then(|| field.stringValue().to_string())
    }

    fn open_remote(
        &self,
        heading: &'static str,
        operation: impl FnOnce() -> Result<PathBuf, String> + Send + 'static,
    ) {
        let (send, receive) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = send.send(operation());
        });
        let delegate = self.retain();
        let poll = block2::RcBlock::new(move |timer: std::ptr::NonNull<NSTimer>| {
            let result = match receive.try_recv() {
                Ok(result) => result,
                Err(std::sync::mpsc::TryRecvError::Empty) => return,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    Err("The project worker stopped unexpectedly.".into())
                }
            };
            unsafe { timer.as_ref() }.invalidate();
            match result {
                Ok(path) => delegate.open_in_editor(&path),
                Err(error) => delegate.show_error(heading, &error),
            }
        });
        let _ = unsafe { NSTimer::scheduledTimerWithTimeInterval_repeats_block(0.1, true, &poll) };
    }

    fn show_error(&self, heading: &str, body: &str) {
        let alert = NSAlert::new(self.mtm());
        alert.setMessageText(&NSString::from_str(heading));
        alert.setInformativeText(&NSString::from_str(body));
        alert.runModal();
    }

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
    shrimply_process_reporting::diagnostics::init();
    let mtm = MainThreadMarker::new().expect("AppKit must start on the main thread");
    NSProcessInfo::processInfo().setProcessName(ns_string!("Shrimply"));
    NSWindow::setAllowsAutomaticWindowTabbing(false, mtm);
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    let delegate = Delegate::new(mtm);
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    app.run();
}
