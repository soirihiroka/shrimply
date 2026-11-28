use super::error_alert;
use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{DefinedClass, MainThreadOnly, Message, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAccessibility, NSAlert, NSAlertFirstButtonReturn, NSAutoresizingMaskOptions,
    NSBackingStoreType, NSBezelStyle, NSBox, NSBoxType, NSButton, NSColor, NSControl,
    NSControlTextEditingDelegate, NSFont, NSFontManager, NSImageView,
    NSLayoutConstraintOrientation, NSLayoutPriorityDefaultLow, NSLineBreakMode, NSModalResponseOK,
    NSOpenPanel, NSPopUpButton, NSScrollView, NSSecureTextField, NSSplitViewController,
    NSSplitViewDividerStyle, NSSplitViewItem, NSSplitViewItemCollapseBehavior, NSStackView,
    NSTableCellView, NSTableColumn, NSTableView, NSTableViewColumnAutoresizingStyle,
    NSTableViewDataSource, NSTableViewDelegate, NSTableViewRowSizeStyle, NSTableViewStyle,
    NSTextAlignment, NSTextField, NSToolbar, NSToolbarDelegate, NSToolbarItem,
    NSToolbarSidebarTrackingSeparatorItemIdentifier, NSView, NSViewController, NSWindow,
    NSWindowDelegate, NSWindowOrderingMode, NSWindowStyleMask, NSWindowTitleVisibility,
    NSWindowToolbarStyle, NSWorkspace,
};
use objc2_foundation::{
    MainThreadMarker, NSArray, NSEdgeInsets, NSIndexSet, NSNotification, NSObject,
    NSObjectProtocol, NSPoint, NSRect, NSSize, NSString, NSTimer, NSURL, ns_string,
};
use objc2_quartz_core::{CAGradientLayer, kCACornerCurveContinuous};
use serde_json::{Value, json};
use shrimply_components_appkit::{
    ActionButton, ColorPicker, NumberPicker, ScrollingColumn, StringChoice, StringSelector, Switch,
    bundled_symbol, column_append, column_stack, group_box, inset, row_stack,
};
use shrimply_editor_state::preferences::{
    self, PreferenceId, PreferencePage, PreferenceValue, SharedPreferences,
};
use shrimply_i18n::text;
use shrimply_inspector_document::preferences::Preferences;
use std::cell::{Cell, OnceCell, RefCell};

struct Layout {
    window: Retained<NSWindow>,
    sidebar: Retained<NSSplitViewItem>,
    navigation: Retained<NSTableView>,
    header: Retained<NSStackView>,
    title: Retained<NSTextField>,
    compact: Retained<NSPopUpButton>,
    body: Retained<NSStackView>,
    scroll: ScrollingColumn,
}

pub struct PreferencesIvars {
    store: SharedPreferences,
    settings: RefCell<Preferences>,
    layout: OnceCell<Layout>,
    selected: Cell<usize>,
    displayed: RefCell<Value>,
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
            NSArray::from_slice(&[
                unsafe { NSToolbarSidebarTrackingSeparatorItemIdentifier },
                ns_string!("settings-page"),
            ])
        }

        #[unsafe(method_id(toolbarAllowedItemIdentifiers:))]
        fn allowed_toolbar_items(&self, toolbar: &NSToolbar) -> Retained<NSArray<NSString>> {
            self.toolbar_items(toolbar)
        }

        #[unsafe(method_id(toolbar:itemForItemIdentifier:willBeInsertedIntoToolbar:))]
        fn toolbar_item(&self, _toolbar: &NSToolbar, identifier: &NSString, _inserted: bool) -> Option<Retained<NSToolbarItem>> {
            if identifier != ns_string!("settings-page") {
                return None;
            }
            let layout = self.ivars().layout.get().expect("settings layout");
            let item = NSToolbarItem::initWithItemIdentifier(NSToolbarItem::alloc(self.mtm()), identifier);
            item.setLabel(&NSString::from_str(&text(preferences::SETTINGS_TITLE)));
            item.setView(Some(&layout.header));
            Some(item)
        }
    }

    unsafe impl NSControlTextEditingDelegate for PreferencesWindow {}

    unsafe impl NSTableViewDataSource for PreferencesWindow {
        #[unsafe(method(numberOfRowsInTableView:))]
        fn category_count(&self, _table: &NSTableView) -> isize {
            PreferencePage::ALL.len() as isize
        }
    }

    unsafe impl NSTableViewDelegate for PreferencesWindow {
        #[unsafe(method_id(tableView:viewForTableColumn:row:))]
        fn category_row(&self, _table: &NSTableView, _column: Option<&NSTableColumn>, row: isize) -> Option<Retained<NSView>> {
            PreferencePage::ALL.get(row as usize).map(|page| sidebar_row(*page, self.mtm()))
        }

        #[unsafe(method(tableViewSelectionDidChange:))]
        fn category_changed(&self, _notification: &NSNotification) {
            if let Some(layout) = self.ivars().layout.get()
                && let Ok(index) = usize::try_from(layout.navigation.selectedRow())
            {
                self.select(index);
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
        window.setTitleVisibility(NSWindowTitleVisibility::Hidden);
        window.setContentMinSize(NSSize::new(460.0, 390.0));
        window.setDelegate(Some(ProtocolObject::from_ref(self)));

        let sidebar_width = f64::from(preferences::SETTINGS_SIDEBAR_WIDTH);
        let sidebar_rect = NSRect::new(NSPoint::ZERO, NSSize::new(sidebar_width, size.height));
        let navigation = NSTableView::initWithFrame(NSTableView::alloc(mtm), sidebar_rect);
        navigation.setStyle(NSTableViewStyle::SourceList);
        navigation.setRowSizeStyle(NSTableViewRowSizeStyle::Custom);
        navigation.setRowHeight(30.0);
        navigation.setHeaderView(None);
        navigation.setBackgroundColor(&NSColor::clearColor());
        navigation.setAllowsEmptySelection(false);
        navigation.setColumnAutoresizingStyle(
            NSTableViewColumnAutoresizingStyle::UniformColumnAutoresizingStyle,
        );
        navigation.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        let column =
            NSTableColumn::initWithIdentifier(NSTableColumn::alloc(mtm), ns_string!("category"));
        column.setWidth(sidebar_width);
        navigation.addTableColumn(&column);
        unsafe {
            navigation.setDataSource(Some(ProtocolObject::from_ref(self)));
            navigation.setDelegate(Some(ProtocolObject::from_ref(self)));
        }
        let sidebar_scroll = NSScrollView::initWithFrame(NSScrollView::alloc(mtm), sidebar_rect);
        sidebar_scroll.setDrawsBackground(false);
        sidebar_scroll.setDocumentView(Some(&navigation));
        let sidebar_controller = NSViewController::new(mtm);
        sidebar_controller.setView(&sidebar_scroll);
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
        let title = NSTextField::labelWithString(
            &NSString::from_str(&text(
                PreferencePage::ALL[self.ivars().selected.get()].title(),
            )),
            mtm,
        );
        title.setFont(Some(&NSFont::titleBarFontOfSize(0.0)));
        let header = row_stack(0.0, mtm);
        header.addArrangedSubview(&title);
        header.addArrangedSubview(&compact);
        compact.setHidden(true);
        header.setFrameSize(header.fittingSize());

        let body = column_stack(18.0, mtm);
        let adjusted = inset(
            &body,
            NSEdgeInsets {
                top: 18.0,
                left: 20.0,
                bottom: 18.0,
                right: 20.0,
            },
            mtm,
        );
        let scroll = ScrollingColumn::new(&adjusted, mtm);
        scroll.view().setDrawsBackground(false);
        let split = NSSplitViewController::new(mtm);
        split.splitView().setVertical(true);
        split
            .splitView()
            .setDividerStyle(NSSplitViewDividerStyle::Thin);
        split.addSplitViewItem(&sidebar);
        split.addSplitViewItem(&super::layout::split_item(scroll.view(), mtm));
        split.view().setFrame(rect);
        window.setContentViewController(Some(&split));
        window.center();
        Layout {
            window,
            sidebar,
            navigation,
            header,
            title,
            compact,
            body,
            scroll,
        }
    }

    fn resize_navigation(&self) {
        if let Some(layout) = self.ivars().layout.get() {
            let wide =
                layout.window.frame().size.width >= f64::from(preferences::SETTINGS_WIDE_WIDTH);
            if layout.sidebar.isCollapsed() == wide {
                layout.sidebar.setCollapsed(!wide);
                layout.title.setHidden(!wide);
                layout.compact.setHidden(wide);
                layout.header.setFrameSize(layout.header.fittingSize());
            }
        }
    }

    fn select(&self, index: usize) {
        if self.ivars().selected.replace(index) != index {
            if let Some(layout) = self.ivars().layout.get() {
                layout.scroll.set_position(0.0);
            }
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
        layout
            .title
            .setStringValue(&NSString::from_str(string(&page["label"])));
        layout.header.setFrameSize(layout.header.fittingSize());
        self.ivars().displayed.replace(page.clone());
        for view in layout.body.arrangedSubviews().iter() {
            layout.body.removeArrangedSubview(&view);
            view.removeFromSuperview();
        }
        let mtm = self.mtm();
        for section in items(&page["controls"]) {
            let group = column_stack(7.0, mtm);
            if section["label"] != page["label"] {
                let title = NSTextField::labelWithString(
                    &NSString::from_str(string(&section["label"])),
                    mtm,
                );
                title.setFont(Some(&NSFont::boldSystemFontOfSize(13.0)));
                column_append(&group, &title);
            }
            let rows = column_stack(0.0, mtm);
            for (index, control) in items(&section["controls"]).iter().enumerate() {
                if index != 0 {
                    let separator = NSBox::new(mtm);
                    separator.setBoxType(NSBoxType::Separator);
                    separator
                        .heightAnchor()
                        .constraintEqualToConstant(1.0)
                        .setActive(true);
                    column_append(&rows, &separator);
                }
                column_append(&rows, &self.control(control));
            }
            column_append(&group, &group_background(&rows, mtm));
            column_append(&layout.body, &group);
        }
    }

    fn control(&self, control: &Value) -> Retained<NSView> {
        let mtm = self.mtm();
        let title = string(&control["label"]);
        let display = string(&control["display"]);
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
                let subtitle = string(&control["subtitle"]);
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
                settings_row(title, subtitle, Some(toggle.view()), mtm)
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
                }
                settings_row(
                    title,
                    string(&control["choice_description"]),
                    Some(picker.view()),
                    mtm,
                )
            }
            "editor" => self.editor_row(title, display, &control["editor"], enabled),
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
                settings_row(title, "", Some(&button), mtm)
            }
            "button" => {
                let command = control["command"].clone();
                let button = ActionButton::new(title, move || send(command.clone()), mtm);
                button.view().setEnabled(enabled);
                let subtitle = string(&control["subtitle"]);
                settings_row(subtitle, "", Some(button.view()), mtm)
            }
            _ if display.is_empty() => settings_row(title, "", None, mtm),
            _ => {
                let value = settings_label(display, mtm);
                value.setAlignment(NSTextAlignment::Right);
                value.setTextColor(Some(&NSColor::secondaryLabelColor()));
                value.setSelectable(true);
                settings_row(title, "", Some(&value), mtm)
            }
        }
    }

    fn editor_row(
        &self,
        title: &str,
        display: &str,
        editor: &Value,
        enabled: bool,
    ) -> Retained<NSView> {
        let mtm = self.mtm();
        let id = editor["command"]["key"]
            .as_str()
            .and_then(PreferenceId::from_key);
        let weak = Weak::new(self);
        if let Some(id) = id {
            let value = preferences::value(&self.ivars().store, id);
            if let (PreferenceValue::Integer(value), Some(range)) =
                (&value, preferences::integer_range(id))
            {
                let scale = range.scale as f64;
                let command = editor["command"].clone();
                let picker = NumberPicker::builder(*value as f64 / scale)
                    .enabled(enabled)
                    .minimum(range.minimum as f64 / scale)
                    .maximum(range.maximum as f64 / scale)
                    .drag_step(range.step as f64 / scale)
                    .digits(match range.scale {
                        1 => 0,
                        10 => 1,
                        _ => 2,
                    })
                    .on_commit(move |value| {
                        if let Some(settings) = weak.load() {
                            let mut command = command.clone();
                            command["values"] = json!([value.to_string()]);
                            settings.apply(&command);
                        }
                    })
                    .build(mtm);
                picker
                    .widthAnchor()
                    .constraintEqualToConstant(80.0)
                    .setActive(true);
                return settings_row(title, "", Some(&picker), mtm);
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
                return settings_row(title, "", Some(picker.view()), mtm);
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
            (title, display)
        } else {
            (display, "")
        };
        settings_row(label, subtitle, Some(button.view()), mtm)
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

fn sidebar_row(page: PreferencePage, mtm: MainThreadMarker) -> Retained<NSView> {
    const TILE_SIZE: f64 = 22.0;
    let bounds = NSRect::new(NSPoint::ZERO, NSSize::new(TILE_SIZE, TILE_SIZE));
    let tile = NSView::initWithFrame(NSView::alloc(mtm), bounds);
    tile.setWantsLayer(true);
    let gradient = CAGradientLayer::layer();
    gradient.setFrame(bounds);
    gradient.setCornerRadius(5.5);
    gradient.setCornerCurve(unsafe { kCACornerCurveContinuous });
    gradient.setMasksToBounds(true);
    let rgb = u32::from_str_radix(&page.color()[1..], 16).expect("settings color");
    let color = NSColor::colorWithSRGBRed_green_blue_alpha(
        f64::from((rgb >> 16) & 255) / 255.0,
        f64::from((rgb >> 8) & 255) / 255.0,
        f64::from(rgb & 255) / 255.0,
        1.0,
    );
    let top = color
        .blendedColorWithFraction_ofColor(0.22, &NSColor::whiteColor())
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
    tile.layer()
        .expect("sidebar icon backing layer")
        .addSublayer(&gradient);
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
    let cell = NSTableCellView::new(mtm);
    cell.addSubview(&row);
    unsafe { cell.setTextField(Some(&label)) };
    row.setTranslatesAutoresizingMaskIntoConstraints(false);
    for constraint in [
        row.leadingAnchor()
            .constraintEqualToAnchor_constant(&cell.leadingAnchor(), 5.0),
        row.trailingAnchor()
            .constraintEqualToAnchor_constant(&cell.trailingAnchor(), -5.0),
        row.centerYAnchor()
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

fn settings_row(
    title: &str,
    subtitle: &str,
    control: Option<&NSView>,
    mtm: MainThreadMarker,
) -> Retained<NSView> {
    let captions = column_stack(2.0, mtm);
    if !title.is_empty() {
        column_append(&captions, &settings_label(title, mtm));
    }
    if !subtitle.is_empty() {
        let subtitle = NSTextField::wrappingLabelWithString(&NSString::from_str(subtitle), mtm);
        subtitle.setTextColor(Some(&NSColor::secondaryLabelColor()));
        column_append(&captions, &subtitle);
    }
    let row = row_stack(12.0, mtm);
    row.addArrangedSubview(&captions);
    captions.setHuggingPriority_forOrientation(
        NSLayoutPriorityDefaultLow - 1.0,
        NSLayoutConstraintOrientation::Horizontal,
    );
    if let Some(control) = control {
        row.addArrangedSubview(control);
        control
            .widthAnchor()
            .constraintLessThanOrEqualToAnchor_multiplier(&row.widthAnchor(), 0.60)
            .setActive(true);
    }
    row.heightAnchor()
        .constraintGreaterThanOrEqualToConstant(30.0)
        .setActive(true);
    inset(
        &row,
        NSEdgeInsets {
            top: 4.0,
            bottom: 4.0,
            left: 12.0,
            right: 12.0,
        },
        mtm,
    )
}

fn group_background(rows: &NSStackView, mtm: MainThreadMarker) -> Retained<NSView> {
    let group = inset(
        rows,
        NSEdgeInsets {
            top: 0.0,
            bottom: 0.0,
            left: 0.0,
            right: 0.0,
        },
        mtm,
    );
    let background = group_box(mtm);
    background.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    group.addSubview_positioned_relativeTo(&background, NSWindowOrderingMode::Below, Some(rows));
    group
}
