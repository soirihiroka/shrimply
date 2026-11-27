use std::{
    cell::RefCell,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};

use hashbrown::HashMap;
use skia_safe::font_style::{Slant, Weight, Width};
use skia_safe::{Font, FontHinting, FontMgr, FontStyle, Typeface, font};

struct SystemFonts {
    generation: u64,
    manager: FontMgr,
    requested: HashMap<(String, i32), Option<Typeface>>,
    defaults: HashMap<i32, Option<Typeface>>,
    families: Option<Vec<String>>,
}

thread_local! {static FONTS: RefCell<Option<SystemFonts>> = const {RefCell::new(None)};}

pub(crate) fn font_generation() -> u64 {
    static GENERATION: AtomicU64 = AtomicU64::new(0);
    static CHANGES: OnceLock<shrimply_fonts::FontChange> = OnceLock::new();
    CHANGES.get_or_init(|| {
        shrimply_fonts::subscribe(Arc::new(|| {
            GENERATION
                .try_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                    value.checked_add(1)
                })
                .expect("font generation overflow");
        }))
    });
    GENERATION.load(Ordering::Acquire)
}

fn with_fonts<R>(access: impl FnOnce(&mut SystemFonts) -> R) -> R {
    let generation = font_generation();
    FONTS.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot
            .as_ref()
            .is_none_or(|fonts| fonts.generation != generation)
        {
            *slot = Some(SystemFonts {
                generation,
                manager: FontMgr::new(),
                requested: HashMap::new(),
                defaults: HashMap::new(),
                families: None,
            });
        }
        access(slot.as_mut().expect("system font manager initialized"))
    })
}

pub(crate) fn font_manager() -> FontMgr {
    with_fonts(|fonts| fonts.manager.clone())
}

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
    with_fonts(|fonts| {
        fonts
            .families
            .get_or_insert_with(|| {
                let mut families: Vec<String> = fonts.manager.family_names().collect();
                families.sort();
                families.dedup();
                families
            })
            .clone()
    })
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
    with_fonts(|fonts| {
        fonts
            .requested
            .entry((family.to_string(), weight))
            .or_insert_with(|| resolve_requested_typeface(&fonts.manager, family, weight))
            .clone()
    })
}

fn resolve_requested_typeface(manager: &FontMgr, family: &str, weight: i32) -> Option<Typeface> {
    let style = font_style(weight);
    manager
        .match_family_style(family, style)
        .or_else(|| manager.legacy_make_typeface(Some(family), style))
}

fn default_typeface(weight: i32) -> Option<Typeface> {
    with_fonts(|fonts| {
        fonts
            .defaults
            .entry(weight)
            .or_insert_with(|| resolve_default_typeface(&fonts.manager, weight))
            .clone()
    })
}

fn resolve_default_typeface(manager: &FontMgr, weight: i32) -> Option<Typeface> {
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
