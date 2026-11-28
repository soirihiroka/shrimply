use super::error_alert;
use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAccessibility, NSAlert, NSAlertFirstButtonReturn, NSAutoresizingMaskOptions,
    NSBackingStoreType, NSBezelStyle, NSBox, NSBoxType, NSButton, NSColor, NSControl,
    NSControlSize, NSControlTextEditingDelegate, NSFontManager, NSImageView, NSLineBreakMode,
    NSModalResponseOK, NSOpenPanel, NSPopUpButton, NSScrollView, NSSecureTextField,
    NSSegmentSwitchTracking, NSSegmentedControl, NSSplitViewController, NSSplitViewDividerStyle,
    NSSplitViewItem, NSSplitViewItemCollapseBehavior, NSStackView, NSStepper, NSTableCellView,
    NSTableColumn, NSTableColumnResizingOptions, NSTableView, NSTableViewColumnAutoresizingStyle,
    NSTableViewDataSource, NSTableViewDelegate, NSTableViewRowSizeStyle, NSTableViewStyle,
    NSTextAlignment, NSTextField, NSTextFieldBezelStyle, NSToolbar, NSToolbarDelegate,
    NSToolbarSidebarTrackingSeparatorItemIdentifier, NSView, NSViewController, NSWindow,
    NSWindowDelegate, NSWindowStyleMask, NSWindowToolbarStyle, NSWorkspace,
};
use objc2_foundation::{
    MainThreadMarker, NSArray, NSEdgeInsets, NSIndexSet, NSNotification, NSNumber,
    NSNumberFormatter, NSNumberFormatterStyle, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize,
    NSString, NSTimer, NSURL, ns_string,
};
use objc2_quartz_core::{CAGradientLayer, kCACornerCurveContinuous};
use serde_json::{Value, json};
use shrimply_components_appkit::{
    ActionButton, ColorPicker, StringChoice, StringSelector, Switch, bundled_symbol, column_append,
    column_stack, inset, row_stack,
};
use shrimply_editor_state::preferences::{
    self, PreferenceId, PreferencePage, PreferenceValue, SharedPreferences,
};
use shrimply_i18n::text;
use shrimply_inspector_document::preferences::Preferences;
use shrimply_math_core::fraction_new;
use std::cell::{Cell, OnceCell, RefCell};
use std::ffi::c_void;

const SIDEBAR_INSET: f64 = 6.0;
const ROW_INSET: NSEdgeInsets = NSEdgeInsets {
    top: 5.0,
    bottom: 5.0,
    left: 12.0,
    right: 12.0,
};

type FormParts = NSArray<AnyObject>;

unsafe extern "C" {
    fn shrimply_settings_form_create() -> *mut c_void;
    fn shrimply_settings_form_update(
        host: *const c_void,
        page: *const c_void,
        sections: *const c_void,
    );
}

struct Layout {
    window: Retained<NSWindow>,
    sidebar: Retained<NSSplitViewItem>,
    navigation: Retained<NSTableView>,
    header: Retained<NSView>,
    compact: Retained<NSPopUpButton>,
    form: Retained<NSView>,
}

struct ServerList {
    table: Retained<NSTableView>,
    segments: Retained<NSSegmentedControl>,
    controls: Vec<Value>,
    urls: Vec<String>,
    project: bool,
}

impl ServerList {
    fn selected(&self) -> Option<&str> {
        self.urls
            .get(usize::try_from(self.table.selectedRow()).ok()?)
            .map(String::as_str)
    }

    fn action(&self, segment: isize) -> Option<&Value> {
        self.controls
            .iter()
            .find(|control| {
                let command = settings_command(control);
                match (self.project, segment, command["action"].as_str()) {
                    (true, 0, Some("project_server_sign_in")) => command["url"].is_null(),
                    (false, 0, Some("server_add")) => true,
                    (true, 1, Some("project_server_remove"))
                    | (true, 2, Some("project_server_sign_in"))
                    | (false, 1, Some("server_remove"))
                    | (false, 2, Some("server_edit")) => {
                        self.selected().is_some_and(|url| command["url"] == url)
                    }
                    _ => false,
                }
            })
            .filter(|control| control["enabled"] != false)
    }

    fn update_actions(&self) -> Option<Value> {
        for segment in 0..3 {
            self.segments
                .setEnabled_forSegment(self.action(segment).is_some(), segment);
        }
        let selector = self
            .controls
            .iter()
            .find(|control| settings_action(control) == "server_select")?;
        let selected = self.selected()?;
        if selector["display"] == selected {
            return None;
        }
        items(&selector["choices"])
            .iter()
            .map(|choice| &choice["command"])
            .find(|command| command["url"] == selected)
            .cloned()
    }
}

pub struct PreferencesIvars {
    store: SharedPreferences,
    settings: RefCell<Preferences>,
    layout: OnceCell<Layout>,
    selected: Cell<usize>,
    displayed: RefCell<Value>,
    server_lists: RefCell<Vec<ServerList>>,
    timer: RefCell<Option<Retained<NSTimer>>>,
    on_close: RefCell<Option<Box<dyn Fn()>>>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = PreferencesIvars]
    pub struct PreferencesWindow;

    unsafe impl NSObjectProtocol for PreferencesWindow {}

    unsafe impl NSWindowDelegate for PreferencesWindow {
        #[unsafe(method(windowDidResize:))]
        fn window_did_resize(&self, _event: &NSNotification) {
            self.resize_navigation();
        }

        #[unsafe(method(windowWillClose:))]
        fn window_will_close(&self, _event: &NSNotification) {
            if !self.ivars().settings.borrow().has_pending_changes() {
                self.finish_closing();
            }
        }
    }

    unsafe impl NSToolbarDelegate for PreferencesWindow {
        #[unsafe(method_id(toolbarDefaultItemIdentifiers:))]
        fn toolbar_items(&self, _toolbar: &NSToolbar) -> Retained<NSArray<NSString>> {
            NSArray::from_slice(&[unsafe { NSToolbarSidebarTrackingSeparatorItemIdentifier }])
        }
    }

    unsafe impl NSControlTextEditingDelegate for PreferencesWindow {}

    unsafe impl NSTableViewDataSource for PreferencesWindow {
        #[unsafe(method(numberOfRowsInTableView:))]
        fn number_of_rows(&self, table: &NSTableView) -> isize {
            if self.is_sidebar(table) {
                return PreferencePage::ALL.len() as isize;
            }
            self.ivars().server_lists.borrow().iter()
                .find(|list| &*list.table == table)
                .map_or(0, |list| list.urls.len() as isize)
        }
    }

    unsafe impl NSTableViewDelegate for PreferencesWindow {
        #[unsafe(method_id(tableView:viewForTableColumn:row:))]
        fn table_row(&self, table: &NSTableView, _column: Option<&NSTableColumn>, row: isize) -> Option<Retained<NSView>> {
            usize::try_from(row).ok().and_then(|index| {
                if self.is_sidebar(table) {
                    return PreferencePage::ALL.get(index).map(|page| sidebar_row(*page, self.mtm()));
                }
                let lists = self.ivars().server_lists.borrow();
                let server = lists.iter().find(|list| &*list.table == table)?.urls.get(index)?;
                let label = settings_label(server, self.mtm());
                Some(settings_table_cell(&label, &label, self.mtm()))
            })
        }

        #[unsafe(method(tableViewSelectionDidChange:))]
        fn table_selection_changed(&self, notification: &NSNotification) {
            let Some(table) = notification.object().and_then(|object| object.downcast::<NSTableView>().ok()) else {
                return;
            };
            if self.is_sidebar(&table) {
                if let Ok(index) = usize::try_from(table.selectedRow()) {
                    self.select(index);
                }
                return;
            }
            let select = self.ivars().server_lists.borrow().iter()
                .find(|list| list.table == table).and_then(ServerList::update_actions);
            if let Some(command) = select {
                self.apply(&command);
            }
        }
    }

    impl PreferencesWindow {
        #[unsafe(method(pollSettings:))]
        fn poll_settings(&self, _timer: &NSTimer) {
            self.refresh();
            if !self.ivars().layout.get().expect("settings layout").window.isVisible()
                && !self.ivars().settings.borrow().has_pending_changes()
            {
                self.finish_closing();
            }
        }

        #[unsafe(method(selectSettingsPage:))]
        fn select_settings_page(&self, sender: &NSPopUpButton) {
            if let Ok(index) = usize::try_from(sender.indexOfSelectedItem()) {
                self.select(index);
            }
        }

        #[unsafe(method(chooseSettingsFont:))]
        fn choose_settings_font(&self, sender: &NSButton) {
            self.choose_font(sender);
        }

        #[unsafe(method(setSettingsNumber:))]
        fn set_settings_number(&self, sender: &NSControl) {
            let id = PreferenceId::ALL[sender.tag() as usize];
            let range = preferences::integer_range(id).expect("numeric preference");
            let scaled = sender.doubleValue() * range.scale as f64;
            if !scaled.is_finite() {
                return;
            }
            let value = scaled.round() as i64;
            let value = value.clamp(range.minimum, range.maximum);
            if preferences::value(&self.ivars().store, id) != PreferenceValue::Integer(value) {
                self.apply(&json!({
                    "action": "set",
                    "key": id.key(),
                    "values": [fraction_new(value, range.scale).to_string()],
                }));
            }
        }

        #[unsafe(method(changeSettingsServerList:))]
        fn change_settings_server_list(&self, sender: &NSSegmentedControl) {
            let action = self.ivars().server_lists.borrow().iter()
                .find(|list| &*list.segments == sender)
                .and_then(|list| list.action(sender.selectedSegment())).cloned();
            if let Some(action) = action {
                if action["widget"] == "editor" {
                    self.edit(&action["editor"]);
                } else {
                    self.apply(&action["command"]);
                }
            }
        }

    }
);

impl PreferencesWindow {
    pub fn new(mtm: MainThreadMarker, store: Option<SharedPreferences>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(PreferencesIvars {
            store: store.unwrap_or_else(preferences::open_with_defaults),
            settings: RefCell::new(Preferences::default()),
            layout: OnceCell::new(),
            selected: Cell::new(0),
            displayed: RefCell::new(Value::Null),
            server_lists: RefCell::new(Vec::new()),
            timer: RefCell::new(None),
            on_close: RefCell::new(None),
        });
        unsafe { msg_send![super(this), init] }
    }

    pub fn present_from_launcher(&self, on_close: impl Fn() + 'static) {
        self.ivars().on_close.replace(Some(Box::new(on_close)));
        self.present();
    }

    pub fn present(&self) {
        let layout = self.ivars().layout.get_or_init(|| self.build());
        if layout.window.toolbar().is_none() {
            let toolbar =
                NSToolbar::initWithIdentifier(NSToolbar::alloc(self.mtm()), ns_string!("Settings"));
            toolbar.setDelegate(Some(ProtocolObject::from_ref(self)));
            layout.window.setToolbar(Some(&toolbar));
            layout.window.setToolbarStyle(NSWindowToolbarStyle::Unified);
        }
        self.resize_navigation();
        self.refresh();
        layout.window.makeKeyAndOrderFront(None);
        let mut timer = self.ivars().timer.borrow_mut();
        if timer.is_none() {
            *timer = Some(unsafe {
                NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
                    0.25,
                    self,
                    sel!(pollSettings:),
                    None,
                    true,
                )
            });
        }
    }

    fn finish_closing(&self) {
        if let Some(timer) = self.ivars().timer.borrow_mut().take() {
            timer.invalidate();
        }
        if let Some(on_close) = self.ivars().on_close.borrow_mut().take() {
            on_close();
        }
    }

    fn build(&self) -> Layout {
        let mtm = self.mtm();
        let size = NSSize::new(810.0, 600.0);
        let rect = NSRect::new(NSPoint::ZERO, size);
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                rect,
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
        window.setTitle(&NSString::from_str(&text(preferences::SETTINGS_TITLE)));
        window.setContentMinSize(NSSize::new(460.0, 390.0));
        window.setDelegate(Some(ProtocolObject::from_ref(self)));

        let sidebar_width = f64::from(preferences::SETTINGS_SIDEBAR_WIDTH);
        let sidebar_rect = NSRect::new(
            NSPoint::ZERO,
            NSSize::new(sidebar_width - 2.0 * SIDEBAR_INSET, size.height),
        );
        let navigation = settings_table(sidebar_rect, NSTableViewStyle::SourceList, 30.0, mtm);
        navigation.setAllowsEmptySelection(false);
        unsafe {
            navigation.setDataSource(Some(ProtocolObject::from_ref(self)));
            navigation.setDelegate(Some(ProtocolObject::from_ref(self)));
        }
        let sidebar_scroll = NSScrollView::initWithFrame(NSScrollView::alloc(mtm), sidebar_rect);
        sidebar_scroll.setDrawsBackground(false);
        sidebar_scroll.setDocumentView(Some(&navigation));
        let sidebar_controller = NSViewController::new(mtm);
        sidebar_controller.setView(&inset(
            &sidebar_scroll,
            NSEdgeInsets {
                top: 0.0,
                bottom: 0.0,
                left: SIDEBAR_INSET,
                right: SIDEBAR_INSET,
            },
            mtm,
        ));
        let sidebar = NSSplitViewItem::sidebarWithViewController(&sidebar_controller);
        sidebar.setMinimumThickness(sidebar_width);
        sidebar.setMaximumThickness(sidebar_width);
        sidebar.setCanCollapseFromWindowResize(false);
        sidebar.setCollapseBehavior(
            NSSplitViewItemCollapseBehavior::PreferResizingSiblingsWithFixedSplitView,
        );

        let compact = shrimply_components_appkit::menu::popup(NSRect::ZERO, false, mtm);
        for (index, page) in PreferencePage::ALL.into_iter().enumerate() {
            let name = text(page.title());
            compact.addItemWithTitle(&NSString::from_str(&name));
            compact
                .itemAtIndex(index as isize)
                .expect("category menu item")
                .setImage(Some(&bundled_symbol(page.icon(), &name)));
        }
        unsafe {
            compact.setTarget(Some(self));
            compact.setAction(Some(sel!(selectSettingsPage:)));
        }
        let compact_menu = column_stack(0.0, mtm);
        compact_menu.addArrangedSubview(&compact);
        let header = inset(
            &compact_menu,
            NSEdgeInsets {
                top: 12.0,
                bottom: 0.0,
                left: 20.0,
                right: 20.0,
            },
            mtm,
        );
        header.setHidden(true);
        // Swift passes ownership of a retained NSHostingView to this AppKit window.
        let form = unsafe { Retained::from_raw(shrimply_settings_form_create().cast::<NSView>()) }
            .expect("create native Settings form");
        let detail = column_stack(0.0, mtm);
        column_append(&detail, &header);
        column_append(&detail, &form);
        let split = NSSplitViewController::new(mtm);
        split.splitView().setVertical(true);
        split
            .splitView()
            .setDividerStyle(NSSplitViewDividerStyle::Thin);
        split.addSplitViewItem(&sidebar);
        split.addSplitViewItem(&super::layout::split_item(&detail, mtm));
        split.view().setFrame(rect);
        window.setContentViewController(Some(&split));
        window.center();
        Layout {
            window,
            sidebar,
            navigation,
            header,
            compact,
            form,
        }
    }

    fn resize_navigation(&self) {
        if let Some(layout) = self.ivars().layout.get() {
            let wide =
                layout.window.frame().size.width >= f64::from(preferences::SETTINGS_WIDE_WIDTH);
            if layout.sidebar.isCollapsed() == wide {
                layout.sidebar.setCollapsed(!wide);
                layout.header.setHidden(wide);
            }
        }
    }

    fn is_sidebar(&self, table: &NSTableView) -> bool {
        // The only table with a data source before building Layout is the sidebar.
        self.ivars()
            .layout
            .get()
            .is_none_or(|layout| &*layout.navigation == table)
    }

    fn select(&self, index: usize) {
        if self.ivars().selected.replace(index) != index {
            self.refresh();
        }
    }

    fn refresh(&self) {
        let document = self
            .ivars()
            .settings
            .borrow_mut()
            .snapshot(&self.ivars().store);
        self.render(&document);
    }

    fn render(&self, document: &Value) {
        let layout = self.ivars().layout.get().expect("settings layout");
        let index = self.ivars().selected.get();
        let id = format!("preferences:{}", PreferencePage::ALL[index].id());
        let page = items(&document["controls"])
            .iter()
            .find(|page| page["id"] == id)
            .expect("shared settings page");
        if *self.ivars().displayed.borrow() == *page {
            return;
        }
        if layout.navigation.selectedRow() != index as isize {
            layout.navigation.selectRowIndexes_byExtendingSelection(
                &NSIndexSet::indexSetWithIndex(index),
                false,
            );
        }
        layout.compact.selectItemAtIndex(index as isize);
        self.ivars().displayed.replace(page.clone());
        let previous_lists = self.ivars().server_lists.take();
        let selected_project_server = previous_lists
            .iter()
            .find(|list| list.project)
            .and_then(ServerList::selected);
        let focused = layout.window.firstResponder().and_then(|view| {
            let table = view.downcast::<NSTableView>().ok()?;
            previous_lists
                .iter()
                .find(|list| list.table == table)
                .map(|list| list.project)
        });
        let sections: Vec<Retained<FormParts>> = items(&page["controls"])
            .iter()
            .map(|section| {
                let title = NSString::from_str(if section["label"] == page["label"] {
                    ""
                } else {
                    string(&section["label"])
                });
                let controls = items(&section["controls"]);
                let list = self.server_list(controls, selected_project_server);
                let has_server_list = list.is_some();
                let rows: Vec<_> = list
                    .into_iter()
                    .map(|view| NSArray::from_slice(&[(&*view).as_ref()]))
                    .chain(
                        controls
                            .iter()
                            .filter(|control| !has_server_list || !is_server_list_control(control))
                            .map(|control| self.control(control)),
                    )
                    .collect();
                let contents: Vec<&AnyObject> = std::iter::once(title.as_ref())
                    .chain(rows.iter().map(|row| (&**row).as_ref()))
                    .collect();
                NSArray::from_slice(&contents)
            })
            .collect();
        let sections = NSArray::from_retained_slice(&sections);
        let page = NSString::from_str(&id);
        // Both sides run on the main thread; Swift retains the snapshot's views and strings.
        unsafe {
            shrimply_settings_form_update(
                Retained::as_ptr(&layout.form).cast(),
                Retained::as_ptr(&page).cast(),
                Retained::as_ptr(&sections).cast(),
            )
        }
        layout.form.layoutSubtreeIfNeeded();
        for list in self.ivars().server_lists.borrow().iter() {
            let row = list.table.selectedRow();
            if row >= 0 {
                list.table.scrollRowToVisible(row);
            }
            if focused == Some(list.project) {
                layout.window.makeFirstResponder(Some(&list.table));
            }
        }
    }

    fn server_list(
        &self,
        controls: &[Value],
        previous_project_server: Option<&str>,
    ) -> Option<Retained<NSView>> {
        let project = controls
            .iter()
            .any(|control| settings_action(control) == "project_server_sign_in");
        let (urls, selected) = if project {
            let urls = controls
                .iter()
                .filter(|control| settings_action(control) == "project_server_sign_in")
                .filter_map(|control| settings_command(control)["url"].as_str());
            (urls.collect::<Vec<_>>(), previous_project_server)
        } else {
            let selector = controls
                .iter()
                .find(|control| settings_action(control) == "server_select")?;
            let urls = items(&selector["choices"])
                .iter()
                .filter_map(|choice| choice["command"]["url"].as_str());
            (urls.collect(), selector["display"].as_str())
        };
        let mtm = self.mtm();
        let table = settings_table(NSRect::ZERO, NSTableViewStyle::Inset, 26.0, mtm);
        let scroll = NSScrollView::initWithFrame(NSScrollView::alloc(mtm), NSRect::ZERO);
        scroll.setDrawsBackground(false);
        scroll.setHasVerticalScroller(urls.len() > 4);
        scroll.setAutohidesScrollers(true);
        scroll.setDocumentView(Some(&table));
        let visible_rows = urls.len().min(4);
        let height = table.rowHeight() + table.intercellSpacing().height;
        scroll
            .heightAnchor()
            .constraintEqualToConstant(height * visible_rows as f64 + 8.0)
            .setActive(true);

        let actions = [
            ("list-add-symbolic", "Add Server"),
            ("list-remove-symbolic", "Remove Server"),
            (
                "document-edit-symbolic",
                if project { "Sign In" } else { "Edit Server" },
            ),
        ];
        let images = actions.map(|(icon, label)| bundled_symbol(icon, &text(label)));
        let segments = unsafe {
            NSSegmentedControl::segmentedControlWithImages_trackingMode_target_action(
                &NSArray::from_retained_slice(&images),
                NSSegmentSwitchTracking::Momentary,
                Some(self),
                Some(sel!(changeSettingsServerList:)),
                mtm,
            )
        };
        segments.setControlSize(NSControlSize::Small);
        for (index, (_, label)) in actions.into_iter().enumerate() {
            segments.setToolTip_forSegment(Some(&NSString::from_str(&text(label))), index as isize);
        }
        let selected = urls.iter().position(|url| Some(*url) == selected);
        self.ivars().server_lists.borrow_mut().push(ServerList {
            table: table.clone(),
            segments: segments.clone(),
            controls: controls.to_vec(),
            urls: urls.into_iter().map(str::to_owned).collect(),
            project,
        });
        unsafe {
            table.setDataSource(Some(ProtocolObject::from_ref(self)));
            table.setDelegate(Some(ProtocolObject::from_ref(self)));
        }
        table.reloadData();
        if let Some(index) = selected {
            table.selectRowIndexes_byExtendingSelection(
                &NSIndexSet::indexSetWithIndex(index),
                false,
            );
        }
        let _ = self
            .ivars()
            .server_lists
            .borrow()
            .last()
            .expect("server list installed")
            .update_actions();

        let group = column_stack(0.0, mtm);
        let bar = row_stack(8.0, mtm);
        bar.addArrangedSubview(&segments);
        bar.addArrangedSubview(&NSView::new(mtm));
        if let Some(check) = controls
            .iter()
            .find(|control| settings_action(control) == "server_check")
        {
            let button = self.command_button(check);
            button.view().setControlSize(NSControlSize::Small);
            bar.addArrangedSubview(button.view());
        }
        if visible_rows != 0 {
            column_append(&group, &scroll);
            column_append(&group, &settings_separator(mtm));
        }
        column_append(&group, &inset(&bar, ROW_INSET, mtm));
        Some(group.into_super())
    }

    fn control(&self, control: &Value) -> Retained<FormParts> {
        let mtm = self.mtm();
        let title = string(&control["label"]);
        let display = string(&control["display"]);
        let subtitle = string(&control["subtitle"]);
        let weak = Weak::new(self);
        let send = move |command: Value| {
            if let Some(settings) = weak.load() {
                settings.apply(&command);
            }
        };
        let enabled = control["enabled"].as_bool().unwrap_or(true);
        match string(&control["widget"]) {
            "toggle" => {
                let command = control["command"].clone();
                let toggle = Switch::new(
                    control["selected"] == true,
                    (!subtitle.is_empty()).then_some(subtitle),
                    move |enabled| {
                        let mut command = command.clone();
                        command["value"] = json!(enabled);
                        send(command);
                    },
                    mtm,
                );
                toggle.view().setEnabled(enabled);
                toggle
                    .view()
                    .setAccessibilityLabel(Some(&NSString::from_str(title)));
                settings_row(title, subtitle, Some(toggle.view().as_ref()))
            }
            "choice" if !items(&control["choices"]).is_empty() => {
                let entries = items(&control["choices"]).to_vec();
                let selected = entries
                    .iter()
                    .position(|choice| choice["label"] == display)
                    .map(|index| index.to_string());
                let choices = selected
                    .is_none()
                    .then(|| StringChoice {
                        label: if display.is_empty() {
                            text("Choose…").into_owned()
                        } else {
                            display.into()
                        },
                        value: String::new(),
                    })
                    .into_iter()
                    .chain(
                        entries
                            .iter()
                            .enumerate()
                            .map(|(index, choice)| StringChoice {
                                label: string(&choice["label"]).to_owned(),
                                value: index.to_string(),
                            }),
                    )
                    .collect();
                let picker = StringSelector::new(
                    selected.as_deref().unwrap_or_default(),
                    choices,
                    move |index| {
                        if let Ok(index) = index.parse::<usize>() {
                            send(entries[index]["command"].clone());
                        }
                    },
                    mtm,
                );
                if let Some(control) = picker.view().downcast_ref::<NSControl>() {
                    control.setEnabled(enabled);
                    control.setAccessibilityLabel(Some(&NSString::from_str(title)));
                }
                if let Some(popup) = picker.view().downcast_ref::<NSPopUpButton>() {
                    popup.setBordered(false);
                }
                settings_row(
                    title,
                    control["choice_description"].as_str().unwrap_or(subtitle),
                    Some(picker.view().as_ref()),
                )
            }
            "editor" => self.editor_row(control, enabled),
            "font" => {
                let caption = if display.is_empty() {
                    text("Choose…")
                } else {
                    display.into()
                };
                let button = unsafe {
                    NSButton::buttonWithTitle_target_action(
                        &NSString::from_str(&caption),
                        Some(self),
                        Some(sel!(chooseSettingsFont:)),
                        mtm,
                    )
                };
                button.setBezelStyle(NSBezelStyle::Push);
                button.setEnabled(enabled);
                button.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
                settings_row(title, subtitle, Some((&*button).as_ref()))
            }
            "button" => {
                let button = self.command_button(control);
                settings_row(subtitle, "", Some(button.view().as_ref()))
            }
            _ if display.is_empty() => settings_row(title, subtitle, None),
            _ => settings_row(
                title,
                subtitle,
                Some((&*NSString::from_str(display)).as_ref()),
            ),
        }
    }

    fn command_button(&self, control: &Value) -> ActionButton {
        let command = control["command"].clone();
        let weak = Weak::new(self);
        let button = ActionButton::new(
            string(&control["label"]),
            move || {
                if let Some(settings) = weak.load() {
                    settings.apply(&command);
                }
            },
            self.mtm(),
        );
        button
            .view()
            .setEnabled(control["enabled"].as_bool().unwrap_or(true));
        button
    }

    fn editor_row(&self, control: &Value, enabled: bool) -> Retained<FormParts> {
        let mtm = self.mtm();
        let title = string(&control["label"]);
        let subtitle = string(&control["subtitle"]);
        let display = string(&control["display"]);
        let editor = &control["editor"];
        let id = editor["command"]["key"]
            .as_str()
            .and_then(PreferenceId::from_key);
        let weak = Weak::new(self);
        if let Some(id) = id {
            let value = preferences::value(&self.ivars().store, id);
            if let PreferenceValue::Integer(value) = value {
                let picker = self.number_picker(id, value, enabled);
                return settings_row(title, subtitle, Some((&*picker).as_ref()));
            }
            if let PreferenceValue::Color(color) = value {
                let command = editor["command"].clone();
                let picker = ColorPicker::new(
                    color,
                    move |color| {
                        if let Some(settings) = weak.load() {
                            let mut command = command.clone();
                            command["values"] =
                                json!(color.to_array().map(|channel| channel.to_string()));
                            settings.apply(&command);
                        }
                    },
                    mtm,
                );
                picker.view().setEnabled(enabled);
                return settings_row(title, subtitle, Some(picker.view().as_ref()));
            }
        }
        let button_title = if id.is_none() {
            title.into()
        } else {
            text(if display.is_empty() {
                "Choose…"
            } else {
                "Edit…"
            })
        };
        let editor = editor.clone();
        let button = ActionButton::new(
            &button_title,
            move || {
                if let Some(settings) = weak.load() {
                    settings.edit(&editor);
                }
            },
            mtm,
        );
        button.view().setEnabled(enabled);
        let (label, subtitle) = if id.is_some() {
            (
                title,
                if display.is_empty() {
                    subtitle
                } else {
                    display
                },
            )
        } else {
            (display, "")
        };
        settings_row(label, subtitle, Some(button.view().as_ref()))
    }

    fn number_picker(&self, id: PreferenceId, value: i64, enabled: bool) -> Retained<NSStackView> {
        let mtm = self.mtm();
        let range = preferences::integer_range(id).expect("numeric preference");
        let scaled = |ticks: i64| ticks as f64 / range.scale as f64;
        let minimum = scaled(range.minimum);
        let maximum = scaled(range.maximum);
        let formatter = NSNumberFormatter::new();
        formatter.setNumberStyle(NSNumberFormatterStyle::DecimalStyle);
        formatter.setUsesGroupingSeparator(false);
        formatter.setMaximumFractionDigits(match range.scale {
            1 => 0,
            10 => 1,
            _ => 2,
        });
        formatter.setMinimum(Some(&NSNumber::new_f64(minimum)));
        formatter.setMaximum(Some(&NSNumber::new_f64(maximum)));

        let field = NSTextField::textFieldWithString(ns_string!(""), mtm);
        field.setAlignment(NSTextAlignment::Right);
        field.setBezelStyle(NSTextFieldBezelStyle::RoundedBezel);
        field.setFormatter(Some(&formatter));
        field
            .widthAnchor()
            .constraintEqualToConstant(55.0)
            .setActive(true);
        field
            .cell()
            .expect("numeric text field cell")
            .setSendsActionOnEndEditing(true);
        let stepper = NSStepper::new(mtm);
        stepper.setMinValue(minimum);
        stepper.setMaxValue(maximum);
        stepper.setIncrement(scaled(range.step));
        stepper.setValueWraps(false);

        let tag = PreferenceId::ALL
            .iter()
            .position(|candidate| *candidate == id)
            .expect("numeric preference ID");
        let label = NSString::from_str(&text(id.presentation().label));
        let row = row_stack(4.0, mtm);
        let controls: [&NSControl; 2] = [&field, &stepper];
        for control in controls {
            control.setTag(tag as isize);
            control.setControlSize(NSControlSize::Small);
            control.setDoubleValue(scaled(value));
            control.setEnabled(enabled);
            control.setAccessibilityLabel(Some(&label));
            unsafe {
                control.setTarget(Some(self));
                control.setAction(Some(sel!(setSettingsNumber:)));
            }
            row.addArrangedSubview(control);
        }
        row
    }

    fn edit(&self, editor: &Value) {
        let mtm = self.mtm();
        let alert = NSAlert::new(mtm);
        alert.setMessageText(&NSString::from_str(string(&editor["title"])));
        let fields = column_stack(8.0, mtm);
        let entries: Vec<Retained<NSTextField>> = items(&editor["fields"])
            .iter()
            .map(|field| {
                let input = if field["keyboard"] == "password" {
                    NSSecureTextField::initWithFrame(NSSecureTextField::alloc(mtm), NSRect::ZERO)
                        .into_super()
                } else {
                    NSTextField::initWithFrame(NSTextField::alloc(mtm), NSRect::ZERO)
                };
                input.setPlaceholderString(Some(&NSString::from_str(string(&field["label"]))));
                input.setStringValue(&NSString::from_str(string(&field["value"])));
                column_append(&fields, &input);
                input
            })
            .collect();
        fields.setFrameSize(NSSize::new(340.0, fields.fittingSize().height));
        alert.setAccessoryView(Some(&fields));
        alert.addButtonWithTitle(&NSString::from_str(string(&editor["confirm_label"])));
        alert.addButtonWithTitle(&NSString::from_str(string(&editor["cancel_label"])));
        if alert.runModal() == NSAlertFirstButtonReturn {
            let mut command = editor["command"].clone();
            command["values"] = json!(
                entries
                    .iter()
                    .map(|input| input.stringValue().to_string())
                    .collect::<Vec<_>>()
            );
            self.apply(&command);
        }
    }

    fn choose_font(&self, anchor: &NSButton) {
        let mtm = self.mtm();
        let current = preferences::snapshot(&self.ivars().store).default_text_font_family;
        let choices = NSFontManager::sharedFontManager(mtm)
            .availableFontFamilies()
            .iter()
            .map(|family| StringChoice {
                label: family.to_string(),
                value: family.to_string(),
            })
            .collect();
        let weak = Weak::new(self);
        let bounds = anchor.bounds();
        shrimply_components_appkit::show_searchable_menu_at(
            anchor,
            (
                NSPoint::new(bounds.origin.x + bounds.size.width / 2.0, bounds.origin.y),
                false,
            ),
            &text("Search Fonts"),
            choices,
            current.name().to_owned(),
            move |name| {
                if let Some(settings) = weak.load() {
                    settings.apply(&json!({"action":"desktop_font","name":name}));
                }
            },
            mtm,
        );
    }

    fn apply(&self, command: &Value) {
        let result = self
            .ivars()
            .settings
            .borrow_mut()
            .apply(&self.ivars().store, command);
        let document = match result {
            Ok(document) => document,
            Err(error) => {
                error_alert::show(self.mtm(), &error);
                return;
            }
        };
        self.render(&document);
        if let Some(path) = document["show_in_files"]
            .as_str()
            .and_then(NSURL::from_file_path)
        {
            NSWorkspace::sharedWorkspace()
                .activateFileViewerSelectingURLs(&NSArray::from_retained_slice(&[path]));
        }
        if let Some(picker) = document["source_picker"].as_object() {
            let panel = NSOpenPanel::openPanel(self.mtm());
            panel.setTitle(Some(&NSString::from_str(string(&picker["title"]))));
            let folder = picker["folder"] == true;
            panel.setCanChooseFiles(!folder);
            panel.setCanChooseDirectories(folder);
            panel.setAllowsMultipleSelection(false);
            let path = (panel.runModal() == NSModalResponseOK)
                .then(|| panel.URL()?.path().map(|value| value.to_string()))
                .flatten();
            let mut command = picker[if path.is_some() { "complete" } else { "cancel" }].clone();
            if let Some(path) = path {
                command["path"] = json!(path);
            }
            self.apply(&command);
        }
        if let Some(dialog) = document["dialog"].as_object() {
            let alert = NSAlert::new(self.mtm());
            alert.setMessageText(&NSString::from_str(string(&dialog["title"])));
            alert.setInformativeText(&NSString::from_str(string(&dialog["body"])));
            alert.addButtonWithTitle(&NSString::from_str(string(&dialog["close_label"])));
            alert.runModal();
            self.apply(&dialog["command"]);
        }
    }
}

fn items(value: &Value) -> &[Value] {
    value.as_array().map_or(&[], Vec::as_slice)
}

fn string(value: &Value) -> &str {
    value.as_str().unwrap_or_default()
}

fn settings_command(control: &Value) -> &Value {
    match string(&control["widget"]) {
        "editor" => &control["editor"]["command"],
        "choice" => &control["choices"][0]["command"],
        _ => &control["command"],
    }
}

fn settings_action(control: &Value) -> &str {
    string(&settings_command(control)["action"])
}

fn is_server_list_control(control: &Value) -> bool {
    matches!(
        settings_action(control),
        "project_server_sign_in"
            | "project_server_remove"
            | "server_add"
            | "server_edit"
            | "server_remove"
            | "server_select"
            | "server_check"
    )
}

fn settings_table(
    frame: NSRect,
    style: NSTableViewStyle,
    row_height: f64,
    mtm: MainThreadMarker,
) -> Retained<NSTableView> {
    let table = NSTableView::initWithFrame(NSTableView::alloc(mtm), frame);
    table.setStyle(style);
    table.setRowSizeStyle(NSTableViewRowSizeStyle::Custom);
    table.setRowHeight(row_height);
    table.setHeaderView(None);
    table.setBackgroundColor(&NSColor::clearColor());
    table.setColumnAutoresizingStyle(
        NSTableViewColumnAutoresizingStyle::UniformColumnAutoresizingStyle,
    );
    table.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
    let column =
        NSTableColumn::initWithIdentifier(NSTableColumn::alloc(mtm), ns_string!("settings"));
    column.setResizingMask(NSTableColumnResizingOptions::AutoresizingMask);
    if frame.size.width > 0.0 {
        column.setWidth(frame.size.width);
    }
    table.addTableColumn(&column);
    table
}

fn sidebar_row(page: PreferencePage, mtm: MainThreadMarker) -> Retained<NSView> {
    const TILE_SIZE: f64 = 20.0;
    let bounds = NSRect::new(NSPoint::ZERO, NSSize::new(TILE_SIZE, TILE_SIZE));
    let tile = NSView::initWithFrame(NSView::alloc(mtm), bounds);
    tile.setWantsLayer(true);
    let gradient = CAGradientLayer::layer();
    gradient.setFrame(bounds);
    gradient.setStartPoint(NSPoint::new(0.5, 1.0));
    gradient.setEndPoint(NSPoint::new(0.5, 0.0));
    gradient.setCornerRadius(5.0);
    gradient.setCornerCurve(unsafe { kCACornerCurveContinuous });
    gradient.setMasksToBounds(true);
    let rgb = u32::from_str_radix(&page.color()[1..], 16).expect("settings color");
    let base = NSColor::colorWithDeviceRed_green_blue_alpha(
        f64::from((rgb >> 16) & 255) / 255.0,
        f64::from((rgb >> 8) & 255) / 255.0,
        f64::from(rgb & 255) / 255.0,
        1.0,
    );
    let color = NSColor::colorWithHue_saturation_brightness_alpha(
        base.hueComponent(),
        (base.saturationComponent() + 0.12).min(1.0),
        (base.brightnessComponent() + 0.09).min(1.0),
        1.0,
    );
    let top = color
        .blendedColorWithFraction_ofColor(0.12, &NSColor::whiteColor())
        .expect("sRGB blend")
        .CGColor();
    let bottom = color
        .blendedColorWithFraction_ofColor(0.16, &NSColor::blackColor())
        .expect("sRGB blend")
        .CGColor();
    // Core Animation takes CGColor objects bridged as Objective-C objects in its NSArray.
    unsafe {
        let colors = [&*top, &*bottom].map(|color| &*std::ptr::from_ref(color).cast::<AnyObject>());
        gradient.setColors(Some(&NSArray::from_slice(&colors)));
    }
    gradient.setBorderWidth(0.5);
    gradient.setBorderColor(Some(
        &NSColor::whiteColor()
            .colorWithAlphaComponent(0.18)
            .CGColor(),
    ));
    let layer = tile.layer().expect("sidebar icon backing layer");
    layer.setShadowOpacity(0.22);
    layer.setShadowRadius(1.4);
    layer.setShadowOffset(NSSize::new(0.0, -1.0));
    layer.addSublayer(&gradient);
    let title = text(page.title());
    let glyph = bundled_symbol(page.icon(), &title);
    let image = NSImageView::imageViewWithImage(&glyph, mtm);
    image.setContentTintColor(Some(&NSColor::whiteColor()));
    image.setFrame(bounds);
    image.setAccessibilityElement(false);
    tile.addSubview(&image);
    tile.widthAnchor()
        .constraintEqualToConstant(TILE_SIZE)
        .setActive(true);
    tile.heightAnchor()
        .constraintEqualToConstant(TILE_SIZE)
        .setActive(true);

    let row = row_stack(8.0, mtm);
    let label = settings_label(&title, mtm);
    row.addArrangedSubview(&tile);
    row.addArrangedSubview(&label);
    settings_table_cell(&row, &label, mtm)
}

fn settings_table_cell(
    view: &NSView,
    label: &NSTextField,
    mtm: MainThreadMarker,
) -> Retained<NSView> {
    let cell = NSTableCellView::new(mtm);
    cell.addSubview(view);
    unsafe { cell.setTextField(Some(label)) };
    view.setTranslatesAutoresizingMaskIntoConstraints(false);
    for constraint in [
        view.leadingAnchor()
            .constraintEqualToAnchor_constant(&cell.leadingAnchor(), 5.0),
        view.trailingAnchor()
            .constraintEqualToAnchor_constant(&cell.trailingAnchor(), -5.0),
        view.centerYAnchor()
            .constraintEqualToAnchor(&cell.centerYAnchor()),
    ] {
        constraint.setActive(true);
    }
    cell.into_super()
}

fn settings_label(title: &str, mtm: MainThreadMarker) -> Retained<NSTextField> {
    let title = NSString::from_str(title);
    let label = NSTextField::labelWithString(&title, mtm);
    label.setLineBreakMode(NSLineBreakMode::ByTruncatingMiddle);
    label.setToolTip(Some(&title));
    label
}

fn settings_row(title: &str, subtitle: &str, value: Option<&AnyObject>) -> Retained<FormParts> {
    if let Some(control) = value.and_then(|value| value.downcast_ref::<NSControl>()) {
        control.setControlSize(NSControlSize::Small);
    }
    let title = NSString::from_str(title);
    let subtitle = NSString::from_str(subtitle);
    let mut parts: Vec<&AnyObject> = vec![title.as_ref(), subtitle.as_ref()];
    parts.extend(value);
    NSArray::from_slice(&parts)
}

fn settings_separator(mtm: MainThreadMarker) -> Retained<NSView> {
    let separator = NSBox::new(mtm);
    separator.setBoxType(NSBoxType::Separator);
    separator
        .heightAnchor()
        .constraintEqualToConstant(1.0)
        .setActive(true);
    inset(
        &separator,
        NSEdgeInsets {
            top: 0.0,
            bottom: 0.0,
            ..ROW_INSET
        },
        mtm,
    )
}
