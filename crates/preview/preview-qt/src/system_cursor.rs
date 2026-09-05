use shrimply_skia_adw_core::Vec2;
use shrimply_skia_adw_core::cursor::SoftwareCursor;
use std::ffi::{CString, c_char, c_int};
use std::ptr;

const DEFAULT_CURSOR_SIZE: i32 = 24;

#[repr(C)]
struct XcursorImage {
    version: u32,
    size: u32,
    width: u32,
    height: u32,
    xhot: u32,
    yhot: u32,
    delay: u32,
    pixels: *mut u32,
}

#[link(name = "Xcursor")]
unsafe extern "C" {
    fn XcursorLibraryLoadImage(
        library: *const c_char,
        theme: *const c_char,
        size: c_int,
    ) -> *mut XcursorImage;
    fn XcursorImageDestroy(image: *mut XcursorImage);
}

pub fn grabbing() -> SoftwareCursor {
    let name = CString::new("grabbing").expect("cursor name must not contain NUL");
    let theme = std::env::var("XCURSOR_THEME")
        .ok()
        .map(|theme| CString::new(theme).expect("cursor theme must not contain NUL"));
    let size = std::env::var("XCURSOR_SIZE")
        .ok()
        .and_then(|size| size.parse().ok())
        .filter(|size| *size > 0)
        .unwrap_or(DEFAULT_CURSOR_SIZE);
    let image = unsafe {
        XcursorLibraryLoadImage(
            name.as_ptr(),
            theme.as_ref().map_or(ptr::null(), |theme| theme.as_ptr()),
            size,
        )
    };
    assert!(
        !image.is_null(),
        "system cursor theme has no grabbing cursor"
    );
    let image_ref = unsafe { &*image };
    let pixel_count = usize::try_from(image_ref.width)
        .expect("cursor width must fit usize")
        .checked_mul(usize::try_from(image_ref.height).expect("cursor height must fit usize"))
        .expect("cursor pixel count overflow");
    assert!(!image_ref.pixels.is_null(), "system cursor has no pixels");
    let argb = unsafe { std::slice::from_raw_parts(image_ref.pixels, pixel_count) };
    let mut rgba = Vec::with_capacity(pixel_count * 4);
    for pixel in argb {
        rgba.extend_from_slice(&[
            (pixel >> 16) as u8,
            (pixel >> 8) as u8,
            *pixel as u8,
            (pixel >> 24) as u8,
        ]);
    }
    let cursor = SoftwareCursor::from_rgba_premultiplied(
        &rgba,
        image_ref.width,
        image_ref.height,
        Vec2::new(image_ref.xhot as f32, image_ref.yhot as f32),
        Vec2::new(image_ref.width as f32, image_ref.height as f32),
    )
    .expect("system cursor must have valid dimensions and pixels");
    unsafe { XcursorImageDestroy(image) };
    cursor
}
