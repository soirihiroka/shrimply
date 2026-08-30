mod color_picker;
mod color_swatch;
mod control_row;
mod i18n;
mod keyed_box;
mod multiline_text_input;
mod number_picker;
#[cfg(target_os = "linux")]
mod pointer_lock;
#[cfg(not(target_os = "linux"))]
mod pointer_lock {
    //! Non-Wayland stub: pointer locking during numeric scrubbing is a no-op.
    use gtk::prelude::IsA;

    pub struct PointerLock {}

    impl PointerLock {
        pub fn new(
            _widget: &impl IsA<gtk::Widget>,
            _on_delta: impl Fn(f64) + 'static,
        ) -> Option<Self> {
            None
        }

        pub fn new_2d(
            _widget: &impl IsA<gtk::Widget>,
            _on_delta: impl Fn(f64, f64) + 'static,
        ) -> Option<Self> {
            None
        }

        pub fn restore_cursor_at(&self, _x: f64, _y: f64) {}
    }
}
mod progress_button;
mod selector;
mod single_line_text_input;
mod split_button;
mod switch_row;

pub use color_picker::{ColorPicker, ColorPickerBuilder};
pub use control_row::control_row;
pub use i18n::{I18nAlertDialogExt, I18nFileFilterExt, I18nMenuExt, I18nWidgetExt, menu_item_i18n};
pub use keyed_box::KeyedBox;
pub use multiline_text_input::{MultilineTextInput, MultilineTextInputBuilder};
pub use number_picker::{Number2Picker, Number3Picker, NumberPicker, NumberPickerHandle};
pub use pointer_lock::PointerLock;
pub use progress_button::{ProgressButton, ProgressButtonState};
pub use selector::{
    StringChoice, StringSelector, dropdown, enum_dropdown, enum_selector, labeled_string_selector,
    selector, string_selector,
};
pub use single_line_text_input::{SingleLineTextInput, SingleLineTextInputBuilder};
pub use split_button::split_button;
pub use switch_row::switch_row;
