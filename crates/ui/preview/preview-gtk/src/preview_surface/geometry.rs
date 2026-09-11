use super::*;
use glib::translate::{ToGlibPtrMut, from_glib};
use shrimply_preview_provider_skia::PreviewViewport;

pub(super) fn surface_viewport(
    area: &gtk::GLArea,
    project: &Project,
    state: &VideoSurfaceState,
) -> PreviewViewport {
    state.navigation.viewport(
        fit_viewport(area, project, state),
        surface_bounds(area, state),
    )
}

pub(super) fn fit_viewport(
    area: &gtk::GLArea,
    project: &Project,
    state: &VideoSurfaceState,
) -> PreviewViewport {
    guides::viewport(
        glam::IVec2::new(area.width(), area.height()),
        project.canvas_size,
        state.preview_padding_px,
        state.guides_visible,
        state.fullscreen,
    )
}

pub(super) fn surface_bounds(area: &gtk::GLArea, state: &VideoSurfaceState) -> Rect {
    guides::bounds(
        glam::vec2(area.width() as f32, area.height() as f32),
        state.preview_padding_px,
        state.guides_visible,
        state.fullscreen,
    )
}

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
