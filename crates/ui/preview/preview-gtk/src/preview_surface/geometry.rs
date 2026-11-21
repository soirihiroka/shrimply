use super::*;
use glib::translate::{ToGlibPtrMut, from_glib};

pub(super) fn theme_window_color(area: &gtk::GLArea) -> Color {
    unsafe {
        let context =
            gtk::ffi::gtk_widget_get_style_context(area.as_ptr() as *mut gtk::ffi::GtkWidget);
        let mut color = gdk::RGBA::TRANSPARENT;
        let found: bool = from_glib(gtk::ffi::gtk_style_context_lookup_color(
            context,
            c"window_bg_color".as_ptr(),
            color.to_glib_none_mut().0,
        ));
        assert!(found, "Adwaita theme does not define window_bg_color");
        color.into()
    }
}
