//! One native shaping pipeline for editor text, 3D text and headless Sanim.
//! Pango layouts remain on their owning thread; immutable outlines share one
//! byte-bounded cache across threads and renderers.
use super::{FontEpoch, PreparedFont, glyph_is_color, prepare, system_font_epoch};
use crate::{Font, TextSegment};
use shrimply_resource_pipeline::cache::Cache;
use std::{
    cell::RefCell,
    rc::Rc,
    sync::{Arc, Mutex, OnceLock, Weak},
};

pub const MAX_TEXT_BYTES: usize = 1_048_576;
const MAX_SHAPED_BYTES: usize = 64 * 1024 * 1024;
const MAX_SHAPED_ENTRIES: usize = 4096;
const MAX_NATIVE_ENTRIES: usize = 512;

#[derive(Clone)]
pub struct Options {
    pub size: f64,
    pub weight: i32,
    pub style: pango::Style,
    pub variations: Vec<(String, f32)>,
    pub vertical: bool,
    pub alignment: pango::Alignment,
    pub justify: bool,
    pub tracking: f64,
    pub line_spacing: f64,
}
impl Options {
    pub fn new(size: f64) -> Self {
        Self {
            size,
            weight: 400,
            style: pango::Style::Normal,
            variations: vec![],
            vertical: false,
            alignment: pango::Alignment::Left,
            justify: false,
            tracking: 0.,
            line_spacing: 1.,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
enum FontKey {
    Asset(usize),
    Family(Arc<[String]>),
}
#[derive(Clone, PartialEq, Eq, Hash)]
struct Key {
    font: FontKey,
    epoch: Option<FontEpoch>,
    text: Arc<str>,
    size: u64,
    weight: i32,
    style: pango::Style,
    variations: Arc<[(String, u32)]>,
    vertical: bool,
    alignment: pango::Alignment,
    justify: bool,
    tracking: u64,
    line_spacing: u64,
}

impl Key {
    fn retained_bytes(&self) -> usize {
        let families = match &self.font {
            FontKey::Family(names) => {
                std::mem::size_of_val(names.as_ref())
                    + names.iter().map(String::capacity).sum::<usize>()
            }
            FontKey::Asset(_) => 0,
        };
        // LRU index keys share these immutable payloads with the primary key.
        2 * std::mem::size_of::<Self>()
            + self.text.len()
            + families
            + std::mem::size_of_val(self.variations.as_ref())
            + self
                .variations
                .iter()
                .map(|(axis, _)| axis.capacity())
                .sum::<usize>()
    }
}

pub struct ShapedText {
    /// Bounds and outline use the first-baseline origin (before any alignment).
    pub bounds: [f64; 4],
    pub outline: Arc<Vec<TextSegment>>,
    pub advance: f64,
    /// Translation from Pango's top-left coordinate system to these outlines.
    pub origin: [f64; 2],
    pub missing_glyphs: i32,
    pub color_glyphs: bool,
    asset: Option<Weak<Vec<u8>>>,
}
pub struct NativeLayout {
    prepared: PreparedFont,
    pub shaped: Arc<ShapedText>,
}
impl NativeLayout {
    pub fn layout(&self) -> &pango::Layout {
        &self.prepared.layout
    }
}

thread_local! {
    static LAYOUTS: RefCell<Cache<Key, NativeLayout, Rc<NativeLayout>>> = RefCell::new(
        Cache::new(MAX_SHAPED_BYTES, MAX_NATIVE_ENTRIES));
}
fn outlines() -> &'static Mutex<Cache<Key, ShapedText>> {
    static CACHE: OnceLock<Mutex<Cache<Key, ShapedText>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Cache::new(MAX_SHAPED_BYTES, MAX_SHAPED_ENTRIES)))
}
fn alive(shaped: &ShapedText) -> bool {
    shaped
        .asset
        .as_ref()
        .is_none_or(|asset| asset.strong_count() > 0)
}
fn key(font: &Font, text: &str, options: &Options) -> Result<Key, String> {
    if text.len() > MAX_TEXT_BYTES {
        return Err("text content exceeds its byte budget".into());
    }
    if !options.size.is_finite()
        || options.size <= 0.
        || !options.tracking.is_finite()
        || !options.line_spacing.is_finite()
        || options.line_spacing <= 0.
        || options.line_spacing > f64::from(f32::MAX)
    {
        return Err("native text size, spacing and tracking must be finite with positive size and line spacing".into());
    }
    let mut variations = options
        .variations
        .iter()
        .filter(|(axis, _)| axis != "wght" && axis != "ital")
        .map(|(axis, value)| (axis.clone(), value.to_bits()))
        .collect::<Vec<_>>();
    if variations.iter().any(|(axis, value)| {
        axis.len() != 4
            || !axis.bytes().all(|b| b.is_ascii_graphic())
            || !f32::from_bits(*value).is_finite()
    }) {
        return Err(
            "font variation axes require four printable ASCII characters and finite values".into(),
        );
    }
    variations.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    variations.dedup_by(|a, b| a.0 == b.0);
    Ok(Key {
        font: match font {
            Font::Asset(bytes) => FontKey::Asset(Arc::as_ptr(bytes) as usize),
            Font::Family(names) => FontKey::Family(names.clone()),
        },
        epoch: matches!(font, Font::Family(_))
            .then(system_font_epoch)
            .flatten(),
        text: text.into(),
        size: options.size.to_bits(),
        weight: options.weight.clamp(1, 1000),
        style: options.style,
        variations: variations.into(),
        vertical: options.vertical,
        alignment: options.alignment,
        justify: options.justify,
        tracking: options.tracking.to_bits(),
        line_spacing: options.line_spacing.to_bits(),
    })
}

/// The shared immutable outline artifact; hits allocate no Pango layout or path.
pub fn shape(font: &Font, text: &str, options: &Options) -> Result<Arc<ShapedText>, String> {
    let key = key(font, text, options)?;
    if let Some(shaped) = outlines()
        .lock()
        .expect("native outline cache poisoned")
        .get(&key)
        && alive(&shaped)
    {
        return Ok(shaped);
    }
    Ok(layout_with_key(font, options, key)?.shaped.clone())
}

/// Native layout evidence for editor glyph masks and color glyphs. Callers must
/// treat the returned Pango layout as immutable, like the shared outline.
pub fn layout(font: &Font, text: &str, options: &Options) -> Result<Rc<NativeLayout>, String> {
    layout_with_key(font, options, key(font, text, options)?)
}
fn layout_with_key(
    font: &Font,
    options: &Options,
    mut key: Key,
) -> Result<Rc<NativeLayout>, String> {
    if let Some(layout) = LAYOUTS.with(|cache| cache.borrow_mut().get(&key))
        && alive(&layout.shaped)
    {
        return Ok(layout);
    }
    let prepared = prepare(font, options.size)?;
    let layout = &prepared.layout;
    let mut description = layout
        .font_description()
        .ok_or("native text has no font description")?;
    description.set_color(pango::FontColor::DontCare);
    description.set_weight(pango::Weight::__Unknown(key.weight));
    description.set_style(key.style);
    let mut variations = key
        .variations
        .iter()
        .map(|(axis, value)| format!("{axis}={}", f32::from_bits(*value)))
        .collect::<Vec<_>>();
    variations.push(format!("wght={}", key.weight));
    if key.style == pango::Style::Italic {
        variations.push("ital=1".into());
    }
    description.set_variations(Some(&variations.join(",")));
    layout.set_font_description(Some(&description));
    let attrs = layout.attributes().unwrap_or_default();
    if options.tracking != 0. {
        let tracking = options.tracking * f64::from(pango::SCALE);
        if tracking.abs() > f64::from(i32::MAX) {
            return Err("text tracking exceeds Pango's range".into());
        }
        attrs.insert(pango::AttrInt::new_letter_spacing(tracking.round() as i32));
    }
    layout.set_attributes(Some(&attrs));
    if options.line_spacing != 1. {
        layout.set_line_spacing(options.line_spacing as f32);
    }
    let surface =
        cairo::RecordingSurface::create(cairo::Content::Alpha, None).map_err(|e| e.to_string())?;
    let canvas = cairo::Context::new(&surface).map_err(|e| e.to_string())?;
    if options.vertical {
        canvas.rotate(std::f64::consts::FRAC_PI_2);
    }
    let context = layout.context();
    pangocairo::functions::update_context(&canvas, &context);
    context.set_base_gravity(if options.vertical {
        pango::Gravity::East
    } else {
        pango::Gravity::South
    });
    context.set_gravity_hint(pango::GravityHint::Natural);
    layout.context_changed();
    layout.set_text(&key.text);
    let (_, natural) = layout.extents();
    if options.alignment != pango::Alignment::Left || options.justify {
        layout.set_width(natural.width().max(1));
    }
    layout.set_alignment(options.alignment);
    layout.set_justify(options.justify);
    layout.set_justify_last_line(options.justify);
    let baseline = f64::from(layout.baseline()) / f64::from(pango::SCALE);
    canvas.move_to(0., -baseline);
    pangocairo::functions::layout_path(&canvas, layout);
    canvas.identity_matrix();
    let (left, top, right, bottom) = canvas.path_extents().map_err(|e| e.to_string())?;
    let origin = if options.vertical {
        [baseline, 0.]
    } else {
        [0., -baseline]
    };
    let (ink, logical) = layout.extents();
    let rectangle = |r: &pango::Rectangle| {
        let scale = f64::from(pango::SCALE);
        let x = f64::from(r.x()) / scale;
        let y = f64::from(r.y()) / scale;
        let w = f64::from(r.width()) / scale;
        let h = f64::from(r.height()) / scale;
        if options.vertical {
            [-y - h + origin[0], x, -y + origin[0], x + w]
        } else {
            [x, y + origin[1], x + w, y + h + origin[1]]
        }
    };
    let ink = rectangle(&ink);
    let logical = rectangle(&logical);
    let bounds = [
        left.min(ink[0]).min(logical[0]),
        top.min(ink[1]).min(logical[1]),
        right.max(ink[2]).max(logical[2]),
        bottom.max(ink[3]).max(logical[3]),
    ];
    if bounds.iter().any(|v| !v.is_finite()) {
        return Err("Pango text bounds overflow".into());
    }
    let outline = canvas
        .copy_path()
        .map_err(|e| e.to_string())?
        .iter()
        .map(|segment| match segment {
            cairo::PathSegment::MoveTo(p) => TextSegment::Move([p.0, p.1]),
            cairo::PathSegment::LineTo(p) => TextSegment::Line([p.0, p.1]),
            cairo::PathSegment::CurveTo(a, b, c) => {
                TextSegment::Curve([a.0, a.1], [b.0, b.1], [c.0, c.1])
            }
            cairo::PathSegment::ClosePath => TextSegment::Close,
        })
        .collect::<Vec<_>>();
    let mut color_glyphs = false;
    let mut runs = layout.iter();
    loop {
        if let Some(run) = runs.run_readonly() {
            color_glyphs |= run.glyph_string().glyph_info().iter().any(glyph_is_color);
        }
        if !runs.next_run() {
            break;
        }
    }
    let shaped = Arc::new(ShapedText {
        bounds,
        outline: Arc::new(outline),
        origin,
        advance: f64::from(natural.width()) / f64::from(pango::SCALE),
        missing_glyphs: layout.unknown_glyphs_count(),
        color_glyphs,
        asset: if let Font::Asset(bytes) = font {
            Some(Arc::downgrade(bytes))
        } else {
            None
        },
    });
    let font_bytes = match font {
        Font::Asset(bytes) => bytes.len(),
        Font::Family(names) => {
            std::mem::size_of_val(names.as_ref())
                + names.iter().map(String::capacity).sum::<usize>()
        }
    };
    let bytes = std::mem::size_of::<ShapedText>()
        + key.retained_bytes()
        + shaped.outline.len() * std::mem::size_of::<TextSegment>();
    if bytes > MAX_SHAPED_BYTES {
        return Err("shaped text exceeds the native outline byte budget".into());
    }
    if key.epoch.is_some() {
        key.epoch = system_font_epoch();
    }
    let shaped = {
        let mut cache = outlines().lock().expect("native outline cache poisoned");
        if let Some(existing) = cache.get(&key)
            && alive(&existing)
        {
            existing
        } else {
            cache.insert(key.clone(), shaped.clone(), bytes);
            shaped
        }
    };
    let native = Rc::new(NativeLayout { prepared, shaped });
    // Pango retains glyph/item arrays alongside the immutable outline.
    let retained = bytes
        .saturating_add(font_bytes)
        .saturating_add(key.text.len().saturating_mul(128));
    LAYOUTS.with(|cache| cache.borrow_mut().insert(key, native.clone(), retained));
    Ok(native)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn variation_key_clones_share_and_charge_their_immutable_payload() {
        let font = Font::default();
        let plain = key(&font, "same text", &Options::new(18.)).unwrap();
        let mut options = Options::new(18.);
        options.variations = vec![("wdth".into(), 90.), ("opsz".into(), 24.)];
        let varied = key(&font, "same text", &options).unwrap();
        let indexed = varied.clone();
        assert!(Arc::ptr_eq(&varied.variations, &indexed.variations));
        let payload = std::mem::size_of_val(varied.variations.as_ref())
            + varied
                .variations
                .iter()
                .map(|(axis, _)| axis.len())
                .sum::<usize>();
        assert!(varied.retained_bytes() >= plain.retained_bytes() + payload);
    }
}
