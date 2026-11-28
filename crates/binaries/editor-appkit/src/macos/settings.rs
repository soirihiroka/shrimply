use super::error_alert;
use objc2::rc::{Retained, Weak};
use objc2::runtime::ProtocolObject;
use objc2::{DefinedClass, MainThreadOnly, Message, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSBackingStoreType, NSButton, NSCellImagePosition, NSColor,
    NSFont, NSFontManager, NSLayoutAttribute, NSModalResponseOK, NSOpenPanel, NSPopUpButton,
    NSSecureTextField, NSStackView, NSTextAlignment, NSTextField, NSView, NSWindow,
    NSWindowDelegate, NSWindowStyleMask, NSWorkspace,
};
use objc2_foundation::{
    MainThreadMarker, NSArray, NSEdgeInsets, NSNotification, NSObject, NSObjectProtocol, NSPoint,
    NSRect, NSSize, NSString, NSTimer, NSURL,
};
use serde_json::{Value, json};
use shrimply_components_appkit::{
    ActionButton, ColorPicker, NumberPicker, ScrollingColumn, StringChoice, StringSelector,
    bundled_symbol, column_append, column_stack, control_row, inset, row_stack, switch_row,
};
use shrimply_editor_state::preferences::{
    self, PreferenceId, PreferencePage, PreferenceValue, SharedPreferences,
};
use shrimply_i18n::text;
use shrimply_inspector_document::preferences::Preferences;
use std::cell::{Cell, OnceCell, RefCell};

struct Layout {
    window: Retained<NSWindow>,
    sidebar: Retained<NSStackView>,
    compact: Retained<NSPopUpButton>,
    categories: Vec<Retained<NSButton>>,
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

    pub fn present_from_launcher(&self, external: bool, on_close: impl Fn() + 'static) {
        self.ivars().on_close.replace(Some(Box::new(on_close)));
        self.present();
        if external {
            self.select(
                PreferencePage::ALL
                    .iter()
                    .position(|id| *id == PreferencePage::External)
                    .expect("External settings page"),
            );
        }
    }

    pub fn present(&self) {
        let layout = self.ivars().layout.get_or_init(|| self.build());
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
        let size = NSSize::new(900.0, 610.0);
        let rect = NSRect::new(NSPoint::ZERO, size);
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                rect,
                NSWindowStyleMask::Titled
                    | NSWindowStyleMask::Closable
                    | NSWindowStyleMask::Resizable,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        unsafe { window.setReleasedWhenClosed(false) };
        window.setTitle(&NSString::from_str(&text(preferences::SETTINGS_TITLE)));
        window.setContentMinSize(NSSize::new(460.0, 390.0));
        window.setDelegate(Some(ProtocolObject::from_ref(self)));

        let sidebar = column_stack(7.0, mtm);
        let compact = shrimply_components_appkit::menu::popup(NSRect::ZERO, false, mtm);
        let categories = PreferencePage::ALL
            .iter()
            .enumerate()
            .map(|(index, page)| {
                let title = text(page.title());
                compact.addItemWithTitle(&NSString::from_str(&title));
                let image = bundled_symbol(page.icon(), &title);
                compact
                    .itemAtIndex(index as isize)
                    .expect("category menu item")
                    .setImage(Some(&image));
                let weak = Weak::new(self);
                let button = ActionButton::new(
                    &title,
                    move || {
                        if let Some(settings) = weak.load() {
                            settings.select(index);
                        }
                    },
                    mtm,
                );
                button.view().setImage(Some(&image));
                button
                    .view()
                    .setImagePosition(NSCellImagePosition::ImageLeading);
                button.view().setAlignment(NSTextAlignment::Left);
                let rgb = u32::from_str_radix(&page.color()[1..], 16).expect("settings color");
                let color = NSColor::colorWithSRGBRed_green_blue_alpha(
                    f64::from((rgb >> 16) & 255) / 255.0,
                    f64::from((rgb >> 8) & 255) / 255.0,
                    f64::from(rgb & 255) / 255.0,
                    1.0,
                );
                button.view().setContentTintColor(Some(&color));
                column_append(&sidebar, button.view());
                Retained::from(button.view())
            })
            .collect();
        unsafe {
            compact.setTarget(Some(self));
            compact.setAction(Some(sel!(selectSettingsPage:)));
        }

        let body = column_stack(16.0, mtm);
        let adjusted = inset(
            &body,
            NSEdgeInsets {
                top: 18.0,
                left: 22.0,
                bottom: 18.0,
                right: 22.0,
            },
            mtm,
        );
        let scroll = ScrollingColumn::new(&adjusted, mtm);
        let detail = column_stack(8.0, mtm);
        column_append(&detail, &compact);
        column_append(&detail, scroll.view());
        let root = row_stack(10.0, mtm);
        root.setAlignment(NSLayoutAttribute::Top);
        sidebar
            .widthAnchor()
            .constraintEqualToConstant(f64::from(preferences::SETTINGS_SIDEBAR_WIDTH))
            .setActive(true);
        root.addArrangedSubview(&sidebar);
        root.addArrangedSubview(&detail);
        detail
            .heightAnchor()
            .constraintEqualToAnchor(&root.heightAnchor())
            .setActive(true);
        let content = inset(
            &root,
            NSEdgeInsets {
                top: 12.0,
                left: 12.0,
                bottom: 12.0,
                right: 12.0,
            },
            mtm,
        );
        window.setContentView(Some(&content));
        window.center();
        Layout {
            window,
            sidebar,
            compact,
            categories,
            body,
            scroll,
        }
    }

    fn resize_navigation(&self) {
        if let Some(layout) = self.ivars().layout.get() {
            let wide =
                layout.window.frame().size.width >= f64::from(preferences::SETTINGS_WIDE_WIDTH);
            layout.sidebar.setHidden(!wide);
            layout.compact.setHidden(wide);
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
        layout.compact.selectItemAtIndex(index as isize);
        for (position, button) in layout.categories.iter().enumerate() {
            button.setBordered(position == index);
        }
        if *self.ivars().displayed.borrow() == *page {
            return;
        }
        self.ivars().displayed.replace(page.clone());
        for view in layout.body.arrangedSubviews().iter() {
            layout.body.removeArrangedSubview(&view);
            view.removeFromSuperview();
        }
        let mtm = self.mtm();
        let title = label(&page["label"], mtm);
        title.setFont(Some(&NSFont::boldSystemFontOfSize(21.0)));
        column_append(&layout.body, &title);
        for section in items(&page["controls"]) {
            let group = column_stack(7.0, mtm);
            let title = label(&section["label"], mtm);
            title.setFont(Some(&NSFont::boldSystemFontOfSize(13.0)));
            column_append(&group, &title);
            for control in items(&section["controls"]) {
                column_append(&group, &self.control(control));
            }
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
                switch_row(
                    title,
                    control["subtitle"].as_str(),
                    control["selected"] == true,
                    move |enabled| {
                        let mut command = command.clone();
                        command["value"] = json!(enabled);
                        send(command);
                    },
                    mtm,
                )
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
                if let Some(control) = picker.view().downcast_ref::<objc2_app_kit::NSControl>() {
                    control.setEnabled(enabled);
                }
                control_row(title, picker.view(), mtm)
            }
            "editor" => self.editor_row(title, display, &control["editor"], enabled),
            "font" => {
                let title_button = if display.is_empty() {
                    text("Choose…").into_owned()
                } else {
                    display.into()
                };
                let button = ActionButton::new(
                    &title_button,
                    {
                        let weak = Weak::new(self);
                        move || {
                            if let Some(settings) = weak.load() {
                                settings.choose_font();
                            }
                        }
                    },
                    mtm,
                );
                control_row(title, button.view(), mtm)
            }
            "button" => {
                let command = control["command"].clone();
                let button = ActionButton::new(title, move || send(command.clone()), mtm);
                button.view().setEnabled(enabled);
                let subtitle = string(&control["subtitle"]);
                if subtitle.is_empty() {
                    let view: &NSView = button.view();
                    view.retain()
                } else {
                    control_row(subtitle, button.view(), mtm)
                }
            }
            _ => {
                let value = NSTextField::wrappingLabelWithString(&NSString::from_str(display), mtm);
                value.setTextColor(Some(&NSColor::secondaryLabelColor()));
                value.setSelectable(true);
                control_row(title, &value, mtm)
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
                return control_row(title, &picker, mtm);
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
                return control_row(title, picker.view(), mtm);
            }
        }
        let editor = editor.clone();
        let button = ActionButton::new(
            text(if display.is_empty() {
                "Choose…"
            } else {
                "Edit…"
            })
            .as_ref(),
            move || {
                if let Some(settings) = weak.load() {
                    settings.edit(&editor);
                }
            },
            mtm,
        );
        button.view().setEnabled(enabled);
        if display.is_empty() {
            control_row(title, button.view(), mtm)
        } else {
            control_row(&format!("{title} · {display}"), button.view(), mtm)
        }
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

    fn choose_font(&self) {
        let mtm = self.mtm();
        let layout = self.ivars().layout.get().expect("settings layout");
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
        shrimply_components_appkit::show_searchable_menu_at(
            &layout.body,
            (NSPoint::new(20.0, 20.0), false),
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

fn label(value: &Value, mtm: MainThreadMarker) -> Retained<NSTextField> {
    NSTextField::labelWithString(&NSString::from_str(string(value)), mtm)
}
