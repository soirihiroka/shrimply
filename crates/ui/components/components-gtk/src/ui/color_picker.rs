use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::cairo;
use gtk::gio;
use gtk::gio::prelude::SettingsExt;
use gtk::glib;
use shrimply_components_core::color::picker::{
    BAR_SIZE, CHECKER_DARK, CHECKER_LIGHT, CHECKER_SIZE, SELECTION_SIZE, THUMB_RADIUS, TRACK_SIZE,
    alpha_stops, hue_stops, plane_stops,
};
use shrimply_components_core::color::{Hsva, PALETTE, RECENT_LIMIT, color_hex};
use shrimply_components_core::math::{ColorSelection, color_selection, color_selection_key};
use shrimply_math_color::{Color, parse_css_color};

use super::color_swatch::{ColorSwatch, SwatchShape};

const WINDOW_WIDTH: i32 = 960;
const WINDOW_HEIGHT: i32 = 480;
const NARROW_WIDTH: i32 = 760;
const SMALL_WIDTH: i32 = 430;
const SMALL_SELECTION_SIZE: i32 = 200;
const CONTENT_MARGIN: i32 = 24;

const COLOR_CHOOSER_SETTINGS: &str = "org.gtk.gtk4.Settings.ColorChooser";
const CUSTOM_COLORS_KEY: &str = "custom-colors";

thread_local! {
    static RECENT_COLORS: RefCell<Vec<Color<u8>>> = RefCell::new(load_recent_colors());
}

pub struct ColorPicker;

impl ColorPicker {
    pub fn builder(color: Color<u8>) -> ColorPickerBuilder {
        ColorPickerBuilder {
            color,
            title: "Select color".to_string(),
            with_alpha: true,
            hexpand: false,
            on_change: None,
        }
    }
}

pub struct ColorPickerBuilder {
    color: Color<u8>,
    title: String,
    with_alpha: bool,
    hexpand: bool,
    on_change: Option<Box<dyn Fn(Color<u8>) + 'static>>,
}

impl ColorPickerBuilder {
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    pub fn with_alpha(mut self, with_alpha: bool) -> Self {
        self.with_alpha = with_alpha;
        if !with_alpha {
            self.color.a = u8::MAX;
        }
        self
    }

    pub fn hexpand(mut self, hexpand: bool) -> Self {
        self.hexpand = hexpand;
        self
    }

    pub fn on_change(mut self, callback: impl Fn(Color<u8>) + 'static) -> Self {
        self.on_change = Some(Box::new(callback));
        self
    }

    pub fn build(self) -> gtk::Widget {
        self.build_with_handle().widget
    }

    pub fn build_with_handle(self) -> ColorPickerParts {
        let ColorPickerBuilder {
            color: initial,
            title,
            with_alpha,
            hexpand,
            on_change,
        } = self;
        let title = crate::i18n::text(&title).into_owned();
        let color = Rc::new(Cell::new(initial));
        let sample = color_sample(color.clone(), 22, 22);
        let hex = gtk::Label::new(Some(&color_hex(initial, with_alpha)));
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.append(&sample);
        row.append(&hex);
        let narrow_width = if with_alpha { 104 } else { 88 };
        let adaptive = adw::BreakpointBin::builder()
            .child(&row)
            .width_request(narrow_width)
            .height_request(22)
            .build();
        let narrow = adw::Breakpoint::new(
            adw::BreakpointCondition::parse(&format!("max-width: {narrow_width}px"))
                .expect("valid color picker breakpoint"),
        );
        narrow.add_setter(&hex, "visible", Some(&false.to_value()));
        adaptive.add_breakpoint(narrow);
        let button = gtk::Button::builder()
            .child(&adaptive)
            .tooltip_text(&title)
            .hexpand(hexpand)
            .valign(gtk::Align::Center)
            .build();
        let callback: Rc<dyn Fn(Color<u8>)> =
            on_change.map_or_else(|| Rc::new(|_| {}) as Rc<dyn Fn(Color<u8>)>, Rc::from);
        button.connect_clicked({
            let color = color.clone();
            let sample = sample.clone();
            let hex = hex.clone();
            move |button| {
                show_window(
                    button,
                    &title,
                    color.clone(),
                    sample.clone(),
                    hex.clone(),
                    with_alpha,
                    callback.clone(),
                );
            }
        });
        ColorPickerParts {
            widget: button.upcast(),
            handle: ColorPickerHandle {
                color,
                sample,
                hex,
                with_alpha,
            },
        }
    }
}

pub struct ColorPickerParts {
    pub widget: gtk::Widget,
    pub handle: ColorPickerHandle,
}

#[derive(Clone)]
pub struct ColorPickerHandle {
    color: Rc<Cell<Color<u8>>>,
    sample: gtk::DrawingArea,
    hex: gtk::Label,
    with_alpha: bool,
}

#[derive(Clone)]
pub struct WeakColorPickerHandle {
    color: Rc<Cell<Color<u8>>>,
    sample: glib::WeakRef<gtk::DrawingArea>,
    hex: glib::WeakRef<gtk::Label>,
    with_alpha: bool,
}

impl ColorPickerHandle {
    pub fn downgrade(&self) -> WeakColorPickerHandle {
        WeakColorPickerHandle {
            color: self.color.clone(),
            sample: self.sample.downgrade(),
            hex: self.hex.downgrade(),
            with_alpha: self.with_alpha,
        }
    }
}

impl WeakColorPickerHandle {
    pub fn set_color(&self, mut color: Color<u8>) -> bool {
        if !self.with_alpha {
            color.a = u8::MAX;
        }
        if self.color.replace(color) == color {
            return self.sample.upgrade().is_some() && self.hex.upgrade().is_some();
        }
        let Some(sample) = self.sample.upgrade() else {
            return false;
        };
        let Some(hex) = self.hex.upgrade() else {
            return false;
        };
        hex.set_text(&color_hex(color, self.with_alpha));
        sample.queue_draw();
        true
    }
}

fn show_window(
    parent: &gtk::Button,
    title: &str,
    selected: Rc<Cell<Color<u8>>>,
    button_sample: gtk::DrawingArea,
    button_hex: gtk::Label,
    with_alpha: bool,
    on_change: Rc<dyn Fn(Color<u8>)>,
) {
    let builder = adw::Window::builder()
        .title(title)
        .default_width(WINDOW_WIDTH)
        .default_height(WINDOW_HEIGHT)
        .destroy_with_parent(true)
        .modal(false);
    let window = match parent.root().and_downcast::<gtk::Window>() {
        Some(parent) => builder.transient_for(&parent).build(),
        None => builder.build(),
    };
    let draft = Rc::new(Cell::new(Hsva::from_color(selected.get())));
    let redraw = Rc::new(RefCell::new(Vec::<gtk::DrawingArea>::new()));
    let swatches = Rc::new(RefCell::new(Vec::<ColorSwatch>::new()));
    let updating = Rc::new(Cell::new(false));
    let entry = gtk::Entry::builder()
        .hexpand(true)
        .max_width_chars(9)
        .tooltip_text(crate::i18n::text("Hexadecimal color").as_ref())
        .build();
    entry.set_text(&color_hex(selected.get(), with_alpha));

    let preview_dialog = gtk::ColorDialog::builder()
        .title(title)
        .with_alpha(with_alpha)
        .build();
    let preview = gtk::ColorDialogButton::new(Some(preview_dialog));
    preview.set_rgba(&selected.get().into());
    preview.set_tooltip_text(Some(
        crate::i18n::text("Open the GTK color picker").as_ref(),
    ));
    preview.set_valign(gtk::Align::Center);

    let update_hsva: Rc<dyn Fn(Hsva)> = {
        let draft = draft.clone();
        let redraw = redraw.clone();
        let entry = entry.clone();
        let preview = preview.clone();
        let updating = updating.clone();
        let swatches = swatches.clone();
        Rc::new(move |mut value| {
            if !with_alpha {
                value.alpha = 1.0;
            }
            if draft.replace(value) == value {
                return;
            }
            updating.set(true);
            let color = value.color();
            entry.set_text(&color_hex(color, with_alpha));
            preview.set_rgba(&color.into());
            updating.set(false);
            entry.remove_css_class("error");
            for area in redraw.borrow().iter() {
                area.queue_draw();
            }
            for swatch in swatches.borrow().iter() {
                swatch.set_selected(swatch.color() == color);
            }
        })
    };
    let select_color: Rc<dyn Fn(Color<u8>)> = {
        let draft = draft.clone();
        let update_hsva = update_hsva.clone();
        Rc::new(move |color| update_hsva(draft.get().update_color(color)))
    };

    preview.connect_rgba_notify({
        let select_color = select_color.clone();
        let updating = updating.clone();
        move |preview| {
            if !updating.get() {
                select_color(preview.rgba().into());
            }
        }
    });
    connect_hex_entry(&entry, select_color.clone(), updating.clone(), with_alpha);

    let value_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    value_row.append(&preview);
    value_row.append(&entry);

    let hue = hue_bar(draft.clone(), update_hsva.clone());
    let plane = color_plane(draft.clone(), update_hsva.clone());
    redraw.borrow_mut().extend([hue.clone(), plane.clone()]);
    let selection = gtk::Grid::builder()
        .row_spacing(10)
        .column_spacing(10)
        .valign(gtk::Align::Start)
        .build();
    selection.attach(&value_row, 0, 0, 2, 1);
    selection.attach(&hue, 0, 1, 1, 1);
    selection.attach(&plane, 1, 1, 1, 1);
    if with_alpha {
        let alpha = alpha_bar(draft.clone(), update_hsva);
        redraw.borrow_mut().push(alpha.clone());
        selection.attach(&alpha, 1, 2, 1, 1);
    }

    let palette = palette_grid(draft.get().color(), select_color.clone(), swatches.clone());
    let palette_column = gtk::Box::new(gtk::Orientation::Vertical, 6);
    palette_column.set_hexpand(true);
    palette_column.set_vexpand(true);
    palette_column.append(&palette);
    palette_column.append(&recent_row(
        draft.get().color(),
        select_color.clone(),
        swatches,
    ));
    palette_column.append(&gtk::Box::builder().vexpand(true).build());

    let main = gtk::Box::new(gtk::Orientation::Horizontal, 24);
    main.set_hexpand(true);
    main.set_vexpand(true);
    main.append(&selection);
    let separator = gtk::Separator::new(gtk::Orientation::Vertical);
    main.append(&separator);
    main.append(&palette_column);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.set_margin_top(18);
    content.set_margin_bottom(18);
    content.set_margin_start(CONTENT_MARGIN);
    content.set_margin_end(CONTENT_MARGIN);
    content.append(&main);
    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&content)
        .build();
    let adaptive = adw::BreakpointBin::builder()
        .child(&scroller)
        .width_request(SMALL_SELECTION_SIZE + BAR_SIZE + 10 + CONTENT_MARGIN * 2)
        .height_request(BAR_SIZE)
        .vexpand(true)
        .build();
    let narrow = adw::Breakpoint::new(
        adw::BreakpointCondition::parse(&format!(
            "min-width: {}px and max-width: {NARROW_WIDTH}px",
            SMALL_WIDTH + 1
        ))
        .expect("valid color picker breakpoint"),
    );
    narrow.add_setter(
        &main,
        "orientation",
        Some(&gtk::Orientation::Vertical.to_value()),
    );
    narrow.add_setter(
        &separator,
        "orientation",
        Some(&gtk::Orientation::Horizontal.to_value()),
    );
    adaptive.add_breakpoint(narrow);
    let small = adw::Breakpoint::new(
        adw::BreakpointCondition::parse(&format!("max-width: {SMALL_WIDTH}px"))
            .expect("valid small color picker breakpoint"),
    );
    small.add_setter(
        &main,
        "orientation",
        Some(&gtk::Orientation::Vertical.to_value()),
    );
    small.add_setter(
        &separator,
        "orientation",
        Some(&gtk::Orientation::Horizontal.to_value()),
    );
    small.add_setter(
        &plane,
        "content-width",
        Some(&SMALL_SELECTION_SIZE.to_value()),
    );
    small.add_setter(
        &plane,
        "content-height",
        Some(&SMALL_SELECTION_SIZE.to_value()),
    );
    small.add_setter(
        &hue,
        "content-height",
        Some(&SMALL_SELECTION_SIZE.to_value()),
    );
    if let Some(alpha) = selection.child_at(1, 2) {
        small.add_setter(
            &alpha,
            "content-width",
            Some(&SMALL_SELECTION_SIZE.to_value()),
        );
    }
    adaptive.add_breakpoint(small);

    let confirm = gtk::Button::builder()
        .icon_name("object-select-symbolic")
        .tooltip_text(crate::i18n::text("Confirm color").as_ref())
        .halign(gtk::Align::End)
        .css_classes(["suggested-action", "circular"])
        .build();
    let picker = screen_picker_button(select_color);
    confirm.connect_clicked({
        let window = window.clone();
        let draft = draft.clone();
        move |_| {
            let color = draft.get().color();
            if selected.replace(color) != color {
                button_sample.queue_draw();
                button_hex.set_label(&color_hex(color, with_alpha));
                on_change(color);
            }
            remember_color(color);
            window.close();
        }
    });
    palette_column.append(&confirm);

    let header = adw::HeaderBar::new();
    header.pack_start(&picker);
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&adaptive));
    window.set_content(Some(&toolbar));
    window.present();
}

fn color_plane(color: Rc<Cell<Hsva>>, update: Rc<dyn Fn(Hsva)>) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::builder()
        .content_width(SELECTION_SIZE)
        .content_height(SELECTION_SIZE)
        .halign(gtk::Align::Start)
        .valign(gtk::Align::Start)
        .focusable(true)
        .tooltip_text(crate::i18n::text("Saturation and value").as_ref())
        .build();
    area.set_draw_func({
        let color = color.clone();
        move |_, context, width, height| {
            let value = color.get();
            for (axis, colors) in plane_stops(value.hue).into_iter().enumerate() {
                let gradient = if axis == 0 {
                    cairo::LinearGradient::new(0.0, 0.0, 0.0, f64::from((height - 1).max(1)))
                } else {
                    cairo::LinearGradient::new(0.0, 0.0, f64::from((width - 1).max(1)), 0.0)
                };
                for (index, color) in colors.into_iter().enumerate() {
                    gradient.add_color_stop_rgba(
                        index as f64,
                        channel(color.r),
                        channel(color.g),
                        channel(color.b),
                        channel(color.a),
                    );
                }
                context.set_source(&gradient).expect("set SV gradient");
                context.paint().expect("draw SV gradient");
            }
            draw_crosshair(
                context,
                f64::from(value.value) * f64::from(width - 1),
                (1.0 - f64::from(value.saturation)) * f64::from(height - 1),
                width,
                height,
            );
        }
    });
    area.set_cursor_from_name(Some("crosshair"));
    connect_keys(&area, color.clone(), update.clone(), ColorSelection::Plane);
    connect_drag(&area, move |x, y, width, height| {
        update(color_selection(
            color.get(),
            ColorSelection::Plane,
            [unit(x, width - 1.0), unit(y, height - 1.0)],
        ));
    });
    area
}

fn hue_bar(color: Rc<Cell<Hsva>>, update: Rc<dyn Fn(Hsva)>) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::builder()
        .content_width(BAR_SIZE)
        .content_height(SELECTION_SIZE)
        .valign(gtk::Align::Start)
        .focusable(true)
        .tooltip_text(crate::i18n::text("Hue").as_ref())
        .build();
    area.set_draw_func({
        let color = color.clone();
        move |_, context, width, height| {
            let start = THUMB_RADIUS;
            let end = f64::from(height) - THUMB_RADIUS;
            let gradient = cairo::LinearGradient::new(0.0, start, 0.0, end);
            for (position, color) in hue_stops() {
                gradient.add_color_stop_rgb(
                    position,
                    channel(color.r),
                    channel(color.g),
                    channel(color.b),
                );
            }
            context.save().expect("save hue clip");
            capsule_path(
                context,
                (f64::from(width) - TRACK_SIZE) / 2.0,
                start,
                TRACK_SIZE,
                end - start,
            );
            context.clip();
            context.set_source(&gradient).expect("set hue gradient");
            context.paint().expect("draw hue gradient");
            context.restore().expect("restore hue clip");
            draw_thumb(
                context,
                f64::from(width) / 2.0,
                start + f64::from(color.get().hue / 360.0) * (end - start),
            );
        }
    });
    connect_keys(&area, color.clone(), update.clone(), ColorSelection::Hue);
    connect_drag(&area, move |_, y, _, height| {
        update(color_selection(
            color.get(),
            ColorSelection::Hue,
            [unit(y - THUMB_RADIUS, height - THUMB_RADIUS * 2.0), 0.0],
        ));
    });
    area
}

fn alpha_bar(color: Rc<Cell<Hsva>>, update: Rc<dyn Fn(Hsva)>) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::builder()
        .content_width(SELECTION_SIZE)
        .content_height(BAR_SIZE)
        .halign(gtk::Align::Start)
        .focusable(true)
        .tooltip_text(crate::i18n::text("Alpha").as_ref())
        .build();
    area.set_draw_func({
        let color = color.clone();
        move |_, context, width, height| {
            let start = THUMB_RADIUS;
            let end = f64::from(width) - THUMB_RADIUS;
            context.save().expect("save alpha clip");
            capsule_path(
                context,
                start,
                (f64::from(height) - TRACK_SIZE) / 2.0,
                end - start,
                TRACK_SIZE,
            );
            context.clip();
            draw_checkerboard(context, width, height);
            let gradient = cairo::LinearGradient::new(start, 0.0, end, 0.0);
            for (index, color) in alpha_stops(color.get().color()).into_iter().enumerate() {
                gradient.add_color_stop_rgba(
                    index as f64,
                    channel(color.r),
                    channel(color.g),
                    channel(color.b),
                    channel(color.a),
                );
            }
            context.set_source(&gradient).expect("set alpha gradient");
            context.paint().expect("draw alpha gradient");
            context.restore().expect("restore alpha clip");
            draw_thumb(
                context,
                start + f64::from(color.get().alpha) * (end - start),
                f64::from(height) / 2.0,
            );
        }
    });
    connect_keys(&area, color.clone(), update.clone(), ColorSelection::Alpha);
    connect_drag(&area, move |x, _, width, _| {
        update(color_selection(
            color.get(),
            ColorSelection::Alpha,
            [unit(x - THUMB_RADIUS, width - THUMB_RADIUS * 2.0), 0.0],
        ));
    });
    area
}

fn connect_keys(
    area: &gtk::DrawingArea,
    color: Rc<Cell<Hsva>>,
    update: Rc<dyn Fn(Hsva)>,
    control: ColorSelection,
) {
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(move |_, key, _, _| {
        let key = match key {
            gtk::gdk::Key::Left => "left",
            gtk::gdk::Key::Right => "right",
            gtk::gdk::Key::Up => "up",
            gtk::gdk::Key::Down => "down",
            _ => return glib::Propagation::Proceed,
        };
        if let Some(value) = color_selection_key(color.get(), control, key) {
            update(value);
        }
        glib::Propagation::Stop
    });
    area.add_controller(keys);
}

fn connect_drag(area: &gtk::DrawingArea, update: impl Fn(f64, f64, f64, f64) + 'static) {
    let update = Rc::new(update);
    let start = Rc::new(Cell::new((0.0, 0.0)));
    let drag = gtk::GestureDrag::new();
    drag.connect_drag_begin({
        let area = area.clone();
        let start = start.clone();
        let update = update.clone();
        move |_, x, y| {
            start.set((x, y));
            update(x, y, f64::from(area.width()), f64::from(area.height()));
        }
    });
    drag.connect_drag_update({
        let area = area.clone();
        move |_, dx, dy| {
            let (x, y) = start.get();
            update(
                x + dx,
                y + dy,
                f64::from(area.width()),
                f64::from(area.height()),
            );
        }
    });
    area.add_controller(drag);
}

fn palette_grid(
    selected: Color<u8>,
    update: Rc<dyn Fn(Color<u8>)>,
    swatches: Rc<RefCell<Vec<ColorSwatch>>>,
) -> gtk::FlowBox {
    let grid = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .min_children_per_line(1)
        .max_children_per_line((PALETTE.len() / 5) as u32)
        .row_spacing(4)
        .column_spacing(4)
        .halign(gtk::Align::Fill)
        .valign(gtk::Align::Start)
        .build();
    for colors in PALETTE.chunks(5) {
        let column = gtk::Box::new(gtk::Orientation::Vertical, 2);
        for (position, &(name, color)) in colors.iter().enumerate() {
            let shape = match position {
                0 => SwatchShape::PaletteTop,
                4 => SwatchShape::PaletteBottom,
                _ => SwatchShape::PaletteMiddle,
            };
            let sample = ColorSwatch::new(color, shape, selected == color, {
                let update = update.clone();
                move || update(color)
            });
            sample.set_tooltip_text(Some(crate::i18n::text(name).as_ref()));
            swatches.borrow_mut().push(sample.clone());
            column.append(&sample);
        }
        grid.insert(&column, -1);
    }
    grid
}

fn recent_row(
    selected: Color<u8>,
    update: Rc<dyn Fn(Color<u8>)>,
    swatches: Rc<RefCell<Vec<ColorSwatch>>>,
) -> gtk::Box {
    let row = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .min_children_per_line(1)
        .max_children_per_line(RECENT_LIMIT as u32)
        .row_spacing(6)
        .column_spacing(6)
        .build();
    for color in RECENT_COLORS.with(|colors| colors.borrow().clone()) {
        let sample = ColorSwatch::new(color, SwatchShape::Rounded, selected == color, {
            let update = update.clone();
            move || update(color)
        });
        swatches.borrow_mut().push(sample.clone());
        row.insert(&sample, -1);
    }
    let section = gtk::Box::new(gtk::Orientation::Vertical, 6);
    section.append(
        &gtk::Label::builder()
            .label(crate::i18n::text("Recent").as_ref())
            .halign(gtk::Align::Start)
            .build(),
    );
    section.append(&row);
    section
}

fn screen_picker_button(update: Rc<dyn Fn(Color<u8>)>) -> gtk::Button {
    let picker = gtk::Button::builder()
        .icon_name("color-select-symbolic")
        .tooltip_text(crate::i18n::text("Pick a color from the screen").as_ref())
        .css_classes(["circular"])
        .build();
    picker.connect_clicked(move |picker| {
        picker.set_sensitive(false);
        let picker = picker.clone();
        let update = update.clone();
        glib::MainContext::default().spawn_local(async move {
            match shrimply_cross_ui_core::screen_color::pick().await {
                Ok([red, green, blue]) => update(Color::from_srgba([
                    red as f32,
                    green as f32,
                    blue as f32,
                    1.0,
                ])),
                Err(error) => tracing::warn!("screen color picker failed: {error}"),
            }
            picker.set_sensitive(true);
        });
    });
    picker
}

fn color_sample(color: Rc<Cell<Color<u8>>>, width: i32, height: i32) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::builder()
        .content_width(width)
        .content_height(height)
        .build();
    area.set_draw_func(move |_, context, width, height| {
        let radius = f64::from(width.min(height)) / 2.0;
        context.arc(
            f64::from(width) / 2.0,
            f64::from(height) / 2.0,
            radius,
            0.0,
            std::f64::consts::TAU,
        );
        context.clip();
        let color = color.get();
        if color.a < u8::MAX {
            draw_checkerboard(context, width, height);
        }
        context.set_source_rgba(
            channel(color.r),
            channel(color.g),
            channel(color.b),
            channel(color.a),
        );
        context.paint().expect("draw color swatch");
    });
    area
}

fn connect_hex_entry(
    entry: &gtk::Entry,
    update: Rc<dyn Fn(Color<u8>)>,
    updating: Rc<Cell<bool>>,
    with_alpha: bool,
) {
    let apply: Rc<dyn Fn()> = {
        let entry = entry.clone();
        let updating = updating.clone();
        Rc::new(move || {
            if updating.get() {
                return;
            }
            match parse_css_color(entry.text().trim()) {
                Some(mut color) => {
                    if !with_alpha {
                        color.a = u8::MAX;
                    }
                    entry.remove_css_class("error");
                    update(color);
                }
                None => entry.add_css_class("error"),
            }
        })
    };
    entry.connect_activate({
        let apply = apply.clone();
        move |_| apply()
    });
    let focus = gtk::EventControllerFocus::new();
    focus.connect_leave(move |_| apply());
    entry.add_controller(focus);
    entry.connect_changed(move |entry| {
        if !updating.get() {
            entry.remove_css_class("error");
        }
    });
}

fn remember_color(color: Color<u8>) {
    RECENT_COLORS.with(|colors| {
        let mut colors = colors.borrow_mut();
        shrimply_components_core::color::remember_color(&mut colors, color);
        let values = colors
            .iter()
            .map(|color| {
                (
                    channel(color.r),
                    channel(color.g),
                    channel(color.b),
                    channel(color.a),
                )
            })
            .collect::<Vec<_>>()
            .to_variant();
        if let Err(error) =
            gio::Settings::new(COLOR_CHOOSER_SETTINGS).set_value(CUSTOM_COLORS_KEY, &values)
        {
            tracing::warn!("could not save recent colors: {error}");
        }
    });
}

fn load_recent_colors() -> Vec<Color<u8>> {
    gio::Settings::new(COLOR_CHOOSER_SETTINGS)
        .value(CUSTOM_COLORS_KEY)
        .iter()
        .filter_map(|value| value.get::<(f64, f64, f64, f64)>())
        .map(|(red, green, blue, alpha)| {
            Color::from_srgba([red as f32, green as f32, blue as f32, alpha as f32])
        })
        .take(RECENT_LIMIT)
        .collect()
}

fn draw_checkerboard(context: &cairo::Context, width: i32, height: i32) {
    context.set_source_rgb(CHECKER_DARK, CHECKER_DARK, CHECKER_DARK);
    context.paint().expect("draw checkerboard base");
    context.set_source_rgb(CHECKER_LIGHT, CHECKER_LIGHT, CHECKER_LIGHT);
    let columns = (f64::from(width) / CHECKER_SIZE).ceil() as i32;
    let rows = (f64::from(height) / CHECKER_SIZE).ceil() as i32;
    for row in 0..rows {
        for column in 0..columns {
            if (row + column) % 2 == 0 {
                context.rectangle(
                    f64::from(column) * CHECKER_SIZE,
                    f64::from(row) * CHECKER_SIZE,
                    CHECKER_SIZE,
                    CHECKER_SIZE,
                );
            }
        }
    }
    context.fill().expect("draw checkerboard squares");
}

fn draw_crosshair(context: &cairo::Context, x: f64, y: f64, width: i32, height: i32) {
    let x = x.clamp(0.0, f64::from(width));
    let y = y.clamp(0.0, f64::from(height));
    context.set_source_rgb(1.0, 1.0, 1.0);
    context.rectangle(0.0, y - 0.5, f64::from(width), 1.0);
    context.rectangle(x - 0.5, 0.0, 1.0, f64::from(height));
    context.fill().expect("draw crosshair");
}

fn draw_thumb(context: &cairo::Context, x: f64, y: f64) {
    context.arc(x, y + 1.0, THUMB_RADIUS, 0.0, std::f64::consts::TAU);
    context.set_source_rgba(0.0, 0.0, 0.0, 0.35);
    context.fill().expect("draw slider shadow");
    context.arc(x, y, THUMB_RADIUS, 0.0, std::f64::consts::TAU);
    context.set_source_rgb(0.92, 0.92, 0.92);
    context.fill_preserve().expect("draw slider thumb");
    context.set_line_width(1.0);
    context.set_source_rgba(0.0, 0.0, 0.0, 0.35);
    context.stroke().expect("draw slider border");
}

fn capsule_path(context: &cairo::Context, x: f64, y: f64, width: f64, height: f64) {
    let radius = width.min(height) / 2.0;
    context.new_sub_path();
    context.move_to(x + radius, y);
    context.line_to(x + width - radius, y);
    context.arc(
        x + width - radius,
        y + radius,
        radius,
        -std::f64::consts::FRAC_PI_2,
        0.0,
    );
    context.line_to(x + width, y + height - radius);
    context.arc(
        x + width - radius,
        y + height - radius,
        radius,
        0.0,
        std::f64::consts::FRAC_PI_2,
    );
    context.line_to(x + radius, y + height);
    context.arc(
        x + radius,
        y + height - radius,
        radius,
        std::f64::consts::FRAC_PI_2,
        std::f64::consts::PI,
    );
    context.line_to(x, y + radius);
    context.arc(
        x + radius,
        y + radius,
        radius,
        std::f64::consts::PI,
        std::f64::consts::PI * 1.5,
    );
    context.close_path();
}

fn unit(value: f64, extent: f64) -> f32 {
    (value / extent.max(1.0)).clamp(0.0, 1.0) as f32
}

fn channel(value: u8) -> f64 {
    f64::from(value) / 255.0
}
