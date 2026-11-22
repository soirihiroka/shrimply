use cached::{Cached, stores::LruCache};
use std::cell::RefCell;

use skia_safe::{Canvas, Picture, PictureRecorder, svg};

use super::{Color, Rect};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Icon(pub &'static str);

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
struct IconCacheKey {
    name: &'static str,
    color: Color<u8>,
    width: u32,
    height: u32,
}

const MAX_ICON_CACHE_ENTRIES: usize = 256;
thread_local! {
    static ICONS: RefCell<(u64,LruCache<IconCacheKey,Option<Picture>>)> = RefCell::new((0,
        LruCache::builder().max_size(MAX_ICON_CACHE_ENTRIES).build().expect("valid icon cache size")));
}

pub fn draw(canvas: &Canvas, icon: Icon, bounds: Rect, color: Color) {
    if !bounds.width().is_finite()
        || !bounds.height().is_finite()
        || bounds.width() <= 0.0
        || bounds.height() <= 0.0
    {
        return;
    }

    let key = IconCacheKey {
        name: icon.0,
        color: Color::from_srgba(color.to_array()),
        width: bounds.width().to_bits(),
        height: bounds.height().to_bits(),
    };

    ICONS.with(|icons| {
        let mut icons = icons.borrow_mut();
        let generation = crate::skia_font::font_generation();
        if icons.0 != generation {
            icons.1.cache_clear();
            icons.0 = generation;
        }
        let Some(picture) = icons.1.cache_get_or_set_with(key, || {
            let raw = shrimply_components_core::icons::svg(icon.0)
                .unwrap_or_else(|| panic!("unknown symbolic icon: {}", icon.0));
            let source = tint(raw, key.color);
            let dom = match svg::Dom::from_str(&source, crate::skia_font::font_manager()) {
                Ok(dom) => dom,
                Err(error) => {
                    eprintln!("could not load bundled icon {}: {error:?}", icon.0);
                    return None;
                }
            };
            let mut root = dom.root();
            root.set_width(svg::Length::new(bounds.width(), svg::LengthUnit::PX));
            root.set_height(svg::Length::new(bounds.height(), svg::LengthUnit::PX));
            let picture_bounds = skia_safe::Rect::from_wh(bounds.width(), bounds.height());
            let mut recorder = PictureRecorder::new();
            dom.render(recorder.begin_recording(picture_bounds, false));
            Some(
                recorder
                    .finish_recording_as_picture(Some(&picture_bounds))
                    .expect("icon recording must contain a canvas"),
            )
        }) else {
            return;
        };

        canvas.save();
        canvas.translate((bounds.left(), bounds.top()));
        canvas.draw_picture(picture, None, None);
        canvas.restore();
    });
}

fn tint(svg: &str, color: Color<u8>) -> String {
    let opacity = (f32::from(color.a) / 255.0).to_string();
    let color = format!("#{:02x}{:02x}{:02x}", color.r, color.g, color.b);
    let Some(root_start) = svg.find("<svg") else {
        return svg.to_owned();
    };
    let Some(root_end) = svg[root_start..].find('>').map(|end| root_start + end) else {
        return svg.to_owned();
    };
    let root = &svg[root_start..root_end];
    let mut source = String::with_capacity(svg.len() + color.len() * 2 + opacity.len() + 32);
    source.push_str(&svg[..root_end]);
    if !root.contains(" fill=") {
        source.push_str(" fill=\"");
        source.push_str(&color);
        source.push('"');
    }
    if !root.contains(" color=") {
        source.push_str(" color=\"");
        source.push_str(&color);
        source.push('"');
    }
    if !root.contains(" opacity=") {
        source.push_str(" opacity=\"");
        source.push_str(&opacity);
        source.push('"');
    }
    source.push_str(&svg[root_end..]);

    let mut tinted = String::with_capacity(source.len());
    let mut rest = source.as_str();
    while let Some(hash) = rest.find('#') {
        tinted.push_str(&rest[..hash]);
        let value = &rest[hash + 1..];
        let digits = value.bytes().take_while(u8::is_ascii_hexdigit).count();
        if matches!(digits, 3 | 6 | 8) {
            tinted.push_str(&color);
            rest = &value[digits..];
        } else {
            tinted.push('#');
            rest = value;
        }
    }
    tinted.push_str(rest);
    tinted
}
