use hashbrown::HashMap;
use shrimply_gtk_components::tr;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use adw::prelude::AdwDialogExt;
use gtk::glib;
use gtk::prelude::*;
use shrimply_project::project::FontFamily as ProjectFontFamily;
use skia_safe::{FontMgr, FontStyle, Typeface};

use crate::Color;
use crate::font_cache::{self, FontFamily, FontSource, GoogleFamily};
use crate::timeline::renderer::{TimelineRenderer, Vec2};

const DIALOG_WIDTH: i32 = 1180;
const DIALOG_HEIGHT: i32 = 760;
const SEARCH_WIDTH: i32 = 400;
const MAX_TYPEFACE_CACHE_ENTRIES: usize = 128;
const MAX_PREVIEW_WORKERS: usize = 2;
const GOOGLE_LOOKUP_DELAY: Duration = Duration::from_millis(500);

thread_local! {
    static FONT_BROWSER: Rc<RefCell<shrimply_inspector_core::font_selector::Browser>> =
        Rc::new(RefCell::new(shrimply_inspector_core::font_selector::Browser::default()));
}

pub(crate) fn font_selector(
    selected_family: &str,
    selected_google: bool,
    on_change: impl Fn(FontFamily) + 'static,
) -> gtk::Widget {
    let selected_label = if selected_family.trim().is_empty() {
        tr!("Choose font").into_owned()
    } else {
        selected_family.trim().to_string()
    };
    let label = gtk::Label::builder()
        .label(&selected_label)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .halign(gtk::Align::Start)
        .xalign(0.0)
        .hexpand(true)
        .build();
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    content.append(&label);
    let source_label = gtk::Label::builder()
        .label(tr!("Google").as_ref())
        .css_classes(["caption", "dim-label"])
        .visible(selected_google)
        .build();
    content.append(&source_label);
    content.append(&gtk::Image::from_icon_name("external-link-symbolic"));
    let button = gtk::Button::builder()
        .child(&content)
        .halign(gtk::Align::Fill)
        .hexpand(true)
        .build();
    let on_change: Rc<dyn Fn(FontFamily)> = Rc::new(on_change);
    button.connect_clicked({
        let selected_family = selected_family.trim().to_string();
        let on_change = on_change.clone();
        let label = label.clone();
        let source_label = source_label.clone();
        move |button| {
            show_font_dialog(button, Some((selected_family.clone(), selected_google)), {
                let label = label.clone();
                let source_label = source_label.clone();
                let on_change = on_change.clone();
                Rc::new(move |choice: FontFamily| {
                    label.set_label(&choice.name);
                    source_label.set_visible(choice.source == FontSource::Google);
                    on_change(choice);
                })
            });
        }
    });
    button.upcast()
}

pub(crate) fn project_font_selector(
    selected: &ProjectFontFamily,
    on_change: impl Fn(ProjectFontFamily) + 'static,
) -> gtk::Widget {
    let selected_google = matches!(selected, ProjectFontFamily::GoogleFonts { .. });
    font_selector(selected.name(), selected_google, move |choice| {
        on_change(font_cache::project_family(&choice));
    })
}

pub(crate) fn font_selector_list(
    selected_families: &[ProjectFontFamily],
    on_change: impl Fn(Vec<ProjectFontFamily>) + 'static,
) -> gtk::Widget {
    let list = gtk::Box::new(gtk::Orientation::Vertical, 2);
    list.set_hexpand(true);
    let selected_families =
        shrimply_inspector_core::font_selector::normalized_families(selected_families);
    let on_change = Rc::new(on_change);

    if selected_families.is_empty() {
        list.append(
            &gtk::Label::builder()
                .label(tr!("System default").as_ref())
                .halign(gtk::Align::Start)
                .css_classes(["dim-label"])
                .build(),
        );
    }

    for (index, family) in selected_families.iter().enumerate() {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        let selected_google = matches!(family, ProjectFontFamily::GoogleFonts { .. });
        row.append(&font_selector(family.name(), selected_google, {
            let selected_families = selected_families.clone();
            let on_change = on_change.clone();
            move |choice| {
                if let Some(next) = shrimply_inspector_core::font_selector::replace_family(
                    &selected_families,
                    index,
                    font_cache::project_family(&choice),
                ) {
                    on_change(next);
                }
            }
        }));
        row.append(&font_order_button(
            "go-up-symbolic",
            "Move up",
            index > 0,
            {
                let selected_families = selected_families.clone();
                let on_change = on_change.clone();
                move || {
                    if let Some(next) = shrimply_inspector_core::font_selector::move_family(
                        &selected_families,
                        index,
                        -1,
                    ) {
                        on_change(next);
                    }
                }
            },
        ));
        row.append(&font_order_button(
            "go-down-symbolic",
            "Move down",
            index + 1 < selected_families.len(),
            {
                let selected_families = selected_families.clone();
                let on_change = on_change.clone();
                move || {
                    if let Some(next) = shrimply_inspector_core::font_selector::move_family(
                        &selected_families,
                        index,
                        1,
                    ) {
                        on_change(next);
                    }
                }
            },
        ));
        row.append(&font_order_button("user-trash-symbolic", "Remove", true, {
            let selected_families = selected_families.clone();
            let on_change = on_change.clone();
            move || {
                if let Some(next) =
                    shrimply_inspector_core::font_selector::remove_family(&selected_families, index)
                {
                    on_change(next);
                }
            }
        }));
        list.append(&row);
    }

    let add = gtk::Button::builder()
        .icon_name("list-add-symbolic")
        .tooltip_text(tr!("Add font").as_ref())
        .build();
    add.set_halign(gtk::Align::End);
    add.add_css_class("flat");
    add.connect_clicked({
        let selected_families = selected_families.clone();
        move |button| {
            let selected_families = selected_families.clone();
            let on_change = on_change.clone();
            show_font_dialog(
                button,
                None,
                Rc::new(move |choice| {
                    if let Some(next) = shrimply_inspector_core::font_selector::append_family(
                        &selected_families,
                        font_cache::project_family(&choice),
                    ) {
                        on_change(next);
                    }
                }),
            );
        }
    });
    list.append(&add);
    list.upcast()
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct TypefaceKey {
    name: String,
    source: FontSource,
    revision: i64,
}

#[derive(Clone)]
struct FontBrowserState {
    dialog: adw::Dialog,
    search: gtk::SearchEntry,
    status: gtk::Label,
    area: gtk::GLArea,
    browser: Rc<RefCell<shrimply_inspector_core::font_selector::Browser>>,
    visible_families: Rc<RefCell<Vec<FontFamily>>>,
    selected: Option<(String, bool)>,
    on_change: Rc<dyn Fn(FontFamily)>,
    scroll: Rc<Cell<f32>>,
    pointer: Rc<Cell<Option<Vec2>>>,
    scrollbar: Rc<RefCell<shrimply_skia_adw_core::slider::Lifecycle>>,
    scrollbar_drag_y: Rc<Cell<Option<f64>>>,
    hovered: Rc<Cell<Option<usize>>>,
    pressed: Rc<Cell<Option<usize>>>,
    buttons: Rc<RefCell<Vec<shrimply_skia_adw_core::button::Button>>>,
    font_loader: Rc<RefCell<shrimply_skia_adw_core::font_grid::FontLoader<TypefaceKey>>>,
    remote_families: Arc<Mutex<HashMap<String, GoogleFamily>>>,
    needs_animation: Rc<Cell<bool>>,
    started_at: Instant,
}

fn show_font_dialog(
    parent: &gtk::Button,
    selected: Option<(String, bool)>,
    on_change: Rc<dyn Fn(FontFamily)>,
) {
    let dialog = adw::Dialog::builder()
        .title(tr!("Fonts").as_ref())
        .content_width(DIALOG_WIDTH)
        .content_height(DIALOG_HEIGHT)
        .build();
    let search = gtk::SearchEntry::builder()
        .placeholder_text(tr!("Search fonts or paste a Google Fonts specimen URL").as_ref())
        .width_request(SEARCH_WIDTH)
        .max_width_chars(64)
        .build();
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&search));

    let area = gtk::GLArea::builder()
        .auto_render(false)
        .has_depth_buffer(false)
        .has_stencil_buffer(false)
        .hexpand(true)
        .vexpand(true)
        .focusable(true)
        .build();
    let status = gtk::Label::builder()
        .halign(gtk::Align::Center)
        .css_classes(["caption", "dim-label"])
        .visible(false)
        .margin_top(3)
        .margin_bottom(3)
        .build();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&status);
    content.append(&area);
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&content));
    dialog.set_child(Some(&toolbar));

    let worker_count = thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .min(MAX_PREVIEW_WORKERS);
    let remote_families = Arc::new(Mutex::new(HashMap::new()));
    let loader_remote_families = remote_families.clone();
    let browser = FONT_BROWSER.with(Clone::clone);
    browser.borrow_mut().set_query("");
    let state = FontBrowserState {
        dialog: dialog.clone(),
        search: search.clone(),
        status,
        area: area.clone(),
        browser,
        visible_families: Rc::new(RefCell::new(Vec::new())),
        selected,
        on_change,
        scroll: Rc::new(Cell::new(0.0)),
        pointer: Rc::new(Cell::new(None)),
        scrollbar: Rc::new(RefCell::new(
            shrimply_skia_adw_core::slider::Lifecycle::default(),
        )),
        scrollbar_drag_y: Rc::new(Cell::new(None)),
        hovered: Rc::new(Cell::new(None)),
        pressed: Rc::new(Cell::new(None)),
        buttons: Rc::new(RefCell::new(Vec::new())),
        font_loader: Rc::new(RefCell::new(
            shrimply_skia_adw_core::font_grid::FontLoader::new(
                worker_count,
                MAX_TYPEFACE_CACHE_ENTRIES,
                move |key| load_typeface(key, &loader_remote_families),
                load_label_typeface,
            ),
        )),
        remote_families,
        needs_animation: Rc::new(Cell::new(false)),
        started_at: Instant::now(),
    };
    connect_canvas(&state);
    search.connect_search_changed({
        let state = state.clone();
        move |_| state.search_changed()
    });
    search.connect_activate({
        let state = state.clone();
        move |_| state.lookup_now()
    });
    state.browser.borrow_mut().open();
    state.rebuild();
    dialog.present(Some(parent.upcast_ref::<gtk::Widget>()));
    search.grab_focus();
    state.update_status();
}

impl FontBrowserState {
    fn rebuild(&self) {
        let visible = self.browser.borrow().visible().to_vec();
        self.scroll.set(0.0);
        self.hovered.set(None);
        self.pressed.set(None);
        self.buttons.replace(vec![
            shrimply_skia_adw_core::button::Button::default();
            visible.len()
        ]);
        self.visible_families.replace(visible);
        self.area.queue_render();
    }

    fn search_changed(&self) {
        let generation = self
            .browser
            .borrow_mut()
            .set_query(self.search.text().to_string());
        self.rebuild();
        self.update_status();
        let state = self.clone();
        glib::timeout_add_local_once(GOOGLE_LOOKUP_DELAY, move || {
            state.browser.borrow_mut().begin_lookup(generation);
            state.update_status();
        });
    }

    fn lookup_now(&self) {
        let generation = self
            .browser
            .borrow_mut()
            .set_query(self.search.text().to_string());
        self.rebuild();
        self.browser.borrow_mut().begin_lookup(generation);
        self.update_status();
    }

    fn activate(&self, position: usize) {
        let Some(family) = self.visible_families.borrow().get(position).cloned() else {
            return;
        };
        if family.source == FontSource::Local {
            (self.on_change)(family);
            self.dialog.close();
            return;
        }
        if let Err(error) = self.browser.borrow_mut().activate(family) {
            show_status(&self.status, &error);
        } else {
            self.update_status();
        }
    }

    fn update_hover(&self, x: f64, y: f64) {
        let next = shrimply_skia_adw_core::font_grid::hit_test(
            self.area.width().max(1) as f32,
            self.scroll.get(),
            glam::Vec2::new(x as f32, y as f32),
            self.visible_families.borrow().len(),
        );
        let previous = self.hovered.replace(next);
        if previous == next {
            return;
        }
        let mut buttons = self.buttons.borrow_mut();
        if let Some(index) = previous.and_then(|index| buttons.get_mut(index)) {
            index.event(shrimply_skia_adw_core::button::Event::PointerLeft);
        }
        if let Some(index) = next.and_then(|index| buttons.get_mut(index)) {
            index.event(shrimply_skia_adw_core::button::Event::PointerEntered);
        }
        self.area.set_cursor_from_name(next.map(|_| "pointer"));
        self.needs_animation.set(true);
        self.area.queue_render();
    }

    fn poll(&self) {
        let poll = self.browser.borrow_mut().poll();
        if poll.visible_changed {
            if let Some(family) = self.browser.borrow().lookup() {
                self.remote_families
                    .lock()
                    .unwrap_or_else(|_| panic!("remote font registry lock died"))
                    .insert(family.name.to_lowercase(), family.clone());
            }
            self.rebuild();
        } else if poll.changed {
            self.area.queue_render();
        }
        self.update_status();
        for result in poll.activations {
            match result {
                Ok(family) => {
                    (self.on_change)(family);
                    self.dialog.close();
                }
                Err(error) => show_status(&self.status, &error),
            }
        }
    }

    fn update_status(&self) {
        let browser = self.browser.borrow();
        self.status.set_label(browser.status());
        self.status.set_visible(!browser.status().is_empty());
    }
}

fn connect_canvas(state: &FontBrowserState) {
    let renderer = Rc::new(RefCell::new(TimelineRenderer::new()));
    state.area.connect_render({
        let state = state.clone();
        let renderer = renderer.clone();
        move |area, _| {
            if let Some(error) = area.error() {
                tracing::error!("Font selector GLArea error: {error}");
                return glib::Propagation::Stop;
            }
            area.make_current();
            if let Some(error) = area.error() {
                tracing::error!("Font selector GLArea error after make_current: {error}");
                return glib::Propagation::Stop;
            }
            let width = area.width().max(1);
            let height = area.height().max(1);
            let scale = area.scale_factor().max(1) as f32;
            let dark = adw::StyleManager::for_display(&area.display()).is_dark();
            let background = if dark {
                Color::VIEW_BG_DARK
            } else {
                Color::VIEW_BG_LIGHT
            };
            let foreground = if dark {
                Color::VIEW_FG_DARK
            } else {
                Color::VIEW_FG_LIGHT
            };
            let maximum_scroll = (shrimply_skia_adw_core::font_grid::content_height(
                width as f32,
                state.visible_families.borrow().len(),
            ) - height as f32)
                .max(0.0);
            state.scroll.set(state.scroll.get().min(maximum_scroll));
            let mut renderer = renderer.borrow_mut();
            let painter = match renderer.begin_frame(
                glam::UVec2::new(
                    (width as f32 * scale).round() as u32,
                    (height as f32 * scale).round() as u32,
                ),
                scale,
                background,
            ) {
                Ok(painter) => painter,
                Err(error) => {
                    tracing::error!("Could not initialize Skia font selector: {error}");
                    return glib::Propagation::Stop;
                }
            };
            let families = state.visible_families.borrow();
            let mut font_loader = state.font_loader.borrow_mut();
            let visible = font_loader.prepare_visible(
                width as f32,
                state.scroll.get(),
                height as f32,
                families.len(),
                |index| TypefaceKey {
                    name: families[index].name.clone(),
                    source: families[index].source,
                    revision: families[index].revision,
                },
            );
            let mut buttons = state.buttons.borrow_mut();
            let label_typeface = font_loader.label_typeface();
            let mut animating = false;
            for index in visible {
                let family = &families[index];
                let key = TypefaceKey {
                    name: family.name.clone(),
                    source: family.source,
                    revision: family.revision,
                };
                let typeface = font_loader.typeface(&key);
                let selected = state.selected.as_ref().is_some_and(|(name, google)| {
                    name.eq_ignore_ascii_case(&family.name)
                        && *google == (family.source == FontSource::Google)
                });
                animating |= shrimply_skia_adw_core::font_grid::draw_specimen(
                    painter.canvas(),
                    shrimply_skia_adw_core::font_grid::cell_bounds(
                        width as f32,
                        state.scroll.get(),
                        index,
                    ),
                    shrimply_skia_adw_core::font_grid::Specimen {
                        label: &family.name,
                        label_typeface: label_typeface.as_ref(),
                        typeface: typeface.as_ref(),
                        selected,
                        button: &mut buttons[index],
                    },
                    foreground,
                    state.started_at.elapsed(),
                );
            }
            let scrollbar = font_scrollbar(
                width as f32,
                height as f32,
                families.len(),
                state.scroll.get(),
                foreground,
                background,
            );
            let scrollbar = state
                .scrollbar
                .borrow_mut()
                .frame(scrollbar, state.pointer.get());
            if let Some(scrollbar) = scrollbar.scrollbar {
                shrimply_skia_adw_core::draw_scrollbar(painter.canvas(), scrollbar);
            }
            animating |= scrollbar.animating;
            state.needs_animation.set(animating);
            if let Err(error) = renderer.end_frame() {
                tracing::error!("Could not finalize Skia font selector: {error}");
            }
            glib::Propagation::Stop
        }
    });
    state.area.add_tick_callback({
        let state = state.clone();
        move |area, _| {
            state.poll();
            let mut scroll = f64::from(state.scroll.get());
            let scrolling = state
                .scrollbar
                .borrow_mut()
                .apply_scroll(|value| scroll = value);
            state.scroll.set(scroll as f32);
            let has_loading_card = state.font_loader.borrow().is_loading();
            if has_loading_card || scrolling || state.needs_animation.get() {
                area.queue_render();
            }
            glib::ControlFlow::Continue
        }
    });
    state.area.connect_unrealize(move |area| {
        area.make_current();
        renderer.borrow_mut().destroy();
    });

    let motion = gtk::EventControllerMotion::new();
    motion.connect_motion({
        let state = state.clone();
        move |_, x, y| {
            state.pointer.set(Some(Vec2::new(x as f32, y as f32)));
            state.update_hover(x, y);
        }
    });
    motion.connect_leave({
        let state = state.clone();
        move |_| {
            state.pointer.set(None);
            if let Some(index) = state.hovered.take() {
                let mut buttons = state.buttons.borrow_mut();
                if let Some(button) = buttons.get_mut(index) {
                    button.event(shrimply_skia_adw_core::button::Event::PointerLeft);
                }
            }
            state.area.set_cursor_from_name(None);
            state.needs_animation.set(true);
            state.area.queue_render();
        }
    });
    state.area.add_controller(motion);

    let drag = gtk::GestureDrag::new();
    drag.set_button(1);
    drag.connect_drag_begin({
        let state = state.clone();
        move |_, x, y| {
            if let Some(scrollbar) = current_font_scrollbar(&state) {
                let mut scroll = f64::from(state.scroll.get());
                if matches!(
                    state.scrollbar.borrow_mut().begin(
                        scrollbar,
                        Vec2::new(x as f32, y as f32),
                        |value| {
                            scroll = value;
                        }
                    ),
                    shrimply_skia_adw_core::slider::Begin::Drag
                ) {
                    state.scroll.set(scroll as f32);
                    state.scrollbar_drag_y.set(Some(y));
                    state.area.queue_render();
                    return;
                }
            }
            state.update_hover(x, y);
            let pressed = state.hovered.get();
            state.pressed.set(pressed);
            if let Some(index) = pressed {
                let mut buttons = state.buttons.borrow_mut();
                if let Some(button) = buttons.get_mut(index) {
                    button.event(shrimply_skia_adw_core::button::Event::Pressed);
                }
            }
            state.needs_animation.set(true);
            state.area.queue_render();
        }
    });
    drag.connect_drag_update({
        let state = state.clone();
        move |_, _, offset_y| {
            if state.scrollbar_drag_y.get().is_none() {
                return;
            }
            let Some(scrollbar) = current_font_scrollbar(&state) else {
                return;
            };
            let mut scroll = f64::from(state.scroll.get());
            state
                .scrollbar
                .borrow_mut()
                .drag_by(scrollbar, offset_y, |value| scroll = value);
            state.scroll.set(scroll as f32);
            state.area.queue_render();
        }
    });
    drag.connect_drag_end({
        let state = state.clone();
        move |gesture, offset_x, offset_y| {
            if state.scrollbar_drag_y.take().is_some() {
                state.scrollbar.borrow_mut().end_drag();
                state.pressed.set(None);
                state.area.queue_render();
                return;
            }
            let position = gesture
                .start_point()
                .map(|(x, y)| Vec2::new((x + offset_x) as f32, (y + offset_y) as f32))
                .or_else(|| state.pointer.get())
                .unwrap_or_default();
            state.update_hover(f64::from(position.x), f64::from(position.y));
            let pressed = state.pressed.take();
            let clicked = pressed
                .and_then(|index| {
                    state.buttons.borrow_mut().get_mut(index).map(|button| {
                        button
                            .event(shrimply_skia_adw_core::button::Event::Released)
                            .clicked
                    })
                })
                .unwrap_or(false);
            state.needs_animation.set(true);
            state.area.queue_render();
            if clicked && let Some(index) = pressed {
                state.activate(index);
            }
        }
    });
    state.area.add_controller(drag);

    let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    scroll.connect_scroll({
        let state = state.clone();
        move |controller, _, dy| {
            let Some(scrollbar) = current_font_scrollbar(&state) else {
                return glib::Propagation::Proceed;
            };
            let input = if controller.unit() == gtk::gdk::ScrollUnit::Wheel {
                shrimply_skia_adw_core::slider::ScrollInput::Wheel
            } else {
                shrimply_skia_adw_core::slider::ScrollInput::Surface
            };
            let mut value = f64::from(state.scroll.get());
            let event = state.scrollbar.borrow_mut().scroll_at(
                scrollbar,
                state.pointer.get(),
                dy,
                input,
                |next| value = next,
            );
            if !event.handled {
                return glib::Propagation::Proceed;
            }
            state.scroll.set(value as f32);
            if let Some(index) = state.hovered.take() {
                let mut buttons = state.buttons.borrow_mut();
                if let Some(button) = buttons.get_mut(index) {
                    button.event(shrimply_skia_adw_core::button::Event::PointerLeft);
                }
            }
            state.area.queue_render();
            glib::Propagation::Stop
        }
    });
    state.area.add_controller(scroll);
}

fn load_typeface(
    key: &TypefaceKey,
    remote_families: &Mutex<HashMap<String, GoogleFamily>>,
) -> Result<Typeface, String> {
    match key.source {
        FontSource::Local => {
            thread_local! {
                static FONT_MANAGER: FontMgr = FontMgr::new();
            }
            FONT_MANAGER.with(|manager| {
                manager
                    .match_family_style(&key.name, FontStyle::default())
                    .ok_or_else(|| format!("local font {} is unavailable", key.name))
            })
        }
        FontSource::Google if key.revision < 0 => {
            let family = remote_families
                .lock()
                .unwrap_or_else(|_| panic!("remote font registry lock died"))
                .get(&key.name.to_lowercase())
                .cloned()
                .ok_or_else(|| format!("Google font {} is unavailable", key.name))?;
            font_cache::preview_google_family(&family)
        }
        FontSource::Google => font_cache::preview_typeface(&key.name),
    }
}

fn load_label_typeface() -> Result<Typeface, String> {
    let manager = FontMgr::new();
    ["Adwaita Sans", "Cantarell", "Noto Sans", "sans-serif"]
        .into_iter()
        .find_map(|family| manager.match_family_style(family, FontStyle::default()))
        .or_else(|| manager.legacy_make_typeface(None, FontStyle::default()))
        .ok_or_else(|| "no UI typeface is available".to_string())
}

fn current_font_scrollbar(state: &FontBrowserState) -> Option<shrimply_skia_adw_core::Scrollbar> {
    let dark = adw::StyleManager::for_display(&state.area.display()).is_dark();
    let (foreground, background) = if dark {
        (Color::VIEW_FG_DARK, Color::VIEW_BG_DARK)
    } else {
        (Color::VIEW_FG_LIGHT, Color::VIEW_BG_LIGHT)
    };
    font_scrollbar(
        state.area.width().max(1) as f32,
        state.area.height().max(1) as f32,
        state.visible_families.borrow().len(),
        state.scroll.get(),
        foreground,
        background,
    )
}

fn font_scrollbar(
    width: f32,
    height: f32,
    item_count: usize,
    value: f32,
    color: Color,
    outline_color: Color,
) -> Option<shrimply_skia_adw_core::Scrollbar> {
    let content_length = shrimply_skia_adw_core::font_grid::content_height(width, item_count);
    (content_length > height).then_some(shrimply_skia_adw_core::Scrollbar {
        axis: shrimply_skia_adw_core::Axis::Vertical,
        bounds: shrimply_skia_adw_core::Rect::from_xywh(0.0, 0.0, width, height),
        content_length: f64::from(content_length),
        viewport_length: f64::from(height),
        value: f64::from(value),
        color,
        outline_color,
        state: shrimply_skia_adw_core::slider::idle_state(),
    })
}

fn show_status(label: &gtk::Label, message: &str) {
    label.set_label(message);
    label.set_visible(true);
}

fn font_order_button(
    icon: &str,
    tooltip: &str,
    sensitive: bool,
    on_click: impl Fn() + 'static,
) -> gtk::Button {
    let button = gtk::Button::builder()
        .icon_name(icon)
        .tooltip_text(tr!(tooltip).as_ref())
        .sensitive(sensitive)
        .build();
    button.add_css_class("flat");
    button.connect_clicked(move |_| on_click());
    button
}
