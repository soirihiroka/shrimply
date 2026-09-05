use hashbrown::HashMap;
use std::cell::RefCell;

use skia_safe::{Canvas, FontMgr, Picture, PictureRecorder, svg};

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

thread_local! {
    static ICONS: RefCell<HashMap<IconCacheKey, Option<Picture>>> = RefCell::new(HashMap::new());
}

include!(concat!(env!("OUT_DIR"), "/icons.rs"));

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
        let Some(picture) = icons.entry(key).or_insert_with(|| {
            let source = tint(icon_svg(icon.0), key);
            let dom = match svg::Dom::from_str(&source, FontMgr::new()) {
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

fn tint(svg: &str, key: IconCacheKey) -> String {
    let opacity = (f32::from(key.color.a) / 255.0).to_string();
    let color = format!("#{:02x}{:02x}{:02x}", key.color.r, key.color.g, key.color.b);
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
