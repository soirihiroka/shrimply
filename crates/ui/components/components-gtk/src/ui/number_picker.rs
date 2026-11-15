use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::prelude::*;
use gtk::{gdk, glib};
use num_traits::ToPrimitive;
use num_traits::{NumCast, PrimInt};
use shrimply_components_core::number::{
    DEFAULT_DRAG_PIXELS, DEFAULT_MAXIMUM, DEFAULT_MINIMUM, NumberConfig as NumberPickerConfig,
    NumberDrag, accepted_value, finite_fraction_or, format_value, parse_fraction,
    positive_fraction_or,
};
use shrimply_math_core::Fraction;
use shrimply_math_core::{
    FRACTION_ZERO, fraction_as_f64, fraction_from_f64, fraction_from_integer,
};

use super::pointer_lock::PointerLock;

const DISPLAY_PAGE: &str = "display";
const ENTRY_PAGE: &str = "entry";
const SLOW_NUMBER_PICKER_LOG_THRESHOLD: Duration = Duration::from_millis(2);

pub fn number_field_row(
    field: &shrimply_components_core::presentation::NumberField,
    value: Fraction,
) -> adw::SpinRow {
    use adw::prelude::*;
    let numeric =
        |text: &str| fraction_as_f64(text.parse().expect("shared number field constraint"));
    let row = adw::SpinRow::with_range(
        numeric(field.minimum),
        numeric(field.maximum),
        numeric(field.step),
    );
    row.set_title(crate::i18n::text(field.label).as_ref());
    row.set_value(fraction_as_f64(value));
    row.set_digits(field.digits as u32);
    row
}

thread_local! {
    static ROTATING_ICON_CSS_INSTALLED: Cell<bool> = const { Cell::new(false) };
}

pub struct NumberPicker;

impl NumberPicker {
    pub fn builder(value: f64) -> NumberPickerBuilder {
        Self::fraction_builder(fraction_from_f64(value))
    }

    pub fn integer_builder<T>(value: T) -> NumberPickerBuilder
    where
        T: PrimInt + ToPrimitive,
    {
        Self::fraction_builder(fraction_from_f64(
            value
                .to_f64()
                .expect("number picker integer must be representable as f64"),
        ))
        .digits(0)
        .drag_step(1.0)
    }

    pub fn fraction_builder(value: Fraction) -> NumberPickerBuilder {
        NumberPickerBuilder {
            value,
            minimum: fraction_from_integer(DEFAULT_MINIMUM),
            maximum: fraction_from_integer(DEFAULT_MAXIMUM),
            drag_step: fraction_from_integer(1),
            drag_pixels: DEFAULT_DRAG_PIXELS,
            digits: 2,
            prefix: None,
            prefix_icon_name: None,
            prefix_icon_rotates: false,
            prefix_icon_rotation_offset_degrees: 0.0,
            suffix: None,
            unit_name: None,
            width_chars: 8,
            on_change: None,
            on_commit: None,
        }
    }
}

pub struct NumberPickerBuilder {
    value: Fraction,
    minimum: Fraction,
    maximum: Fraction,
    drag_step: Fraction,
    drag_pixels: f64,
    digits: usize,
    prefix: Option<String>,
    prefix_icon_name: Option<String>,
    prefix_icon_rotates: bool,
    prefix_icon_rotation_offset_degrees: f64,
    suffix: Option<String>,
    unit_name: Option<String>,
    width_chars: i32,
    on_change: Option<Box<dyn Fn(Fraction) + 'static>>,
    on_commit: Option<Box<dyn Fn(Fraction) + 'static>>,
}

impl NumberPickerBuilder {
    pub fn accepted_range(mut self, minimum: f64, maximum: f64) -> Self {
        self.minimum = fraction_from_f64(minimum);
        self.maximum = fraction_from_f64(maximum);
        self
    }

    pub fn minimum(mut self, value: f64) -> Self {
        self.minimum = fraction_from_f64(value);
        self
    }

    pub fn maximum(mut self, value: f64) -> Self {
        self.maximum = fraction_from_f64(value);
        self
    }

    pub fn drag_step(mut self, value: f64) -> Self {
        self.drag_step = positive_fraction_or(fraction_from_f64(value), self.drag_step);
        self
    }

    pub fn digits(mut self, value: usize) -> Self {
        self.digits = value;
        self
    }

    pub fn prefix(mut self, value: impl Into<String>) -> Self {
        self.prefix = Some(value.into());
        self
    }

    pub fn rotating_prefix_icon_name(mut self, value: impl Into<String>) -> Self {
        self.prefix_icon_name = Some(value.into());
        self.prefix_icon_rotates = true;
        self
    }

    pub fn rotating_prefix_icon_name_with_offset(
        mut self,
        value: impl Into<String>,
        offset_degrees: f64,
    ) -> Self {
        self.prefix_icon_name = Some(value.into());
        self.prefix_icon_rotates = true;
        self.prefix_icon_rotation_offset_degrees = offset_degrees;
        self
    }

    pub fn unit_name(mut self, value: impl Into<String>) -> Self {
        self.unit_name = Some(value.into());
        self
    }

    pub fn width_chars(mut self, value: i32) -> Self {
        if value > 0 {
            self.width_chars = value;
        }
        self
    }

    pub fn on_change(mut self, callback: impl Fn(f64) + 'static) -> Self {
        self.on_change = Some(Box::new(move |value| callback(fraction_as_f64(value))));
        self
    }

    pub fn on_change_fraction(mut self, callback: impl Fn(Fraction) + 'static) -> Self {
        self.on_change = Some(Box::new(callback));
        self
    }

    pub fn on_change_integer<T>(mut self, callback: impl Fn(T) + 'static) -> Self
    where
        T: PrimInt + NumCast,
    {
        self.on_change = Some(Box::new(move |value| {
            callback(integer_from_fraction(value))
        }));
        self
    }

    pub fn on_commit(mut self, callback: impl Fn(f64) + 'static) -> Self {
        self.on_commit = Some(Box::new(move |value| callback(fraction_as_f64(value))));
        self
    }

    pub fn on_commit_integer<T>(mut self, callback: impl Fn(T) + 'static) -> Self
    where
        T: PrimInt + NumCast,
    {
        self.on_commit = Some(Box::new(move |value| {
            callback(integer_from_fraction(value))
        }));
        self
    }

    pub fn on_commit_fraction(mut self, callback: impl Fn(Fraction) + 'static) -> Self {
        self.on_commit = Some(Box::new(callback));
        self
    }

    pub fn build(self) -> gtk::Widget {
        self.build_with_handle().widget
    }

    pub fn build_with_handle(self) -> NumberPickerParts {
        self.build_parts()
    }

    fn build_parts(self) -> NumberPickerParts {
        let NumberPickerBuilder {
            value,
            minimum,
            maximum,
            drag_step,
            drag_pixels,
            digits,
            prefix,
            prefix_icon_name,
            prefix_icon_rotates,
            prefix_icon_rotation_offset_degrees,
            suffix,
            unit_name,
            width_chars,
            on_change,
            on_commit,
        } = self;
        let config = Rc::new(NumberPickerConfig {
            minimum,
            maximum,
            drag_step,
            drag_pixels,
            digits,
            fallback: finite_fraction_or(value, FRACTION_ZERO),
        });
        let value = Rc::new(Cell::new(accepted_value(&config, value)));
        let on_change: Rc<dyn Fn(Fraction)> = match on_change {
            Some(callback) => Rc::from(callback),
            None => Rc::new(|_| {}),
        };
        let on_commit: Rc<dyn Fn(Fraction)> = match on_commit {
            Some(callback) => Rc::from(callback),
            None => Rc::new(|_| {}),
        };

        let stack = gtk::Stack::new();
        stack.set_hexpand(true);
        stack.set_hhomogeneous(true);
        stack.set_vhomogeneous(true);
        stack.set_transition_type(gtk::StackTransitionType::Crossfade);

        let display = gtk::Button::new();
        display.set_hexpand(true);
        display.set_width_request(width_chars * 12);
        display.set_tooltip_text(Some(
            crate::i18n::text("Click to type, drag horizontally to adjust").as_ref(),
        ));
        display.set_cursor_from_name(Some("ew-resize"));

        let display_content = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        display_content.set_hexpand(true);

        let mut rotating_icon = None;
        if let Some(icon_name) = prefix_icon_name
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            let icon = gtk::Image::from_icon_name(icon_name);
            icon.add_css_class("dim-label");
            if prefix_icon_rotates {
                install_rotating_icon_css(&icon.display());
                rotating_icon = Some(RotatingIcon {
                    icon: icon.clone(),
                    degrees: Rc::new(Cell::new(0)),
                    offset_degrees: prefix_icon_rotation_offset_degrees,
                });
            }
            display_content.append(&icon);
        }

        if let Some(prefix) = prefix.as_deref().filter(|value| !value.is_empty()) {
            let prefix = gtk::Label::new(Some(prefix));
            prefix.add_css_class("dim-label");
            display_content.append(&prefix);
        }

        let value_label = gtk::Label::new(None);
        value_label.set_hexpand(true);
        value_label.set_xalign(1.0);
        value_label.add_css_class("numeric");
        value_label.set_text(&format_value(&config, value.get()));
        update_rotating_icon(rotating_icon.as_ref(), value.get());
        display_content.append(&value_label);

        let suffix = suffix
            .as_deref()
            .filter(|value| !value.is_empty())
            .or_else(|| unit_name.as_deref().filter(|value| !value.is_empty()));
        if let Some(suffix) = suffix {
            let suffix = gtk::Label::new(Some(suffix));
            suffix.add_css_class("dim-label");
            display_content.append(&suffix);
        }

        display.set_child(Some(&display_content));

        let entry = gtk::Entry::new();
        entry.set_hexpand(true);
        entry.set_width_chars(width_chars);
        entry.set_input_purpose(gtk::InputPurpose::Number);

        stack.add_named(&display, Some(DISPLAY_PAGE));
        stack.add_named(&entry, Some(ENTRY_PAGE));
        stack.set_visible_child_name(DISPLAY_PAGE);

        let outside_click = Rc::new(RefCell::new(None));

        let drag_state = Rc::new(Cell::new(NumberDrag::begin(value.get())));
        let pointer_lock_attempted = Rc::new(Cell::new(false));
        let pointer_lock = Rc::new(RefCell::new(None));
        let drag = gtk::GestureDrag::new();
        {
            let value = value.clone();
            let drag_state = drag_state.clone();
            let pointer_lock_attempted = pointer_lock_attempted.clone();
            drag.connect_drag_begin(move |_, _, _| {
                tracing::trace!("number_picker: drag_begin");
                drag_state.set(NumberDrag::begin(value.get()));
                pointer_lock_attempted.set(false);
            });
        }
        {
            let stack = stack.clone();
            let display = display.clone();
            let display_content = display_content.clone();
            let value = value.clone();
            let value_label = value_label.clone();
            let rotating_icon = rotating_icon.clone();
            let config = config.clone();
            let on_change = on_change.clone();
            let drag_state = drag_state.clone();
            let pointer_lock_attempted = pointer_lock_attempted.clone();
            let pointer_lock = pointer_lock.clone();
            drag.connect_drag_update(move |_, offset_x, _| {
                tracing::trace!(
                    "number_picker: drag_update offset_x={offset_x:.3} moved={} lock={} attempted={}",
                    drag_state.get().moved(),
                    pointer_lock.borrow().is_some(),
                    pointer_lock_attempted.get()
                );
                if pointer_lock.borrow().is_some() {
                    return;
                }

                let mut drag_value = drag_state.get();
                if !drag_value.update_absolute(offset_x) {
                    return;
                }
                drag_state.set(drag_value);
                apply_drag_offset(
                    &value,
                    &value_label,
                    rotating_icon.as_ref(),
                    &config,
                    on_change.as_ref(),
                    drag_value,
                );
                if !pointer_lock_attempted.replace(true) {
                    let value = value.clone();
                    let value_label = value_label.clone();
                    let rotating_icon = rotating_icon.clone();
                    let config = config.clone();
                    let on_change = on_change.clone();
                    let drag_state = drag_state.clone();
                    *pointer_lock.borrow_mut() = PointerLock::new(&display, move |offset_x| {
                        let mut drag_value = drag_state.get();
                        if !drag_value.update_relative(offset_x) {
                            return;
                        }
                        drag_state.set(drag_value);
                        apply_drag_offset(
                            &value,
                            &value_label,
                            rotating_icon.as_ref(),
                            &config,
                            on_change.as_ref(),
                            drag_value,
                        );
                    });
                    set_display_cursor(&stack, &display, &display_content, Some("none"));
                }
            });
        }
        {
            let stack = stack.clone();
            let entry = entry.clone();
            let display = display.clone();
            let display_content = display_content.clone();
            let value = value.clone();
            let value_label = value_label.clone();
            let rotating_icon = rotating_icon.clone();
            let config = config.clone();
            let on_change = on_change.clone();
            let on_commit = on_commit.clone();
            let outside_click = outside_click.clone();
            let drag_state = drag_state.clone();
            let pointer_lock = pointer_lock.clone();
            drag.connect_drag_end(move |_, offset_x, _| {
                tracing::trace!(
                    "number_picker: drag_end offset_x={offset_x:.3} moved={} lock={}",
                    drag_state.get().moved(),
                    pointer_lock.borrow().is_some()
                );
                if drag_state.get().is_click(offset_x) {
                    begin_edit(&stack, &entry, value.get(), &config);
                    schedule_outside_click(
                        stack.clone(),
                        entry.clone(),
                        value.clone(),
                        value_label.clone(),
                        rotating_icon.clone(),
                        config.clone(),
                        on_change.clone(),
                        on_commit.clone(),
                        outside_click.clone(),
                    );
                } else if drag_state.get().moved() {
                    on_commit(value.get());
                }
                release_pointer_lock(&pointer_lock);
                set_display_cursor(&stack, &display, &display_content, Some("ew-resize"));
            });
        }
        display.add_controller(drag);

        {
            let stack = stack.clone();
            let config = config.clone();
            let on_change = on_change.clone();
            let preview_value = Rc::new(Cell::new(None));
            entry.connect_changed(move |entry| {
                if stack.visible_child_name().as_deref() != Some(ENTRY_PAGE) {
                    preview_value.set(None);
                    return;
                }
                if let Some(next) = parse_fraction(entry.text().trim()) {
                    let next = accepted_value(&config, next);
                    if preview_value.replace(Some(next)) != Some(next) {
                        on_change(next);
                    }
                }
            });
        }

        {
            let stack = stack.clone();
            let value = value.clone();
            let value_label = value_label.clone();
            let rotating_icon = rotating_icon.clone();
            let config = config.clone();
            let on_change = on_change.clone();
            let on_commit = on_commit.clone();
            let outside_click = outside_click.clone();
            entry.connect_activate(move |entry| {
                tracing::trace!("number_picker: entry_activate");
                commit_entry(
                    "activate",
                    &stack,
                    entry,
                    &value,
                    &value_label,
                    rotating_icon.as_ref(),
                    &config,
                    on_change.as_ref(),
                    on_commit.as_ref(),
                    &outside_click,
                );
            });
        }

        let focus = gtk::EventControllerFocus::new();
        {
            let stack = stack.clone();
            let entry = entry.clone();
            let value = value.clone();
            let value_label = value_label.clone();
            let rotating_icon = rotating_icon.clone();
            let config = config.clone();
            let on_change = on_change.clone();
            let on_commit = on_commit.clone();
            let outside_click = outside_click.clone();
            focus.connect_leave(move |_| {
                tracing::trace!("number_picker: focus_leave");
                commit_entry(
                    "focus_leave",
                    &stack,
                    &entry,
                    &value,
                    &value_label,
                    rotating_icon.as_ref(),
                    &config,
                    on_change.as_ref(),
                    on_commit.as_ref(),
                    &outside_click,
                );
            });
        }
        entry.add_controller(focus);

        NumberPickerParts {
            widget: stack.clone().upcast(),
            handle: NumberPickerHandle {
                value,
                stack,
                label: value_label,
                rotating_icon,
                config,
            },
        }
    }
}

pub struct NumberPickerParts {
    pub widget: gtk::Widget,
    pub handle: NumberPickerHandle,
}

#[derive(Clone)]
pub struct NumberPickerHandle {
    value: Rc<Cell<Fraction>>,
    stack: gtk::Stack,
    label: gtk::Label,
    rotating_icon: Option<RotatingIcon>,
    config: Rc<NumberPickerConfig>,
}

#[derive(Clone)]
pub struct WeakNumberPickerHandle {
    value: Rc<Cell<Fraction>>,
    stack: glib::WeakRef<gtk::Stack>,
    label: glib::WeakRef<gtk::Label>,
    rotating_icon: Option<RotatingIcon>,
    config: Rc<NumberPickerConfig>,
}

#[derive(Clone)]
struct RotatingIcon {
    icon: gtk::Image,
    degrees: Rc<Cell<u16>>,
    offset_degrees: f64,
}

impl NumberPickerHandle {
    pub fn downgrade(&self) -> WeakNumberPickerHandle {
        WeakNumberPickerHandle {
            value: self.value.clone(),
            stack: self.stack.downgrade(),
            label: self.label.downgrade(),
            rotating_icon: self.rotating_icon.clone(),
            config: self.config.clone(),
        }
    }

    pub fn set_f64(&self, value: f64) {
        set_display_handle_value(self, fraction_from_f64(value));
    }

    pub fn widget_is_mapped(&self) -> bool {
        self.stack.is_mapped()
    }
}

impl WeakNumberPickerHandle {
    pub fn upgrade(&self) -> Option<NumberPickerHandle> {
        Some(NumberPickerHandle {
            value: self.value.clone(),
            stack: self.stack.upgrade()?,
            label: self.label.upgrade()?,
            rotating_icon: self.rotating_icon.clone(),
            config: self.config.clone(),
        })
    }
}

pub struct Number2Picker;

impl Number2Picker {
    pub fn builder(first: f64, second: f64) -> Number2PickerBuilder {
        Self::fraction_builder(fraction_from_f64(first), fraction_from_f64(second))
    }

    pub fn fraction_builder(first: Fraction, second: Fraction) -> Number2PickerBuilder {
        Number2PickerBuilder {
            first: NumberPicker::fraction_builder(first),
            second: NumberPicker::fraction_builder(second),
            enable_lock: false,
            on_change: None,
            on_first_change: None,
            on_second_change: None,
            on_first_commit: None,
            on_second_commit: None,
        }
    }
}

pub struct Number2PickerBuilder {
    first: NumberPickerBuilder,
    second: NumberPickerBuilder,
    enable_lock: bool,
    on_change: Number2GroupCallback,
    on_first_change: Option<Box<dyn Fn(Fraction) + 'static>>,
    on_second_change: Option<Box<dyn Fn(Fraction) + 'static>>,
    on_first_commit: Option<Box<dyn Fn(Fraction) + 'static>>,
    on_second_commit: Option<Box<dyn Fn(Fraction) + 'static>>,
}

type Number2GroupCallback = Option<Box<dyn Fn([Fraction; 2], usize) + 'static>>;

impl Number2PickerBuilder {
    pub fn minimum(mut self, value: f64) -> Self {
        self.first = self.first.minimum(value);
        self.second = self.second.minimum(value);
        self
    }

    pub fn maximum(mut self, value: f64) -> Self {
        self.first = self.first.maximum(value);
        self.second = self.second.maximum(value);
        self
    }

    pub fn drag_step(mut self, value: f64) -> Self {
        self.first = self.first.drag_step(value);
        self.second = self.second.drag_step(value);
        self
    }

    pub fn digits(mut self, value: usize) -> Self {
        self.first = self.first.digits(value);
        self.second = self.second.digits(value);
        self
    }

    pub fn width_chars(mut self, value: i32) -> Self {
        self.first = self.first.width_chars(value);
        self.second = self.second.width_chars(value);
        self
    }

    pub fn first_prefix(mut self, value: impl Into<String>) -> Self {
        self.first = self.first.prefix(value);
        self
    }

    pub fn second_prefix(mut self, value: impl Into<String>) -> Self {
        self.second = self.second.prefix(value);
        self
    }

    pub fn unit_name(mut self, value: impl Into<String>) -> Self {
        let value = value.into();
        self.first = self.first.unit_name(value.clone());
        self.second = self.second.unit_name(value);
        self
    }

    pub fn enable_lock(mut self) -> Self {
        self.enable_lock = true;
        self
    }

    pub fn on_change(mut self, callback: impl Fn([f64; 2], usize) + 'static) -> Self {
        self.on_change = Some(Box::new(move |values, component| {
            callback(values.map(fraction_as_f64), component);
        }));
        self
    }

    pub fn on_first_change(mut self, callback: impl Fn(f64) + 'static) -> Self {
        self.on_first_change = Some(Box::new(move |value| callback(fraction_as_f64(value))));
        self
    }

    pub fn on_second_change(mut self, callback: impl Fn(f64) + 'static) -> Self {
        self.on_second_change = Some(Box::new(move |value| callback(fraction_as_f64(value))));
        self
    }

    pub fn on_first_commit(mut self, callback: impl Fn(f64) + 'static) -> Self {
        self.on_first_commit = Some(Box::new(move |value| callback(fraction_as_f64(value))));
        self
    }

    pub fn on_second_commit(mut self, callback: impl Fn(f64) + 'static) -> Self {
        self.on_second_commit = Some(Box::new(move |value| callback(fraction_as_f64(value))));
        self
    }

    pub fn build_with_handles(self) -> Number2PickerParts {
        let Number2PickerBuilder {
            first,
            second,
            enable_lock,
            on_change,
            on_first_change,
            on_second_change,
            on_first_commit,
            on_second_commit,
        } = self;
        let initial_first = first.value;
        let initial_second = second.value;
        let first_handle = Rc::new(RefCell::new(None::<NumberPickerHandle>));
        let second_handle = Rc::new(RefCell::new(None::<NumberPickerHandle>));
        let on_change: Rc<dyn Fn([Fraction; 2], usize)> = match on_change {
            Some(callback) => Rc::from(callback),
            None => Rc::new(|_, _| {}),
        };
        let on_first_change: Rc<dyn Fn(Fraction)> = match on_first_change {
            Some(callback) => Rc::from(callback),
            None => Rc::new(|_| {}),
        };
        let on_second_change: Rc<dyn Fn(Fraction)> = match on_second_change {
            Some(callback) => Rc::from(callback),
            None => Rc::new(|_| {}),
        };
        let on_first_commit: Rc<dyn Fn(Fraction)> = match on_first_commit {
            Some(callback) => Rc::from(callback),
            None => Rc::new(|_| {}),
        };
        let on_second_commit: Rc<dyn Fn(Fraction)> = match on_second_commit {
            Some(callback) => Rc::from(callback),
            None => Rc::new(|_| {}),
        };
        if !enable_lock {
            let first = first
                .on_change_fraction({
                    let second_handle = second_handle.clone();
                    let on_change = on_change.clone();
                    let on_first_change = on_first_change.clone();
                    move |value| {
                        on_first_change(value);
                        let second = second_handle
                            .borrow()
                            .as_ref()
                            .map_or(initial_second, |handle| handle.value.get());
                        on_change([value, second], 0);
                    }
                })
                .on_commit_fraction({
                    let on_first_commit = on_first_commit.clone();
                    move |value| on_first_commit(value)
                });
            let second = second
                .on_change_fraction({
                    let first_handle = first_handle.clone();
                    let on_change = on_change.clone();
                    let on_second_change = on_second_change.clone();
                    move |value| {
                        on_second_change(value);
                        let first = first_handle
                            .borrow()
                            .as_ref()
                            .map_or(initial_first, |handle| handle.value.get());
                        on_change([first, value], 1);
                    }
                })
                .on_commit_fraction({
                    let on_second_commit = on_second_commit.clone();
                    move |value| on_second_commit(value)
                });
            let first = first.build_parts();
            *first_handle.borrow_mut() = Some(first.handle.clone());
            let second = second.build_parts();
            *second_handle.borrow_mut() = Some(second.handle.clone());
            return Number2PickerParts {
                widget: build_number_pair_row(first.widget, second.widget, None),
                first: first.handle,
                second: second.handle,
            };
        }

        let initial_ratio = shrimply_components_core::number::pair_ratio(first.value, second.value);
        let locked = Rc::new(Cell::new(true));
        let locked_ratio = Rc::new(Cell::new(initial_ratio));

        let first = first
            .on_change_fraction({
                let locked = locked.clone();
                let locked_ratio = locked_ratio.clone();
                let second_handle = second_handle.clone();
                let on_change = on_change.clone();
                let on_first_change = on_first_change.clone();
                let on_second_change = on_second_change.clone();
                move |value| {
                    let primary_started = Instant::now();
                    on_first_change(value);
                    let primary_elapsed = primary_started.elapsed();
                    if locked.get() {
                        let ratio = locked_ratio.get();
                        let next = shrimply_components_core::number::pair_second(value, ratio);
                        if let Some(handle) = second_handle.borrow().as_ref() {
                            let cascade_started = Instant::now();
                            let next = set_handle_value(handle, next);
                            on_second_change(next);
                            let cascade_elapsed = cascade_started.elapsed();
                            if primary_elapsed >= SLOW_NUMBER_PICKER_LOG_THRESHOLD
                                || cascade_elapsed >= SLOW_NUMBER_PICKER_LOG_THRESHOLD
                            {
                                tracing::debug!(
                                    "number2_picker: lock_cascade source=first value={:.6} cascaded={:.6} ratio={:.6} primary_on_change_elapsed_us={} cascade_elapsed_us={}",
                                    fraction_as_f64(value),
                                    fraction_as_f64(next),
                                    fraction_as_f64(ratio),
                                    primary_elapsed.as_micros(),
                                    cascade_elapsed.as_micros(),
                                );
                            }
                        }
                    }
                    let second = second_handle
                        .borrow()
                        .as_ref()
                        .map_or(initial_second, |handle| handle.value.get());
                    on_change([value, second], 0);
                }
            })
            .on_commit_fraction({
                let on_first_commit = on_first_commit.clone();
                move |value| on_first_commit(value)
            });
        let second = second
            .on_change_fraction({
                let locked = locked.clone();
                let locked_ratio = locked_ratio.clone();
                let first_handle = first_handle.clone();
                let on_change = on_change.clone();
                let on_first_change = on_first_change.clone();
                let on_second_change = on_second_change.clone();
                move |value| {
                    let primary_started = Instant::now();
                    on_second_change(value);
                    let primary_elapsed = primary_started.elapsed();
                    if locked.get() {
                        let next = value * locked_ratio.get();
                        if let Some(handle) = first_handle.borrow().as_ref() {
                            let cascade_started = Instant::now();
                            let next = set_handle_value(handle, next);
                            on_first_change(next);
                            let cascade_elapsed = cascade_started.elapsed();
                            if primary_elapsed >= SLOW_NUMBER_PICKER_LOG_THRESHOLD
                                || cascade_elapsed >= SLOW_NUMBER_PICKER_LOG_THRESHOLD
                            {
                                tracing::debug!(
                                    "number2_picker: lock_cascade source=second value={:.6} cascaded={:.6} ratio={:.6} primary_on_change_elapsed_us={} cascade_elapsed_us={}",
                                    fraction_as_f64(value),
                                    fraction_as_f64(next),
                                    fraction_as_f64(locked_ratio.get()),
                                    primary_elapsed.as_micros(),
                                    cascade_elapsed.as_micros(),
                                );
                            }
                        }
                    }
                    let first = first_handle
                        .borrow()
                        .as_ref()
                        .map_or(initial_first, |handle| handle.value.get());
                    on_change([first, value], 1);
                }
            })
            .on_commit_fraction({
                let on_second_commit = on_second_commit.clone();
                move |value| on_second_commit(value)
            });

        let first = first.build_parts();
        *first_handle.borrow_mut() = Some(first.handle.clone());
        let second = second.build_parts();
        *second_handle.borrow_mut() = Some(second.handle.clone());
        let output_first = first.handle.clone();
        let output_second = second.handle.clone();

        let lock = gtk::ToggleButton::new();
        lock.set_icon_name("padlock2-symbolic");
        lock.set_tooltip_text(Some(crate::i18n::text("Lock ratio").as_ref()));
        lock.set_valign(gtk::Align::Center);
        lock.set_active(true);
        lock.add_css_class("flat");
        {
            let locked = locked.clone();
            let locked_ratio = locked_ratio.clone();
            let first_handle = first_handle.clone();
            let second_handle = second_handle.clone();
            lock.connect_toggled(move |button| {
                let active = button.is_active();
                locked.set(active);
                button.set_icon_name(if active {
                    "padlock2-symbolic"
                } else {
                    "padlock2-open-symbolic"
                });
                if !active {
                    return;
                }
                let Some(first) = first_handle.borrow().clone() else {
                    return;
                };
                let Some(second) = second_handle.borrow().clone() else {
                    return;
                };
                locked_ratio.set(shrimply_components_core::number::pair_ratio(
                    first.value.get(),
                    second.value.get(),
                ));
            });
        }

        Number2PickerParts {
            widget: build_number_pair_row(first.widget, second.widget, Some(lock.upcast())),
            first: output_first,
            second: output_second,
        }
    }
}

pub struct Number2PickerParts {
    pub widget: gtk::Widget,
    pub first: NumberPickerHandle,
    pub second: NumberPickerHandle,
}

pub struct Number3Picker;

impl Number3Picker {
    pub fn builder(first: f64, second: f64, third: f64) -> Number3PickerBuilder {
        Number3PickerBuilder {
            values: [
                NumberPicker::builder(first),
                NumberPicker::builder(second),
                NumberPicker::builder(third),
            ],
            enable_lock: false,
            on_change: [None, None, None],
            on_commit: [None, None, None],
        }
    }
}

type Number3Callback = Option<Box<dyn Fn(Fraction) + 'static>>;

pub struct Number3PickerBuilder {
    values: [NumberPickerBuilder; 3],
    enable_lock: bool,
    on_change: [Number3Callback; 3],
    on_commit: [Number3Callback; 3],
}

impl Number3PickerBuilder {
    pub fn minimum(mut self, value: f64) -> Self {
        self.values = self.values.map(|picker| picker.minimum(value));
        self
    }

    pub fn drag_step(mut self, value: f64) -> Self {
        self.values = self.values.map(|picker| picker.drag_step(value));
        self
    }

    pub fn digits(mut self, value: usize) -> Self {
        self.values = self.values.map(|picker| picker.digits(value));
        self
    }

    pub fn width_chars(mut self, value: i32) -> Self {
        self.values = self.values.map(|picker| picker.width_chars(value));
        self
    }

    pub fn prefixes(mut self, values: [&str; 3]) -> Self {
        for (picker, prefix) in self.values.iter_mut().zip(values) {
            *picker = std::mem::replace(picker, NumberPicker::builder(0.0)).prefix(prefix);
        }
        self
    }

    pub fn unit_name(mut self, value: impl Into<String>) -> Self {
        let value = value.into();
        self.values = self.values.map(|picker| picker.unit_name(value.clone()));
        self
    }

    pub fn enable_lock(mut self) -> Self {
        self.enable_lock = true;
        self
    }

    pub fn on_change(mut self, axis: usize, callback: impl Fn(f64) + 'static) -> Self {
        self.on_change[axis] = Some(Box::new(move |value| callback(fraction_as_f64(value))));
        self
    }

    pub fn on_commit(mut self, axis: usize, callback: impl Fn(f64) + 'static) -> Self {
        self.on_commit[axis] = Some(Box::new(move |value| callback(fraction_as_f64(value))));
        self
    }

    pub fn build_with_handles(self) -> Number3PickerParts {
        let callbacks: [Rc<dyn Fn(Fraction)>; 3] = self
            .on_change
            .map(|callback| callback.map_or_else(|| Rc::new(|_| {}) as Rc<_>, Rc::from));
        let commits: [Rc<dyn Fn(Fraction)>; 3] = self
            .on_commit
            .map(|callback| callback.map_or_else(|| Rc::new(|_| {}) as Rc<_>, Rc::from));
        let locked = Rc::new(Cell::new(self.enable_lock));
        let ratios = Rc::new(Cell::new(shrimply_components_core::number::triple_ratios(
            [
                self.values[0].value,
                self.values[1].value,
                self.values[2].value,
            ],
        )));
        let handles = Rc::new(RefCell::new([None, None, None]));
        let mut parts = Vec::with_capacity(3);
        for (axis, picker) in self.values.into_iter().enumerate() {
            let callbacks = callbacks.clone();
            let change_handles = handles.clone();
            let locked = locked.clone();
            let ratios = ratios.clone();
            let commit = commits[axis].clone();
            let part = picker
                .on_change_fraction(move |value| {
                    callbacks[axis](value);
                    if !locked.get() {
                        return;
                    }
                    let next =
                        shrimply_components_core::number::locked_triple(axis, value, ratios.get());
                    for other in 0..3 {
                        if other == axis {
                            continue;
                        }
                        let Some(handle) = change_handles.borrow()[other].clone() else {
                            continue;
                        };
                        let next = set_handle_value(&handle, next[other]);
                        callbacks[other](next);
                    }
                })
                .on_commit_fraction(move |value| commit(value))
                .build_parts();
            handles.borrow_mut()[axis] = Some(part.handle.clone());
            parts.push(part);
        }

        let lock = self.enable_lock.then(|| {
            let button = gtk::ToggleButton::new();
            button.set_icon_name("padlock2-symbolic");
            button.set_tooltip_text(Some(crate::i18n::text("Lock ratio").as_ref()));
            button.set_valign(gtk::Align::Center);
            button.set_active(true);
            button.add_css_class("flat");
            let locked = locked.clone();
            let ratios = ratios.clone();
            let handles = handles.clone();
            button.connect_toggled(move |button| {
                locked.set(button.is_active());
                button.set_icon_name(if button.is_active() {
                    "padlock2-symbolic"
                } else {
                    "padlock2-open-symbolic"
                });
                if button.is_active() {
                    ratios.set(number3_handle_ratios(&handles.borrow()));
                }
            });
            button.upcast()
        });

        let [first, second, third] = parts
            .try_into()
            .unwrap_or_else(|_| unreachable!("three number pickers were built"));
        Number3PickerParts {
            widget: build_number_row([first.widget, second.widget, third.widget], lock),
            handles: [first.handle, second.handle, third.handle],
        }
    }
}

pub struct Number3PickerParts {
    pub widget: gtk::Widget,
    pub handles: [NumberPickerHandle; 3],
}

fn number3_handle_ratios(handles: &[Option<NumberPickerHandle>; 3]) -> [Fraction; 2] {
    let Some(first) = handles[0].as_ref() else {
        return [fraction_from_integer(1); 2];
    };
    let Some(second) = handles[1].as_ref() else {
        return [fraction_from_integer(1); 2];
    };
    let Some(third) = handles[2].as_ref() else {
        return [fraction_from_integer(1); 2];
    };
    shrimply_components_core::number::triple_ratios([
        first.value.get(),
        second.value.get(),
        third.value.get(),
    ])
}

fn build_number_row<const N: usize>(
    values: [gtk::Widget; N],
    prefix: Option<gtk::Widget>,
) -> gtk::Widget {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.set_hexpand(true);
    if let Some(prefix) = prefix {
        row.append(&prefix);
    }
    for value in values {
        value.set_hexpand(true);
        row.append(&value);
    }
    row.upcast()
}

fn build_number_pair_row(
    first: gtk::Widget,
    second: gtk::Widget,
    lock: Option<gtk::Widget>,
) -> gtk::Widget {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.set_hexpand(true);

    first.set_hexpand(true);
    second.set_hexpand(true);
    if let Some(lock) = lock {
        row.append(&lock);
    }
    row.append(&first);
    row.append(&second);

    row.upcast()
}

fn set_handle_value(handle: &NumberPickerHandle, value: Fraction) -> Fraction {
    let value = accepted_value(&handle.config, value);
    handle.value.set(value);
    handle.label.set_text(&format_value(&handle.config, value));
    update_rotating_icon(handle.rotating_icon.as_ref(), value);
    value
}

fn update_rotating_icon(icon: Option<&RotatingIcon>, value: Fraction) {
    let Some(icon) = icon else {
        return;
    };
    let degrees = shrimply_components_core::math::number_rotation(
        fraction_as_f64(value),
        icon.offset_degrees,
    );
    let previous = icon.degrees.replace(degrees);
    icon.icon
        .remove_css_class(&format!("number-picker-rotation-{previous}"));
    icon.icon
        .add_css_class(&format!("number-picker-rotation-{degrees}"));
}

fn install_rotating_icon_css(display: &gdk::Display) {
    ROTATING_ICON_CSS_INSTALLED.with(|installed| {
        if installed.replace(true) {
            return;
        }
        let mut css = String::new();
        for degrees in 0..360 {
            css.push_str(&format!(
                ".number-picker-rotation-{degrees} {{ -gtk-icon-transform: rotate({degrees}deg); }}"
            ));
        }
        let provider = gtk::CssProvider::new();
        provider.load_from_string(&css);
        gtk::style_context_add_provider_for_display(
            display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    });
}

fn set_display_handle_value(handle: &NumberPickerHandle, value: Fraction) -> Fraction {
    if handle.stack.visible_child_name().as_deref() == Some(ENTRY_PAGE) {
        return handle.value.get();
    }
    set_handle_value(handle, value)
}

fn begin_edit(
    stack: &gtk::Stack,
    entry: &gtk::Entry,
    value: Fraction,
    config: &NumberPickerConfig,
) {
    tracing::trace!(
        "number_picker: begin_edit visible={:?} has_focus={}",
        stack.visible_child_name(),
        entry.has_focus()
    );
    entry.set_text(&format_value(config, value));
    stack.set_visible_child_name(ENTRY_PAGE);
    let focus_entry = entry.clone();
    glib::idle_add_local_once(move || {
        focus_entry.grab_focus();
        focus_entry.select_region(0, -1);
        tracing::trace!(
            "number_picker: begin_edit_focus visible_entry={} has_focus={}",
            gtk::prelude::WidgetExt::is_visible(&focus_entry),
            focus_entry.has_focus()
        );
    });
    tracing::trace!(
        "number_picker: begin_edit_done visible={:?} has_focus={}",
        stack.visible_child_name(),
        entry.has_focus()
    );
}

#[allow(clippy::too_many_arguments)]
fn commit_entry(
    reason: &str,
    stack: &gtk::Stack,
    entry: &gtk::Entry,
    value: &Cell<Fraction>,
    label: &gtk::Label,
    rotating_icon: Option<&RotatingIcon>,
    config: &NumberPickerConfig,
    on_change: &dyn Fn(Fraction),
    on_commit: &dyn Fn(Fraction),
    outside_click: &Rc<RefCell<Option<OutsideClickController>>>,
) {
    tracing::trace!(
        "number_picker: commit_entry reason={reason} visible={:?} has_focus={} text={:?}",
        stack.visible_child_name(),
        entry.has_focus(),
        entry.text()
    );
    if stack.visible_child_name().as_deref() != Some(ENTRY_PAGE) {
        tracing::trace!("number_picker: commit_entry_ignored reason={reason}");
        return;
    }

    if let Some(next) = parse_fraction(entry.text().trim())
        && set_value(value, label, rotating_icon, config, on_change, next)
    {
        on_commit(value.get());
    }
    entry.set_text(&format_value(config, value.get()));
    stack.set_visible_child_name(DISPLAY_PAGE);
    remove_outside_click(outside_click);
    tracing::trace!(
        "number_picker: commit_entry_done reason={reason} visible={:?}",
        stack.visible_child_name()
    );
}

fn set_display_cursor(
    stack: &gtk::Stack,
    display: &gtk::Button,
    display_content: &gtk::Box,
    name: Option<&str>,
) {
    stack.set_cursor_from_name(name);
    display.set_cursor_from_name(name);
    display_content.set_cursor_from_name(name);
    let mut child = display_content.first_child();
    while let Some(widget) = child {
        widget.set_cursor_from_name(name);
        child = widget.next_sibling();
    }
}

struct OutsideClickController {
    root: gtk::Widget,
    controller: gtk::EventControllerLegacy,
}

#[allow(clippy::too_many_arguments)]
fn schedule_outside_click(
    stack: gtk::Stack,
    entry: gtk::Entry,
    value: Rc<Cell<Fraction>>,
    label: gtk::Label,
    rotating_icon: Option<RotatingIcon>,
    config: Rc<NumberPickerConfig>,
    on_change: Rc<dyn Fn(Fraction)>,
    on_commit: Rc<dyn Fn(Fraction)>,
    outside_click: Rc<RefCell<Option<OutsideClickController>>>,
) {
    tracing::trace!("number_picker: schedule_outside_click");
    glib::idle_add_local_once(move || {
        tracing::trace!("number_picker: schedule_outside_click_idle");
        install_outside_click(
            &stack,
            &entry,
            &value,
            &label,
            rotating_icon,
            &config,
            on_change,
            on_commit,
            &outside_click,
        );
    });
}

#[allow(clippy::too_many_arguments)]
fn install_outside_click(
    stack: &gtk::Stack,
    entry: &gtk::Entry,
    value: &Rc<Cell<Fraction>>,
    label: &gtk::Label,
    rotating_icon: Option<RotatingIcon>,
    config: &Rc<NumberPickerConfig>,
    on_change: Rc<dyn Fn(Fraction)>,
    on_commit: Rc<dyn Fn(Fraction)>,
    outside_click: &Rc<RefCell<Option<OutsideClickController>>>,
) {
    if outside_click.borrow().is_some() {
        tracing::trace!("number_picker: install_outside_click skipped existing");
        return;
    }

    let Some(root) = stack.root().map(|root| root.upcast::<gtk::Widget>()) else {
        tracing::trace!("number_picker: install_outside_click skipped no_root");
        return;
    };

    tracing::trace!("number_picker: install_outside_click");
    let controller = gtk::EventControllerLegacy::new();
    controller.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let stack = stack.clone();
        let entry = entry.clone();
        let value = value.clone();
        let label = label.clone();
        let rotating_icon = rotating_icon.clone();
        let config = config.clone();
        let on_commit = on_commit.clone();
        let outside_click = outside_click.clone();
        let root = root.clone();
        controller.connect_event(move |_, event| {
            if !matches!(
                event.event_type(),
                gdk::EventType::ButtonPress | gdk::EventType::TouchBegin
            ) {
                return glib::Propagation::Proceed;
            }

            tracing::trace!(
                "number_picker: outside_event type={:?} position={:?}",
                event.event_type(),
                event.position()
            );
            if event_is_outside_widget(event, &stack, &root) {
                tracing::trace!("number_picker: outside_event_commit");
                commit_entry(
                    "outside_click",
                    &stack,
                    &entry,
                    &value,
                    &label,
                    rotating_icon.as_ref(),
                    &config,
                    on_change.as_ref(),
                    on_commit.as_ref(),
                    &outside_click,
                );
            }
            glib::Propagation::Proceed
        });
    }

    root.add_controller(controller.clone());
    *outside_click.borrow_mut() = Some(OutsideClickController { root, controller });
}

fn event_is_outside_widget(
    event: &gdk::Event,
    widget: &impl IsA<gtk::Widget>,
    root: &gtk::Widget,
) -> bool {
    let Some((x, y)) = event.position() else {
        return false;
    };
    let Some(bounds) = widget.compute_bounds(root) else {
        return false;
    };

    let x = x as f32;
    let y = y as f32;
    x < bounds.x()
        || y < bounds.y()
        || x >= bounds.x() + bounds.width()
        || y >= bounds.y() + bounds.height()
}

fn remove_outside_click(outside_click: &Rc<RefCell<Option<OutsideClickController>>>) {
    let Some(outside_click) = outside_click.borrow_mut().take() else {
        tracing::trace!("number_picker: remove_outside_click skipped none");
        return;
    };
    tracing::trace!("number_picker: remove_outside_click");
    outside_click
        .root
        .remove_controller(&outside_click.controller);
}

fn set_value(
    value: &Cell<Fraction>,
    label: &gtk::Label,
    rotating_icon: Option<&RotatingIcon>,
    config: &NumberPickerConfig,
    on_change: &dyn Fn(Fraction),
    next: Fraction,
) -> bool {
    let started = Instant::now();
    let next = accepted_value(config, next);
    let previous = value.replace(next);
    label.set_text(&format_value(config, next));
    update_rotating_icon(rotating_icon, next);
    if previous != next {
        let change_started = Instant::now();
        on_change(next);
        let change_elapsed = change_started.elapsed();
        let elapsed = started.elapsed();
        if change_elapsed >= SLOW_NUMBER_PICKER_LOG_THRESHOLD
            || elapsed >= SLOW_NUMBER_PICKER_LOG_THRESHOLD
        {
            tracing::debug!(
                "number_picker: value_changed previous={:.6} next={:.6} on_change_elapsed_us={} total_elapsed_us={}",
                fraction_as_f64(previous),
                fraction_as_f64(next),
                change_elapsed.as_micros(),
                elapsed.as_micros(),
            );
        }
        return true;
    }
    false
}

fn apply_drag_offset(
    value: &Cell<Fraction>,
    label: &gtk::Label,
    rotating_icon: Option<&RotatingIcon>,
    config: &NumberPickerConfig,
    on_change: &dyn Fn(Fraction),
    drag: NumberDrag,
) {
    let started = Instant::now();
    let changed = set_value(
        value,
        label,
        rotating_icon,
        config,
        on_change,
        drag.value(config),
    );
    let elapsed = started.elapsed();
    if elapsed >= SLOW_NUMBER_PICKER_LOG_THRESHOLD {
        tracing::debug!(
            "number_picker: apply_drag_offset changed={changed} elapsed_us={}",
            elapsed.as_micros(),
        );
    }
}

fn release_pointer_lock(lock: &Rc<RefCell<Option<PointerLock>>>) {
    lock.borrow_mut().take();
}

fn integer_from_fraction<T>(value: Fraction) -> T
where
    T: PrimInt + NumCast,
{
    NumCast::from(fraction_as_f64(value).round())
        .expect("accepted number picker value must fit the requested integer type")
}
