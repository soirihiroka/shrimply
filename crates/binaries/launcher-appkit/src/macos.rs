mod create_project;

use block2::StackBlock;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{DefinedClass, MainThreadOnly, Message, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSAlertSecondButtonReturn, NSAlertThirdButtonReturn,
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSAutoresizingMaskOptions,
    NSBackingStoreType, NSBezelStyle, NSButton, NSCellImagePosition, NSColor, NSControlSize,
    NSControlTextEditingDelegate, NSFont, NSImage, NSMenu, NSMenuItem, NSModalResponseOK,
    NSOpenPanel, NSOpenSavePanelDelegate, NSSavePanel, NSScrollView, NSSearchField, NSTableColumn,
    NSTableView, NSTableViewDataSource, NSTableViewDelegate, NSTextAlignment, NSTextField,
    NSTitlebarSeparatorStyle, NSToolbar, NSView, NSWindow, NSWindowDelegate, NSWindowStyleMask,
    NSWindowTitleVisibility, NSWindowToolbarStyle,
};
use objc2_foundation::{
    MainThreadMarker, NSNotification, NSObject, NSObjectProtocol, NSPoint, NSProcessInfo, NSRect,
    NSSize, NSString, NSTimer, NSURL, ns_string,
};
use shrimply_components_appkit::{EffectRole, EffectView};
use shrimply_cross_ui_core::launcher::{self, Launcher};
use std::cell::{OnceCell, RefCell};
use std::path::{Path, PathBuf};

const WINDOW_SIZE: NSSize = NSSize::new(760.0, 560.0);
const MINIMUM_WINDOW_SIZE: NSSize = NSSize::new(560.0, 420.0);
const SIDEBAR_WIDTH: f64 = 330.0;
const CONTENT_MARGIN: f64 = 12.0;
const TITLEBAR_HEIGHT: f64 = 52.0;
const SEARCH_HEIGHT: f64 = 32.0;
const SIDEBAR_BUTTON_WIDTH: f64 = 160.0;
const SIDEBAR_BUTTON_HEIGHT: f64 = 32.0;

struct DelegateIvars {
    window: OnceCell<Retained<NSWindow>>,
    projects: OnceCell<Retained<NSTableView>>,
    model: RefCell<Launcher>,
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
                ("Project Servers", sel!(projectServers:)),
            ].into_iter().enumerate() {
                let button = unsafe {
                    NSButton::buttonWithTitle_target_action(
                        &NSString::from_str(label), Some(self), Some(action), mtm,
                    )
                };
                button.setFrame(NSRect::new(
                    NSPoint::new(
                        (WINDOW_SIZE.width + SIDEBAR_WIDTH - SIDEBAR_BUTTON_WIDTH) / 2.0,
                        WINDOW_SIZE.height - TITLEBAR_HEIGHT
                            - (index + 1) as f64 * (CONTENT_MARGIN + SIDEBAR_BUTTON_HEIGHT),
                    ),
                    NSSize::new(SIDEBAR_BUTTON_WIDTH, SIDEBAR_BUTTON_HEIGHT),
                ));
                button.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinYMargin
                    | NSAutoresizingMaskOptions::ViewMinXMargin | NSAutoresizingMaskOptions::ViewMaxXMargin);
                button.setControlSize(NSControlSize::Regular);
                button.setBezelStyle(NSBezelStyle::Push);
                if index == 0 {
                    button.setKeyEquivalent(ns_string!("\r"));
                }
                content.addSubview(&button);
            }
            content.addSubview(sidebar);

            let width = SIDEBAR_WIDTH - CONTENT_MARGIN * 2.0;
            let title = NSTextField::labelWithString(ns_string!("Projects"), mtm);
            title.setFont(Some(&NSFont::titleBarFontOfSize(0.0)));
            title.sizeToFit();
            title.setFrameOrigin(NSPoint::new(CONTENT_MARGIN,
                WINDOW_SIZE.height - (TITLEBAR_HEIGHT + title.frame().size.height) / 2.0));
            title.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinYMargin);
            sidebar_content.addSubview(&title);

            let search = NSSearchField::initWithFrame(NSSearchField::alloc(mtm), NSRect::new(
                NSPoint::new(CONTENT_MARGIN, WINDOW_SIZE.height - TITLEBAR_HEIGHT - SEARCH_HEIGHT),
                NSSize::new(width, SEARCH_HEIGHT),
            ));
            search.setPlaceholderString(Some(ns_string!("Search Projects")));
            search.setContinuous(true);
            search.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinYMargin);
            unsafe {
                search.setTarget(Some(self));
                search.setAction(Some(sel!(searchChanged:)));
            }
            sidebar_content.addSubview(&search);

            let frame = NSRect::new(NSPoint::new(CONTENT_MARGIN, CONTENT_MARGIN), NSSize::new(
                width, WINDOW_SIZE.height - TITLEBAR_HEIGHT - SEARCH_HEIGHT - CONTENT_MARGIN * 2.0,
            ));
            let scroll = NSScrollView::initWithFrame(NSScrollView::alloc(mtm), frame);
            scroll.setDrawsBackground(false);
            scroll.setHasVerticalScroller(true);
            scroll.setAutoresizingMask(NSAutoresizingMaskOptions::ViewHeightSizable);
            let table = NSTableView::initWithFrame(NSTableView::alloc(mtm), frame);
            table.setBackgroundColor(&NSColor::clearColor());
            table.setHeaderView(None);
            table.setRowHeight(44.0);
            let column = NSTableColumn::initWithIdentifier(NSTableColumn::alloc(mtm), ns_string!("project"));
            column.setWidth(width);
            table.addTableColumn(&column);
            unsafe {
                table.setDataSource(Some(ProtocolObject::from_ref(self)));
                table.setDelegate(Some(ProtocolObject::from_ref(self)));
            }
            scroll.setDocumentView(Some(&table));
            sidebar_content.addSubview(&scroll);
            self.ivars().projects.set(table).expect("project list must only be created once");

            window.setContentView(Some(root_glass));
            window.makeKeyAndOrderFront(None);
            window.makeFirstResponder(None);
            self.ivars()
                .window
                .set(window)
                .expect("window must only be created once");
            if let Ok(projects) = launcher::local_projects() {
                self.ivars().model.borrow_mut().set_projects(projects);
                self.ivars().projects.get().expect("project list").reloadData();
            }
            self.reload_projects();

            app.activate();
        }
    }

    unsafe impl NSWindowDelegate for Delegate {
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

    unsafe impl NSTableViewDataSource for Delegate {
        #[unsafe(method(numberOfRowsInTableView:))]
        fn project_count(&self, _table: &NSTableView) -> isize {
            self.ivars().model.borrow().project_count() as isize
        }
    }

    unsafe impl NSControlTextEditingDelegate for Delegate {}

    unsafe impl NSTableViewDelegate for Delegate {
        #[unsafe(method(tableView:viewForTableColumn:row:))]
        fn project_row(&self, _table: &NSTableView, _column: Option<&NSTableColumn>, row: isize) -> Option<Retained<NSView>> {
            let model = self.ivars().model.borrow();
            let project = model.project(usize::try_from(row).ok()?)?;
            let button = unsafe {
                NSButton::buttonWithTitle_target_action(
                    &NSString::from_str(&project.name), Some(self), Some(sel!(openListedProject:)), self.mtm(),
                )
            };
            let symbol = if let Some(remote) = &project.remote {
                button.setToolTip(Some(&NSString::from_str(&remote.server)));
                ns_string!("externaldrive.connected.to.line.below")
            } else {
                ns_string!("folder")
            };
            let icon = NSImage::imageWithSystemSymbolName_accessibilityDescription(symbol, None);
            button.setImage(icon.as_deref());
            button.setImagePosition(NSCellImagePosition::ImageLeading);
            button.setAlignment(NSTextAlignment::Left);
            button.setBordered(false);
            button.setTag(row);
            Some(button.into_super().into_super())
        }
    }

    impl Delegate {
        #[unsafe(method(searchChanged:))]
        fn search_changed(&self, sender: &NSSearchField) {
            self.ivars().model.borrow_mut().set_search(&sender.stringValue().to_string());
            self.ivars().projects.get().expect("project list").reloadData();
        }

        #[unsafe(method(openListedProject:))]
        fn open_listed_project(&self, sender: &NSButton) {
            if let Some(project) = self.ivars().model.borrow().project(sender.tag() as usize).cloned() {
                self.open_remote("Could not open project", move || project.open(None));
            }
        }

        #[unsafe(method(createProject:))]
        fn create_project(&self, _sender: &NSButton) {
            let window = self.ivars().window.get().expect("launcher window must exist");
            let Some(request) = create_project::show(window, self.mtm()) else {
                return;
            };
            if let Some(server) = request.server {
                self.open_remote("Could not create project", move || {
                    launcher::create_remote_project(
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
            self.open_in_editor(&path);
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
                    _ => { self.reload_projects(); return; },
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
        self.project_task(operation, move |delegate, result| match result {
            Ok(path) => delegate.open_in_editor(&path),
            Err(error) => delegate.show_error(heading, &error),
        });
    }

    fn reload_projects(&self) {
        let load = self.ivars().model.borrow_mut().begin_project_load();
        self.project_task(launcher::load_projects, move |delegate, projects| {
            let result = delegate
                .ivars()
                .model
                .borrow_mut()
                .finish_project_load(load, projects);
            match result {
                Ok(true) => delegate
                    .ivars()
                    .projects
                    .get()
                    .expect("project list")
                    .reloadData(),
                Ok(false) => (),
                Err(error) => delegate.show_error("Could not load projects", &error),
            }
        });
    }

    fn project_task<T: Send + 'static>(
        &self,
        operation: impl FnOnce() -> Result<T, String> + Send + 'static,
        finish: impl Fn(&Self, Result<T, String>) + 'static,
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
            finish(&delegate, result);
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
            projects: OnceCell::new(),
            model: RefCell::new(Launcher::default()),
        });
        unsafe { msg_send![super(this), init] }
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
