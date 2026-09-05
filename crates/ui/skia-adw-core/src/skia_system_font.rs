use std::{cell::RefCell, rc::Rc};

use cached::{Cached, stores::LruCache};
use skia_safe::{
    FontMgr,
    textlayout::{FontCollection, Paragraph, ParagraphBuilder, ParagraphStyle, TextStyle},
};

use shrimply_math_color::Color;

const MAX_PARAGRAPH_CACHE_ENTRIES: usize = 256;

thread_local! {
    static FONTS: FontCollection = {
        let mut fonts = FontCollection::new();
        fonts.set_default_font_manager(FontMgr::new(), None);
        fonts
    };
    static PARAGRAPHS: RefCell<LruCache<Key, Rc<Paragraph>>> = RefCell::new(
        LruCache::builder()
            .max_size(MAX_PARAGRAPH_CACHE_ENTRIES)
            .build()
            .expect("valid Skia paragraph cache size"),
    );
}

pub fn paragraph(text: &str, font_size: f32, color: Color) -> Rc<Paragraph> {
    let key = Key {
        text: text.to_string(),
        font_size: font_size.to_bits(),
        color: color.map(f32::to_bits),
        max_width: None,
    };
    if let Some(paragraph) =
        PARAGRAPHS.with(|paragraphs| paragraphs.borrow_mut().cache_get(&key).map(Rc::clone))
    {
        return paragraph;
    }

    let paragraph =
        FONTS.with(|fonts| build_paragraph(text, font_size, color, None, fonts.clone()));
    PARAGRAPHS.with(|paragraphs| {
        paragraphs
            .borrow_mut()
            .cache_set(key, Rc::clone(&paragraph));
    });
    paragraph
}

pub fn ellipsized_paragraph(
    text: &str,
    font_size: f32,
    color: Color,
    max_width: f32,
) -> Rc<Paragraph> {
    let max_width = max_width.floor().max(1.0);
    let key = Key {
        text: text.to_string(),
        font_size: font_size.to_bits(),
        color: color.map(f32::to_bits),
        max_width: Some(max_width.to_bits()),
    };
    if let Some(paragraph) =
        PARAGRAPHS.with(|paragraphs| paragraphs.borrow_mut().cache_get(&key).map(Rc::clone))
    {
        return paragraph;
    }

    let paragraph =
        FONTS.with(|fonts| build_paragraph(text, font_size, color, Some(max_width), fonts.clone()));
    PARAGRAPHS.with(|paragraphs| {
        paragraphs
            .borrow_mut()
            .cache_set(key, Rc::clone(&paragraph));
    });
    paragraph
}

#[derive(Clone, Eq, Hash, PartialEq)]
struct Key {
    text: String,
    font_size: u32,
    color: Color<u32>,
    max_width: Option<u32>,
}

fn build_paragraph(
    text: &str,
    font_size: f32,
    color: Color,
    max_width: Option<f32>,
    fonts: FontCollection,
) -> Rc<Paragraph> {
    let mut style = TextStyle::new();
    style
        .set_color(color)
        .set_font_size(font_size.max(1.0))
        .set_font_families(&["sans-serif"]);

    let mut paragraph_style = ParagraphStyle::new();
    if max_width.is_some() {
        paragraph_style.set_max_lines(1).set_ellipsis("...");
    }
    let mut builder = ParagraphBuilder::new(&paragraph_style, fonts);
    builder.push_style(&style).add_text(text);
    let mut paragraph = builder.build();
    paragraph.layout(max_width.unwrap_or(f32::MAX));
    Rc::new(paragraph)
}
