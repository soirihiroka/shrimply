use shrimply_components_skia::Vec2;
use shrimply_components_skia::cursor::SoftwareCursor;
#[cfg(target_os = "linux")]
use std::ffi::{CString, c_char, c_int};
#[cfg(target_os = "linux")]
use std::ptr;
#[cfg(windows)]
use windows::Win32::{
    Graphics::Gdi::{
        BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection,
        DIB_RGB_COLORS, DeleteDC, DeleteObject, GdiFlush, GetObjectW, SelectObject,
    },
    UI::WindowsAndMessaging::{DI_NORMAL, DrawIconEx, GetIconInfo, ICONINFO, IDC_SIZEALL},
};

#[cfg(target_os = "linux")]
const DEFAULT_CURSOR_SIZE: i32 = 24;

#[cfg(target_os = "linux")]
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

#[cfg(target_os = "linux")]
#[link(name = "Xcursor")]
unsafe extern "C" {
    fn XcursorLibraryLoadImage(
        library: *const c_char,
        theme: *const c_char,
        size: c_int,
    ) -> *mut XcursorImage;
    fn XcursorImageDestroy(image: *mut XcursorImage);
}

#[cfg(target_os = "linux")]
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

#[cfg(windows)]
pub fn grabbing(scale: f32) -> SoftwareCursor {
    assert!(
        scale.is_finite() && scale > 0.0,
        "Qt cursor scale must be positive"
    );
    let cursor = unsafe {
        windows::Win32::UI::WindowsAndMessaging::LoadCursorW(None, IDC_SIZEALL)
            .expect("Windows panning cursor must exist")
    };

    let mut icon_info = ICONINFO::default();
    unsafe { GetIconInfo(cursor.into(), &mut icon_info) }
        .expect("Windows panning cursor must expose its hotspot");
    let monochrome = icon_info.hbmColor.is_invalid();
    let cursor_bitmap = if monochrome {
        icon_info.hbmMask
    } else {
        icon_info.hbmColor
    };
    assert!(!cursor_bitmap.is_invalid(), "Windows cursor has no bitmap");
    let mut bitmap = BITMAP::default();
    let bitmap_size = i32::try_from(std::mem::size_of::<BITMAP>())
        .expect("Windows bitmap metadata size must fit i32");
    assert_eq!(
        unsafe {
            GetObjectW(
                cursor_bitmap.into(),
                bitmap_size,
                Some((&mut bitmap as *mut BITMAP).cast()),
            )
        },
        bitmap_size,
        "could not read Windows cursor bitmap metadata"
    );
    let width = bitmap.bmWidth;
    let height = if monochrome {
        bitmap.bmHeight / 2
    } else {
        bitmap.bmHeight
    };
    assert!(
        width > 0 && height > 0,
        "Windows cursor size must be positive"
    );
    let hot_spot = Vec2::new(
        icon_info.xHotspot as f32 / scale,
        icon_info.yHotspot as f32 / scale,
    );
    unsafe {
        if !icon_info.hbmMask.is_invalid() {
            assert!(DeleteObject(icon_info.hbmMask.into()).as_bool());
        }
        if !icon_info.hbmColor.is_invalid() {
            assert!(DeleteObject(icon_info.hbmColor.into()).as_bool());
        }
    }

    let pixel_count = usize::try_from(width)
        .expect("cursor width must fit usize")
        .checked_mul(usize::try_from(height).expect("cursor height must fit usize"))
        .expect("cursor pixel count overflow");
    let bitmap_info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };

    let dc = unsafe { CreateCompatibleDC(None) };
    assert!(
        !dc.is_invalid(),
        "could not create Windows cursor drawing context"
    );
    let mut bits = std::ptr::null_mut();
    let bitmap =
        unsafe { CreateDIBSection(Some(dc), &bitmap_info, DIB_RGB_COLORS, &mut bits, None, 0) }
            .expect("could not create Windows cursor bitmap");
    assert!(!bits.is_null(), "Windows cursor bitmap has no pixels");
    let previous = unsafe { SelectObject(dc, bitmap.into()) };
    assert!(
        !previous.is_invalid(),
        "could not select Windows cursor bitmap"
    );

    let pixels = unsafe { std::slice::from_raw_parts_mut(bits.cast::<u8>(), pixel_count * 4) };
    pixels.fill(0);
    unsafe { DrawIconEx(dc, 0, 0, cursor.into(), width, height, 0, None, DI_NORMAL) }
        .expect("could not render Windows cursor on black");
    assert!(
        unsafe { GdiFlush() }.as_bool(),
        "could not flush Windows cursor drawing"
    );
    let black = pixels.to_vec();
    pixels.fill(255);
    unsafe { DrawIconEx(dc, 0, 0, cursor.into(), width, height, 0, None, DI_NORMAL) }
        .expect("could not render Windows cursor on white");
    assert!(
        unsafe { GdiFlush() }.as_bool(),
        "could not flush Windows cursor drawing"
    );

    let mut rgba = Vec::with_capacity(pixel_count * 4);
    for (black, white) in black.chunks_exact(4).zip(pixels.chunks_exact(4)) {
        let background = white[0]
            .saturating_sub(black[0])
            .max(white[1].saturating_sub(black[1]))
            .max(white[2].saturating_sub(black[2]));
        rgba.extend_from_slice(&[black[2], black[1], black[0], 255 - background]);
    }

    unsafe {
        assert!(!SelectObject(dc, previous).is_invalid());
        assert!(DeleteObject(bitmap.into()).as_bool());
        assert!(DeleteDC(dc).as_bool());
    }
    SoftwareCursor::from_rgba_premultiplied(
        &rgba,
        width as u32,
        height as u32,
        hot_spot,
        Vec2::new(width as f32 / scale, height as f32 / scale),
    )
    .expect("Windows system cursor must have valid dimensions and pixels")
}
