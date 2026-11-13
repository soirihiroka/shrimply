//! Bounded immutable outlines shared by labels, code runs, reverse seeks and hot patches.
use super::*;
use sanim_core::{Element, Font, Geometry, cache::Cache};
use std::sync::{Arc, Mutex, OnceLock, Weak};

const MAX_SHAPED_BYTES: usize = 64 * 1024 * 1024;
const MAX_SHAPED_ENTRIES: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum FontKey {
    Asset(usize),
    Family(Arc<[String]>),
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Key {
    font: FontKey,
    epoch: Option<FontEpoch>,
    text: String,
    size: Fraction,
    bold: bool,
    italic: bool,
}

pub struct ShapedText {
    pub bounds: Bounds,
    pub outline: Arc<Vec<TextSegment>>,
    pub advance: f64,
    // A weak allocation guard rejects reused addresses without retaining a font per run.
    asset: Option<Weak<Vec<u8>>>,
}

pub fn cached_shape(
    font: &Font,
    text: &str,
    size: Fraction,
    bold: bool,
    italic: bool,
) -> Result<Arc<ShapedText>, String> {
    static CACHE: OnceLock<Mutex<Cache<Key, ShapedText>>> = OnceLock::new();
    if text.len() > sanim_core::math::format::MAX_OUTPUT_BYTES {
        return Err("text content exceeds its byte budget".into());
    }
    let (font_key, epoch, font_bytes) = match font {
        Font::Asset(bytes) => (FontKey::Asset(Arc::as_ptr(bytes) as usize), None, 0),
        Font::Family(families) => (
            FontKey::Family(families.clone()),
            system_font_epoch(),
            families.iter().map(String::len).sum(),
        ),
    };
    let key = Key {
        font: font_key,
        epoch,
        text: text.into(),
        size,
        bold,
        italic,
    };
    let cache = CACHE.get_or_init(|| Mutex::new(Cache::new(MAX_SHAPED_BYTES, MAX_SHAPED_ENTRIES)));
    if let Some(shaped) = cache
        .lock()
        .expect("native text outline cache poisoned")
        .get(&key)
        && shaped
            .asset
            .as_ref()
            .is_none_or(|asset| asset.strong_count() > 0)
    {
        return Ok(shaped);
    }
    let (bounds, outline, advance) = shape_styled(font, text, size, bold, italic)?;
    let bytes = std::mem::size_of::<ShapedText>()
        + std::mem::size_of::<Key>()
        + text.len()
        + font_bytes
        + outline.len() * std::mem::size_of::<TextSegment>();
    if bytes > MAX_SHAPED_BYTES {
        return Err("shaped text exceeds the native outline byte budget".into());
    }
    let shaped = Arc::new(ShapedText {
        bounds,
        outline: Arc::new(outline),
        advance,
        asset: if let Font::Asset(bytes) = font {
            Some(Arc::downgrade(bytes))
        } else {
            None
        },
    });
    cache
        .lock()
        .expect("native text outline cache poisoned")
        .insert(key, shaped.clone(), bytes);
    Ok(shaped)
}

pub fn resolve_pending(element: &mut Element) -> Result<(), String> {
    let Geometry::Text {
        text,
        font,
        size,
        pending: true,
        ..
    } = element.geometry.as_ref()
    else {
        return Ok(());
    };
    let shaped = cached_shape(font, text, *size, false, false)
        .map_err(|error| format!("Text '{}': {error}", element.id))?;
    let Geometry::Text {
        bounds,
        outline,
        pending,
        ..
    } = Arc::make_mut(&mut element.geometry)
    else {
        unreachable!("resolved Text descriptor")
    };
    *bounds = shaped.bounds;
    *outline = shaped.outline.clone();
    *pending = false;
    Ok(())
}
