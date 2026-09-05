use std::cell::RefCell;

use hashbrown::HashMap;
use skia_safe::font_style::{Slant, Weight, Width};
use skia_safe::{Font, FontHinting, FontMgr, FontStyle, Typeface, font};

pub fn font_with_families(families: &[String], weight: f32, size: f32) -> Font {
    let size = size.max(1.0);
    let weight = font_weight(weight);
    let font = requested_typeface(families, weight)
        .or_else(|| default_typeface(weight))
        .map(|typeface| Font::from_typeface(typeface, size))
        .unwrap_or_else(|| {
            let mut fallback = Font::default();
            fallback.set_size(size);
            fallback
        });
    configured_font(font)
}

pub fn family_names() -> Vec<String> {
    thread_local! {
        static FAMILY_NAMES: Vec<String> = {
            let mut families: Vec<String> = FontMgr::new().family_names().collect();
            families.sort();
            families.dedup();
            families
        };
    }

    FAMILY_NAMES.with(Clone::clone)
}

fn configured_font(mut font: Font) -> Font {
    font.set_subpixel(true);
    font.set_linear_metrics(true);
    font.set_embedded_bitmaps(false);
    font.set_baseline_snap(false);
    font.set_hinting(FontHinting::Slight);
    font.set_edging(font::Edging::AntiAlias);
    font
}

fn requested_typeface(families: &[String], weight: i32) -> Option<Typeface> {
    families
        .iter()
        .map(|family| family.trim())
        .find_map(|family| requested_typeface_for_family(family, weight))
}

fn requested_typeface_for_family(family: &str, weight: i32) -> Option<Typeface> {
    let family = family.trim();
    if family.is_empty() {
        return None;
    }
    thread_local! {
        static REQUESTED_TYPEFACES: RefCell<HashMap<(String, i32), Option<Typeface>>> =
            RefCell::new(HashMap::new());
    }

    REQUESTED_TYPEFACES.with(|typefaces| {
        let key = (family.to_string(), weight);
        if let Some(typeface) = typefaces.borrow().get(&key) {
            return typeface.clone();
        }
        let typeface = resolve_requested_typeface(family, weight);
        typefaces.borrow_mut().insert(key, typeface.clone());
        typeface
    })
}

fn resolve_requested_typeface(family: &str, weight: i32) -> Option<Typeface> {
    let manager = FontMgr::new();
    let style = font_style(weight);
    manager
        .match_family_style(family, style)
        .or_else(|| manager.legacy_make_typeface(Some(family), style))
}

fn default_typeface(weight: i32) -> Option<Typeface> {
    thread_local! {
        static DEFAULT_TYPEFACES: RefCell<HashMap<i32, Option<Typeface>>> =
            RefCell::new(HashMap::new());
    }

    DEFAULT_TYPEFACES.with(|typefaces| {
        if let Some(typeface) = typefaces.borrow().get(&weight) {
            return typeface.clone();
        }
        let typeface = resolve_default_typeface(weight);
        typefaces.borrow_mut().insert(weight, typeface.clone());
        typeface
    })
}

fn resolve_default_typeface(weight: i32) -> Option<Typeface> {
    let manager = FontMgr::new();
    let style = font_style(weight);
    for family in [
        "Cantarell",
        "DejaVu Sans",
        "Product Sans",
        "Noto Sans",
        "Liberation Sans",
        "Arial",
        "sans-serif",
    ] {
        if let Some(typeface) = manager.match_family_style(family, style) {
            return Some(typeface);
        }
        if let Some(typeface) = manager.legacy_make_typeface(Some(family), style) {
            return Some(typeface);
        }
    }

    None
}

fn font_weight(weight: f32) -> i32 {
    weight.round().clamp(1.0, 1000.0) as i32
}

fn font_style(weight: i32) -> FontStyle {
    FontStyle::new(Weight::from(weight), Width::NORMAL, Slant::Upright)
}
