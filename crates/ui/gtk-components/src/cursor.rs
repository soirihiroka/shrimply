use crate::canvas::vec2;
use gtk::gdk;
use gtk::gdk::prelude::*;
use shrimply_skia_adw_core::cursor::SoftwareCursor;

pub use shrimply_skia_adw_core::cursor::{PlayheadStyle, draw_playhead};

pub const DEFAULT_CURSOR_THEME_SIZE: i32 = 24;

pub fn software_cursor_from_name(name: &str, display: &gdk::Display) -> SoftwareCursor {
    let (name, hot_spot) = match name {
        "crosshair" => ("crosshair", vec2(15.0, 15.0)),
        "e-resize" => ("e-resize", vec2(25.0, 17.0)),
        "w-resize" => ("w-resize", vec2(8.0, 17.0)),
        "ew-resize" => ("ew-resize", vec2(16.0, 15.0)),
        "grabbing" => ("grabbing", vec2(15.0, 14.0)),
        _ => ("default", vec2(5.0, 5.0)),
    };
    let texture = gdk::Texture::from_resource(&format!("/org/gtk/libgdk/cursor/{name}"));
    let width = texture.width();
    let height = texture.height();
    let stride = usize::try_from(width).expect("positive cursor width") * 4;
    let mut bgra = vec![0; stride * usize::try_from(height).expect("positive cursor height")];
    texture.download(&mut bgra, stride);
    let mut rgba = Vec::with_capacity(bgra.len());
    for pixel in bgra.chunks_exact(4) {
        rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
    }
    let theme_size = gtk::Settings::for_display(display).gtk_cursor_theme_size();
    let theme_size = if theme_size > 0 {
        theme_size
    } else {
        DEFAULT_CURSOR_THEME_SIZE
    };
    let scale = theme_size as f32 / width as f32;
    SoftwareCursor::from_rgba_premultiplied(
        &rgba,
        width as u32,
        height as u32,
        (hot_spot * scale).round(),
        vec2(
            (width as f32 * scale).round(),
            (height as f32 * scale).round(),
        ),
    )
    .expect("GTK system cursor must have valid pixels")
}
