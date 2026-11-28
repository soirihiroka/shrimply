mod create_project;

use block2::StackBlock;
use objc2::rc::{Retained, Weak};
use objc2::runtime::{ProtocolObject, Sel};
use objc2::{DefinedClass, MainThreadOnly, Message, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAccessibility, NSAlert, NSAlertSecondButtonReturn, NSApplication,
    NSApplicationActivationPolicy, NSApplicationDelegate, NSAutoresizingMaskOptions,
    NSBackingStoreType, NSBezelStyle, NSButton, NSColor, NSControlSize,
    NSControlTextEditingDelegate, NSEventModifierFlags, NSFont, NSImageView, NSMenu, NSMenuItem,
    NSModalResponseOK, NSOpenPanel, NSOpenSavePanelDelegate, NSPopUpArrowPosition, NSPopUpButton,
    NSPopUpButtonCell, NSSavePanel, NSScrollView, NSScrollerStyle, NSSearchField,
    NSSplitViewController, NSSplitViewItem, NSTableCellView, NSTableColumn, NSTableView,
    NSTableViewColumnAutoresizingStyle, NSTableViewDataSource, NSTableViewDelegate,
    NSTableViewRowSizeStyle, NSTableViewStyle, NSTextField, NSTitlebarSeparatorStyle, NSToolbar,
    NSView, NSViewController, NSWindow, NSWindowDelegate, NSWindowStyleMask,
    NSWindowTitleVisibility, NSWindowToolbarStyle, NSWorkspace,
};
use objc2_foundation::{
    MainThreadMarker, NSArray, NSIndexSet, NSNotification, NSObject, NSObjectProtocol, NSPoint,
    NSProcessInfo, NSRect, NSRectEdge, NSSize, NSString, NSTimer, NSURL, ns_string,
};
use shrimply_components_appkit::{bundled_symbol, column_stack, row_stack};
use shrimply_cross_ui_core::launcher::{self, Launcher, ProjectLocation};
use std::cell::{OnceCell, RefCell};
use std::path::{Path, PathBuf};

const WINDOW_SIZE: NSSize = NSSize::new(760.0, 560.0);
const MINIMUM_WINDOW_SIZE: NSSize = NSSize::new(560.0, 420.0);
const SIDEBAR_WIDTH: f64 = 212.0;
const CONTENT_MARGIN: f64 = 12.0;
const TITLEBAR_HEIGHT: f64 = 52.0;
const SEARCH_HEIGHT: f64 = 32.0;
const MENU_SIZE: f64 = 32.0;
const SIDEBAR_ICON_SIZE: f64 = 56.0;

struct DelegateIvars {
    window: OnceCell<Retained<NSWindow>>,
    projects: OnceCell<Retained<NSTableView>>,
    sources: OnceCell<Retained<NSTableView>>,
    model: RefCell<Launcher>,
    preferences: OnceCell<Retained<shrimply_editor_appkit::PreferencesWindow>>,
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
            window.setMovableByWindowBackground(true);
            window.center();
            window.setDelegate(Some(ProtocolObject::from_ref(self)));

            let content_width = WINDOW_SIZE.width - SIDEBAR_WIDTH;
            let content = NSView::initWithFrame(
                NSView::alloc(mtm),
                NSRect::new(NSPoint::ZERO, NSSize::new(content_width, WINDOW_SIZE.height)),
            );
            let sidebar = NSView::initWithFrame(
                NSView::alloc(mtm),
                NSRect::new(NSPoint::ZERO, NSSize::new(SIDEBAR_WIDTH, WINDOW_SIZE.height)),
            );

            let identity = row_stack(8.0, mtm);
            let icon = NSImageView::new(mtm);
            icon.setImage(app.applicationIconImage().as_deref());
            identity.addArrangedSubview(&icon);
            icon.widthAnchor().constraintEqualToConstant(SIDEBAR_ICON_SIZE).setActive(true);
            icon.heightAnchor().constraintEqualToConstant(SIDEBAR_ICON_SIZE).setActive(true);
            let labels = column_stack(3.0, mtm);
            let name = NSTextField::labelWithString(ns_string!("Shrimply"), mtm);
            name.setFont(Some(&NSFont::boldSystemFontOfSize(NSFont::systemFontSize())));
            labels.addArrangedSubview(&name);
            let version = NSTextField::labelWithString(&NSString::from_str(&launcher::version()), mtm);
            version.setFont(Some(&NSFont::systemFontOfSize(NSFont::smallSystemFontSize())));
            version.setTextColor(Some(&NSColor::secondaryLabelColor()));
            labels.addArrangedSubview(&version);
            identity.addArrangedSubview(&labels);
            let identity_y = WINDOW_SIZE.height - TITLEBAR_HEIGHT - SIDEBAR_ICON_SIZE;
            identity.setFrame(NSRect::new(
                NSPoint::new(CONTENT_MARGIN, identity_y),
                NSSize::new(SIDEBAR_WIDTH - 2.0 * CONTENT_MARGIN, SIDEBAR_ICON_SIZE),
            ));
            identity.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinYMargin);
            sidebar.addSubview(&identity);

            let menu = self.popup(
                NSPoint::new(CONTENT_MARGIN, CONTENT_MARGIN),
                ("open-menu-symbolic", "Main Menu"),
                &[
                    ("Settings", sel!(showPreferences:)),
                    ("About Shrimply", sel!(showAbout:)),
                ],
                None,
            );
            sidebar.addSubview(&menu);
            let sources_y = CONTENT_MARGIN * 2.0 + MENU_SIZE;
            let sources_frame = NSRect::new(
                NSPoint::new(0.0, sources_y),
                NSSize::new(SIDEBAR_WIDTH, identity_y - sources_y - CONTENT_MARGIN),
            );
            let source_scroll = NSScrollView::initWithFrame(NSScrollView::alloc(mtm), sources_frame);
            source_scroll.setDrawsBackground(false);
            source_scroll.setHasVerticalScroller(true);
            source_scroll.setAutohidesScrollers(true);
            source_scroll.setAutoresizingMask(NSAutoresizingMaskOptions::ViewHeightSizable);
            let sources = NSTableView::initWithFrame(
                NSTableView::alloc(mtm), NSRect::new(NSPoint::ZERO, source_scroll.contentSize())
            );
            sources.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
            sources.setHeaderView(None);
            sources.setStyle(NSTableViewStyle::Inset);
            sources.setBackgroundColor(&NSColor::clearColor());
            sources.setColumnAutoresizingStyle(NSTableViewColumnAutoresizingStyle::LastColumnOnlyAutoresizingStyle);
            let column = NSTableColumn::initWithIdentifier(NSTableColumn::alloc(mtm), ns_string!("source"));
            column.setWidth(source_scroll.contentSize().width);
            sources.addTableColumn(&column);
            self.ivars().sources.set(sources.clone()).expect("create source list once");
            unsafe {
                sources.setDataSource(Some(ProtocolObject::from_ref(self)));
                sources.setDelegate(Some(ProtocolObject::from_ref(self)));
            }
            source_scroll.setDocumentView(Some(&sources));
            sidebar.addSubview(&source_scroll);

            let x = CONTENT_MARGIN;
            let width = content_width - 2.0 * CONTENT_MARGIN;
            let title = NSTextField::labelWithString(ns_string!("Projects"), mtm);
            title.setFont(Some(&NSFont::titleBarFontOfSize(0.0)));
            title.sizeToFit();
            title.setFrameOrigin(NSPoint::new(x,
                WINDOW_SIZE.height - (TITLEBAR_HEIGHT + title.frame().size.height) / 2.0));
            title.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinYMargin);
            content.addSubview(&title);

            for (index, (symbol, title, action, key)) in [
                ("list-add-symbolic", "Create Project", sel!(createProject:), ns_string!("n")),
                ("folder-open-symbolic", "Open Project", sel!(openProject:), ns_string!("o")),
            ].into_iter().enumerate() {
                let icon = bundled_symbol(symbol, title);
                let button = unsafe {
                    NSButton::buttonWithImage_target_action(&icon, Some(self), Some(action), mtm)
                };
                let title = NSString::from_str(title);
                button.setToolTip(Some(&title));
                button.setAccessibilityLabel(Some(&title));
                button.setKeyEquivalent(key);
                button.setKeyEquivalentModifierMask(NSEventModifierFlags::Command);
                button.setBezelStyle(NSBezelStyle::Push);
                button.setControlSize(NSControlSize::Regular);
                button.setFrame(NSRect::new(NSPoint::new(
                    content_width - CONTENT_MARGIN - (2 - index) as f64 * MENU_SIZE - (1 - index) as f64 * 6.0,
                    WINDOW_SIZE.height - (TITLEBAR_HEIGHT + MENU_SIZE) / 2.0
                ), NSSize::new(MENU_SIZE, MENU_SIZE)));
                button.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinXMargin | NSAutoresizingMaskOptions::ViewMinYMargin);
                content.addSubview(&button);
            }

            let search = NSSearchField::initWithFrame(NSSearchField::alloc(mtm), NSRect::new(
                NSPoint::new(x, WINDOW_SIZE.height - TITLEBAR_HEIGHT - SEARCH_HEIGHT),
                NSSize::new(width, SEARCH_HEIGHT),
            ));
            search.setPlaceholderString(Some(ns_string!("Search Projects")));
            search.setContinuous(true);
            search.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinYMargin
                | NSAutoresizingMaskOptions::ViewWidthSizable);
            unsafe {
                search.setTarget(Some(self));
                search.setAction(Some(sel!(searchChanged:)));
            }
            content.addSubview(&search);

            let frame = NSRect::new(NSPoint::ZERO, NSSize::new(
                content_width, WINDOW_SIZE.height - TITLEBAR_HEIGHT - SEARCH_HEIGHT - CONTENT_MARGIN,
            ));
            let scroll = NSScrollView::initWithFrame(NSScrollView::alloc(mtm), frame);
            scroll.setDrawsBackground(false);
            scroll.setHasVerticalScroller(true);
            scroll.setScrollerStyle(NSScrollerStyle::Legacy);
            scroll.setAutohidesScrollers(true);
            scroll.setAutoresizingMask(NSAutoresizingMaskOptions::ViewHeightSizable
                | NSAutoresizingMaskOptions::ViewWidthSizable);
            let list_size = scroll.contentSize();
            let table = NSTableView::initWithFrame(NSTableView::alloc(mtm),
                NSRect::new(NSPoint::new(0.0, 0.0), list_size));
            table.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
            table.setBackgroundColor(&NSColor::clearColor());
            table.setHeaderView(None);
            table.setStyle(NSTableViewStyle::Inset);
            table.setColumnAutoresizingStyle(NSTableViewColumnAutoresizingStyle::LastColumnOnlyAutoresizingStyle);
            table.setRowSizeStyle(NSTableViewRowSizeStyle::Custom);
            table.setRowHeight(50.0);
            table.setIntercellSpacing(NSSize::new(0.0, 7.0));
            let column = NSTableColumn::initWithIdentifier(NSTableColumn::alloc(mtm), ns_string!("project"));
            column.setWidth(list_size.width);
            table.addTableColumn(&column);
            unsafe {
                table.setDataSource(Some(ProtocolObject::from_ref(self)));
                table.setDelegate(Some(ProtocolObject::from_ref(self)));
            }
            scroll.setDocumentView(Some(&table));
            content.addSubview(&scroll);
            self.ivars().projects.set(table).expect("project list must only be created once");

            let sidebar_controller = NSViewController::new(mtm);
            sidebar_controller.setView(&sidebar);
            let sidebar_item = NSSplitViewItem::sidebarWithViewController(&sidebar_controller);
            sidebar_item.setMinimumThickness(SIDEBAR_WIDTH);
            sidebar_item.setMaximumThickness(SIDEBAR_WIDTH);
            sidebar_item.setCanCollapse(false);
            let content_controller = NSViewController::new(mtm);
            content_controller.setView(&content);
            let content_item = NSSplitViewItem::splitViewItemWithViewController(&content_controller);
            let split = NSSplitViewController::new(mtm);
            split.splitView().setVertical(true);
            split.addSplitViewItem(&sidebar_item);
            split.addSplitViewItem(&content_item);
            split.view().setFrame(NSRect::new(NSPoint::ZERO, WINDOW_SIZE));
            window.setContentViewController(Some(&split));
            self.ivars().window.set(window.clone()).expect("window must only be created once");
            window.makeKeyAndOrderFront(None);
            window.makeFirstResponder(None);
            if let Ok(catalog) = launcher::local_catalog() {
                self.ivars().model.borrow_mut().set_catalog(catalog);
            }
            self.reload_lists();
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
        fn project_count(&self, table: &NSTableView) -> isize {
            let model = self.ivars().model.borrow();
            if self.ivars().sources.get().is_some_and(|sources| std::ptr::eq(&**sources, table)) {
                return model.sources().count() as isize;
            }
            model.project_count() as isize
        }
    }

    unsafe impl NSControlTextEditingDelegate for Delegate {}

    unsafe impl NSTableViewDelegate for Delegate {
        #[unsafe(method(tableViewSelectionDidChange:))]
        fn source_changed(&self, notification: &NSNotification) {
            let sources = self.ivars().sources.get().expect("source list");
            if !notification.object()
                .and_then(|object| object.downcast::<NSTableView>().ok())
                .is_some_and(|table| std::ptr::eq(&*table, &**sources))
            {
                return;
            }
            let mut model = self.ivars().model.borrow_mut();
            let Some((source, _)) = model.sources().nth(sources.selectedRow() as usize) else { return; };
            let source = source.to_owned();
            if model.source() != source {
                model.set_source(source);
                drop(model);
                let projects = self.ivars().projects.get().expect("project list");
                projects.reloadData();
                if projects.numberOfRows() > 0 {
                    projects.scrollRowToVisible(0);
                }
            }
        }

        #[unsafe(method_id(tableView:viewForTableColumn:row:))]
        fn project_row(&self, table: &NSTableView, column: Option<&NSTableColumn>, row: isize) -> Option<Retained<NSView>> {
            let model = self.ivars().model.borrow();
            usize::try_from(row).ok().and_then(|index| {
                if self.ivars().sources.get().is_some_and(|sources| std::ptr::eq(&**sources, table)) {
                    let (url, name) = model.sources().nth(index)?;
                    let label = NSTextField::labelWithString(&NSString::from_str(name), self.mtm());
                    if url.starts_with("http") {
                        label.setToolTip(Some(&NSString::from_str(url)));
                    }
                    let cell = NSTableCellView::new(self.mtm());
                    label.setTranslatesAutoresizingMaskIntoConstraints(false);
                    cell.addSubview(&label);
                    unsafe { cell.setTextField(Some(&label)) };
                    for constraint in [
                        label.leadingAnchor().constraintEqualToAnchor_constant(&cell.leadingAnchor(), 8.0),
                        label.trailingAnchor().constraintEqualToAnchor_constant(&cell.trailingAnchor(), -8.0),
                        label.centerYAnchor().constraintEqualToAnchor(&cell.centerYAnchor()),
                    ] {
                        constraint.setActive(true);
                    }
                    return Some(cell.into_super());
                }
                let project = model.project(index)?;
                let width = column.map_or(table.bounds().size.width, NSTableColumn::width);
                let height = table.rowHeight();
                let view = NSView::initWithFrame(NSView::alloc(self.mtm()),
                    NSRect::new(NSPoint::ZERO, NSSize::new(width, height)));
                let access = launcher::project_access_label(project);
                let label_height = if access.is_some() { 19.0 } else { 0.0 };
                let title = NSString::from_str(&project.name);
                let button = unsafe {
                    NSButton::buttonWithTitle_target_action(
                        &title, Some(self), Some(sel!(openListedProject:)), self.mtm(),
                    )
                };
                let symbol = if let Some(remote) = &project.remote {
                    button.setToolTip(Some(&NSString::from_str(&remote.server)));
                    ("folder-remote-symbolic", "Remote project")
                } else {
                    ("folder-symbolic", "Local project")
                };
                let icon_size = 16.0;
                let icon = NSImageView::initWithFrame(NSImageView::alloc(self.mtm()), NSRect::new(
                    NSPoint::new(CONTENT_MARGIN, (height - icon_size) / 2.0),
                    NSSize::new(icon_size, icon_size),
                ));
                icon.setImage(Some(&bundled_symbol(symbol.0, symbol.1)));
                icon.setContentTintColor(Some(&NSColor::secondaryLabelColor()));
                view.addSubview(&icon);
                let text_x = CONTENT_MARGIN + icon_size + 8.0;
                let text_width = (width - MENU_SIZE - CONTENT_MARGIN - text_x).max(0.0);
                let name = NSTextField::labelWithString(&title, self.mtm());
                name.sizeToFit();
                name.setFrame(NSRect::new(
                    NSPoint::new(text_x, (height + label_height - name.frame().size.height) / 2.0),
                    NSSize::new(text_width, name.frame().size.height),
                ));
                name.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
                name.setAccessibilityElement(false);
                view.addSubview(&name);
                if let Some(access) = access {
                    let label = NSTextField::labelWithString(&NSString::from_str(&access), self.mtm());
                    label.setFont(Some(&NSFont::systemFontOfSize(NSFont::smallSystemFontSize())));
                    label.setTextColor(Some(&NSColor::secondaryLabelColor()));
                    label.setFrame(NSRect::new(NSPoint::new(text_x, 3.0),
                        NSSize::new(text_width, label_height)));
                    label.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
                    view.addSubview(&label);
                }
                button.setTransparent(true);
                button.setTag(row);
                button.setFrame(NSRect::new(NSPoint::ZERO,
                    NSSize::new((width - MENU_SIZE - CONTENT_MARGIN).max(0.0), height)));
                button.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
                view.addSubview(&button);
                let mut items = vec![("Project Information", sel!(showProjectInfo:))];
                if project.path.is_some() {
                    items.extend([
                        ("Show in Finder", sel!(revealProject:)),
                        ("Restart Local Service", sel!(restartProjectService:)),
                    ]);
                }
                items.push(("Delete Project…", sel!(deleteProject:)));
                let options = self.popup(
                    NSPoint::new(width - MENU_SIZE - CONTENT_MARGIN, (height - MENU_SIZE) / 2.0),
                    ("view-more-symbolic", "Project options"),
                    &items, Some(&project.key()),
                );
                options.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinXMargin);
                view.addSubview(&options);
                Some(view)
            })
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
                self.open_remote("Could not open project", move || launcher::launch_listed_project(&project));
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
                    let path = launcher::create_project(
                        ProjectLocation::Remote(server), &request.name, (request.canvas_size, request.fps), &[],
                    )?;
                    launcher::launch_editor(&path)
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
                ProjectLocation::Local(path), &request.name, (request.canvas_size, request.fps), &[],
            ) {
                Ok(path) => path,
                Err(error) => {
                    self.show_error("Could not create project", &error);
                    return;
                }
            };
            self.open_in_editor(&path);
        }

        #[unsafe(method(showPreferences:))]
        fn show_preferences(&self, _sender: &NSObject) {
            self.settings();
        }

        #[unsafe(method(showAbout:))]
        fn show_about(&self, _sender: &NSObject) {
            shrimply_editor_appkit::show_about(self.mtm());
        }

        #[unsafe(method(showProjectInfo:))]
        fn show_project_info(&self, sender: &NSMenuItem) {
            if let Some(project) = self.menu_project(sender) {
                self.show_error(&project.name, &launcher::project_details(&project));
            }
        }

        #[unsafe(method(revealProject:))]
        fn reveal_project(&self, sender: &NSMenuItem) {
            if let Some(url) = self.menu_project(sender).and_then(|project| project.path)
                .and_then(NSURL::from_file_path) {
                NSWorkspace::sharedWorkspace().activateFileViewerSelectingURLs(&NSArray::from_retained_slice(&[url]));
            }
        }

        #[unsafe(method(restartProjectService:))]
        fn restart_project_service(&self, sender: &NSMenuItem) {
            if let Some(path) = self.menu_project(sender).and_then(|project| project.path) {
                self.project_task(move || launcher::restart_local_service(&path), |delegate, result| {
                    if let Err(error) = result {
                        delegate.show_error("Could not restart the project service", &error);
                    }
                });
            }
        }

        #[unsafe(method(deleteProject:))]
        fn delete_project(&self, sender: &NSMenuItem) {
            let Some(project) = self.menu_project(sender) else { return; };
            let alert = NSAlert::new(self.mtm());
            alert.setMessageText(&NSString::from_str(&project.name));
            alert.setInformativeText(if project.remote.is_some() {
                ns_string!("Delete this project from the server for everyone?")
            } else {
                ns_string!("Permanently delete this project file?")
            });
            alert.addButtonWithTitle(ns_string!("Cancel"));
            alert.addButtonWithTitle(ns_string!("Delete Project")).setHasDestructiveAction(true);
            if alert.runModal() == NSAlertSecondButtonReturn {
                self.project_task(move || launcher::delete_project(&project), |delegate, result| {
                    match result {
                        Ok(()) => delegate.reload_projects(),
                        Err(error) => delegate.show_error("Could not delete project", &error),
                    }
                });
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
        self.start_editor(launcher::launch_editor(path));
    }

    fn start_editor(&self, result: Result<std::process::Child, String>) {
        let mut child = match result {
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

    fn settings(&self) {
        let owner = Weak::new(self);
        self.ivars()
            .preferences
            .get_or_init(|| shrimply_editor_appkit::PreferencesWindow::new(self.mtm(), None))
            .present_from_launcher(move || {
                if let Some(delegate) = owner.load() {
                    delegate.reload_projects();
                }
            });
    }

    fn open_remote(
        &self,
        heading: &'static str,
        operation: impl FnOnce() -> Result<std::process::Child, String> + Send + 'static,
    ) {
        self.project_task(operation, move |delegate, result| match result {
            Ok(child) => delegate.start_editor(Ok(child)),
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
                Ok(true) => delegate.reload_lists(),
                Ok(false) => (),
                Err(error) => delegate.show_error("Could not load projects", &error),
            }
        });
    }

    fn reload_lists(&self) {
        let sources = self.ivars().sources.get().expect("source list");
        let selected = {
            let model = self.ivars().model.borrow();
            model
                .sources()
                .position(|(url, _)| url == model.source())
                .unwrap_or(0)
        };
        sources.reloadData();
        sources
            .selectRowIndexes_byExtendingSelection(&NSIndexSet::indexSetWithIndex(selected), false);
        self.ivars()
            .projects
            .get()
            .expect("project list")
            .reloadData();
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

    fn popup(
        &self,
        origin: NSPoint,
        (symbol, tooltip): (&str, &str),
        items: &[(&str, Sel)],
        key: Option<&str>,
    ) -> Retained<NSPopUpButton> {
        let button = shrimply_components_appkit::menu::popup(
            NSRect::new(origin, NSSize::new(MENU_SIZE, MENU_SIZE)),
            true,
            self.mtm(),
        );
        button.setBordered(false);
        button.setPreferredEdge(NSRectEdge::MinY);
        button
            .cell()
            .and_then(|cell| cell.downcast::<NSPopUpButtonCell>().ok())
            .expect("pull-down button cell")
            .setArrowPosition(NSPopUpArrowPosition::NoArrow);
        let icon = bundled_symbol(symbol, tooltip);
        let tooltip = NSString::from_str(tooltip);
        button.setToolTip(Some(&tooltip));
        button.setAccessibilityLabel(Some(&tooltip));
        let menu = button.menu().expect("pop-up button menu");
        button.addItemWithTitle(ns_string!(""));
        let heading = button.itemAtIndex(0).expect("pop-up button label");
        heading.setImage(Some(&icon));
        let key = key.map(NSString::from_str);
        for (label, action) in items {
            unsafe {
                let item = menu.addItemWithTitle_action_keyEquivalent(
                    &NSString::from_str(label),
                    Some(*action),
                    ns_string!(""),
                );
                item.setTarget(Some(self));
                item.setRepresentedObject(key.as_deref().map(|key| key.as_ref()));
            }
        }
        button
    }

    fn menu_project(&self, sender: &NSMenuItem) -> Option<launcher::ProjectEntry> {
        let key = sender.representedObject()?.downcast::<NSString>().ok()?;
        self.ivars()
            .model
            .borrow()
            .project_by_key(&key.to_string())
            .cloned()
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
            sources: OnceCell::new(),
            model: RefCell::new(Launcher::default()),
            preferences: OnceCell::new(),
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
