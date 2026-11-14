//! One native shaping pipeline for editor text, 3D text and headless Sanim.
//! Pango layouts remain on their owning thread; immutable outlines share one
//! byte-bounded cache across threads and renderers.
use super::{FontEpoch, PreparedFont, glyph_is_color, prepare, system_font_epoch};
use crate::{Font, TextSegment};
use shrimply_resource_pipeline::cache::Cache;
use std::{
    cell::{OnceCell, RefCell},
    hash::{Hash, Hasher},
    rc::Rc,
    sync::{Arc, Mutex, OnceLock, Weak},
};

pub const MAX_TEXT_BYTES: usize = 1_048_576;
const MAX_SHAPED_BYTES: usize = 64 * 1024 * 1024;
const MAX_SHAPED_ENTRIES: usize = 4096;
const MAX_NATIVE_ENTRIES: usize = 512;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct StyleSpan {
    pub start: u32,
    pub end: u32,
    pub weight: i32,
    pub style: pango::Style,
}
/// Immutable text and style ranges with content fingerprints computed once.
/// All native APIs share this content-key domain; Arc identity is only an
/// equality fast path, never a second cache or a substitute for content identity.
#[derive(Clone)]
pub struct Source {
    text: Arc<str>,
    spans: Arc<[StyleSpan]>,
    text_hash: u64,
    spans_hash: u64,
}
impl Source {
    pub fn new(text: Arc<str>) -> Result<Self, String> {
        Self::styled(text, Arc::from([]))
    }
    pub fn styled(text: Arc<str>, spans: Arc<[StyleSpan]>) -> Result<Self, String> {
        if text.len() > MAX_TEXT_BYTES {
            return Err("text content exceeds its byte budget".into());
        }
        if spans.iter().any(|span| {
            span.start > span.end
                || span.end as usize > text.len()
                || !text.is_char_boundary(span.start as usize)
                || !text.is_char_boundary(span.end as usize)
        }) {
            return Err("native text style spans must use UTF-8 boundaries inside the text".into());
        }
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut hash);
        let text_hash = hash.finish();
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        spans.hash(&mut hash);
        let spans_hash = hash.finish();
        Ok(Self {
            text,
            spans,
            text_hash,
            spans_hash,
        })
    }
    pub fn text(&self) -> &Arc<str> {
        &self.text
    }
    pub fn spans(&self) -> &Arc<[StyleSpan]> {
        &self.spans
    }
    pub fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>() + self.text.len() + std::mem::size_of_val(self.spans.as_ref())
    }
}
impl PartialEq for Source {
    fn eq(&self, other: &Self) -> bool {
        self.text_hash == other.text_hash
            && self.spans_hash == other.spans_hash
            && (Arc::ptr_eq(&self.text, &other.text) || self.text == other.text)
            && (Arc::ptr_eq(&self.spans, &other.spans) || self.spans == other.spans)
    }
}
impl Eq for Source {}
impl Hash for Source {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.text_hash.hash(state);
        self.spans_hash.hash(state);
    }
}

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
    pub width: Option<f64>,
    pub wrap: pango::WrapMode,
    pub tab_width: Option<f64>,
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
            width: None,
            wrap: pango::WrapMode::Word,
            tab_width: None,
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
    source: Source,
    size: u64,
    weight: i32,
    style: pango::Style,
    variations: Arc<[(String, u32)]>,
    vertical: bool,
    alignment: pango::Alignment,
    justify: bool,
    tracking: u64,
    line_spacing: u64,
    width: Option<u64>,
    wrap: pango::WrapMode,
    tab_width: Option<u64>,
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
            + self.source.retained_bytes()
            + families
            + std::mem::size_of_val(self.variations.as_ref())
            + self
                .variations
                .iter()
                .map(|(axis, _)| axis.capacity())
                .sum::<usize>()
    }
}

pub use crate::ShapedText;

pub struct Metrics {
    pub bounds: [f64; 4],
    pub advance: f64,
    pub origin: [f64; 2],
    pub missing_glyphs: i32,
    pub color_glyphs: bool,
}
pub struct NativeLayout {
    prepared: PreparedFont,
    key: Key,
    pub metrics: Metrics,
    asset: Option<Weak<Vec<u8>>>,
    font_bytes: usize,
    shaped: OnceCell<Arc<ShapedText>>,
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
fn key(font: &Font, text: Arc<str>, options: &Options) -> Result<Key, String> {
    key_with_source(font, Source::new(text)?, options)
}
fn key_with_source(font: &Font, source: Source, options: &Options) -> Result<Key, String> {
    if !options.size.is_finite()
        || options.size <= 0.
        || options.width.is_some_and(|value| {
            !value.is_finite()
                || value < 0.
                || value * f64::from(pango::SCALE) > f64::from(i32::MAX)
        })
        || options.tab_width.is_some_and(|value| {
            !value.is_finite()
                || value <= 0.
                || value * f64::from(pango::SCALE) > f64::from(i32::MAX)
        })
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
        source,
        size: options.size.to_bits(),
        weight: options.weight.clamp(1, 1000),
        style: options.style,
        variations: variations.into(),
        vertical: options.vertical,
        alignment: options.alignment,
        justify: options.justify,
        tracking: options.tracking.to_bits(),
        line_spacing: options.line_spacing.to_bits(),
        width: options.width.map(f64::to_bits),
        wrap: options.wrap,
        tab_width: options.tab_width.map(f64::to_bits),
    })
}

/// The shared immutable outline artifact; hits allocate no Pango layout or path.
pub fn shape(font: &Font, text: &str, options: &Options) -> Result<Arc<ShapedText>, String> {
    let key = key(font, text.into(), options)?;
    if let Some(shaped) = outlines()
        .lock()
        .expect("native outline cache poisoned")
        .get(&key)
        && alive(&shaped)
    {
        return Ok(shaped);
    }
    Ok(layout_with_key(font, options, key)?.shaped()?.clone())
}

/// Native layout evidence for editor glyph masks and color glyphs. Callers must
/// treat the returned Pango layout as immutable, like the shared outline.
pub fn layout(font: &Font, text: &str, options: &Options) -> Result<Rc<NativeLayout>, String> {
    layout_with_key(font, options, key(font, text.into(), options)?)
}
/// Retained code/text sources can share their UTF-8 payload across width keys.
pub fn layout_shared(
    font: &Font,
    source: &Source,
    options: &Options,
) -> Result<Rc<NativeLayout>, String> {
    layout_with_key(
        font,
        options,
        key_with_source(font, source.clone(), options)?,
    )
}
fn layout_with_key(
    font: &Font,
    options: &Options,
    mut key: Key,
) -> Result<Rc<NativeLayout>, String> {
    if let Some(layout) = LAYOUTS.with(|cache| cache.borrow_mut().get(&key))
        && layout
            .asset
            .as_ref()
            .is_none_or(|asset| asset.strong_count() > 0)
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
    // CSS overflow wrapping introduces a break, never a hyphen character.
    if options.width.is_some() {
        attrs.insert(pango::AttrInt::new_insert_hyphens(false));
    }
    if options.tracking != 0. {
        let tracking = options.tracking * f64::from(pango::SCALE);
        if tracking.abs() > f64::from(i32::MAX) {
            return Err("text tracking exceeds Pango's range".into());
        }
        attrs.insert(pango::AttrInt::new_letter_spacing(tracking.round() as i32));
    }
    for span in key.source.spans.iter() {
        let mut font = description.clone();
        font.set_weight(pango::Weight::__Unknown(span.weight));
        font.set_style(span.style);
        let mut variations = key
            .variations
            .iter()
            .map(|(axis, value)| format!("{axis}={}", f32::from_bits(*value)))
            .collect::<Vec<_>>();
        variations.push(format!("wght={}", span.weight));
        if span.style == pango::Style::Italic {
            variations.push("ital=1".into());
        }
        font.set_variations(Some(&variations.join(",")));
        let mut attr = pango::AttrFontDesc::new(&font);
        attr.set_start_index(span.start);
        attr.set_end_index(span.end);
        attrs.insert(attr);
    }
    layout.set_attributes(Some(&attrs));
    if let Some(width) = options.width {
        layout.set_width((width * f64::from(pango::SCALE)).round() as i32);
        layout.set_wrap(options.wrap);
    }
    if let Some(width) = options.tab_width {
        let mut tabs = pango::TabArray::new(1, false);
        tabs.set_tab(
            0,
            pango::TabAlign::Left,
            (width * f64::from(pango::SCALE)).round() as i32,
        );
        layout.set_tabs(Some(&tabs));
    }
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
    layout.set_text(&key.source.text);
    let (_, natural) = layout.extents();
    if options.width.is_none() && (options.alignment != pango::Alignment::Left || options.justify) {
        layout.set_width(natural.width().max(1));
    }
    layout.set_alignment(options.alignment);
    layout.set_justify(options.justify);
    layout.set_justify_last_line(options.justify);
    let baseline = f64::from(layout.baseline()) / f64::from(pango::SCALE);
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
        ink[0].min(logical[0]),
        ink[1].min(logical[1]),
        ink[2].max(logical[2]),
        ink[3].max(logical[3]),
    ];
    if bounds.iter().any(|v| !v.is_finite()) {
        return Err("Pango text bounds overflow".into());
    }
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
    let metrics = Metrics {
        bounds,
        origin,
        advance: f64::from(natural.width()) / f64::from(pango::SCALE),
        missing_glyphs: layout.unknown_glyphs_count(),
        color_glyphs,
    };
    let asset = if let Font::Asset(bytes) = font {
        Some(Arc::downgrade(bytes))
    } else {
        None
    };
    let font_bytes = match font {
        Font::Asset(bytes) => bytes.len(),
        Font::Family(names) => {
            std::mem::size_of_val(names.as_ref())
                + names.iter().map(String::capacity).sum::<usize>()
        }
    };
    if key.epoch.is_some() {
        key.epoch = system_font_epoch();
    }
    let retained = std::mem::size_of::<NativeLayout>()
        + key.retained_bytes()
        + font_bytes
        + key.source.text.len().saturating_mul(128);
    let native = Rc::new(NativeLayout {
        prepared,
        key: key.clone(),
        metrics,
        asset,
        font_bytes,
        shaped: OnceCell::new(),
    });
    LAYOUTS.with(|cache| cache.borrow_mut().insert(key, native.clone(), retained));
    Ok(native)
}

impl NativeLayout {
    /// Real outlines are produced only when a renderer asks for them. Code
    /// paragraph measurement uses metrics and never constructs an unused path.
    pub fn shaped(&self) -> Result<&Arc<ShapedText>, String> {
        if self.shaped.get().is_none() {
            let existing = outlines()
                .lock()
                .expect("native outline cache poisoned")
                .get(&self.key)
                .filter(|value| alive(value));
            let shaped = if let Some(existing) = existing {
                existing
            } else {
                let surface = cairo::RecordingSurface::create(cairo::Content::Alpha, None)
                    .map_err(|error| error.to_string())?;
                let canvas = cairo::Context::new(&surface).map_err(|error| error.to_string())?;
                let baseline = if self.key.vertical {
                    canvas.rotate(std::f64::consts::FRAC_PI_2);
                    self.metrics.origin[0]
                } else {
                    -self.metrics.origin[1]
                };
                canvas.move_to(0., -baseline);
                pangocairo::functions::layout_path(&canvas, self.layout());
                canvas.identity_matrix();
                let (left, top, right, bottom) =
                    canvas.path_extents().map_err(|error| error.to_string())?;
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
                let bounds = self.metrics.bounds;
                let shaped = Arc::new(ShapedText {
                    bounds: [
                        left.min(bounds[0]),
                        top.min(bounds[1]),
                        right.max(bounds[2]),
                        bottom.max(bounds[3]),
                    ],
                    outline: Arc::new(outline),
                    origin: self.metrics.origin,
                    advance: self.metrics.advance,
                    missing_glyphs: self.metrics.missing_glyphs,
                    color_glyphs: self.metrics.color_glyphs,
                    asset: self.asset.clone(),
                });
                let bytes = std::mem::size_of::<ShapedText>()
                    + self.key.retained_bytes()
                    + shaped.outline.len() * std::mem::size_of::<TextSegment>();
                if bytes > MAX_SHAPED_BYTES {
                    return Err("shaped text exceeds the native outline byte budget".into());
                }
                let mut cache = outlines().lock().expect("native outline cache poisoned");
                if let Some(existing) = cache.get(&self.key).filter(|value| alive(value)) {
                    existing
                } else {
                    cache.insert(self.key.clone(), shaped.clone(), bytes);
                    shaped
                }
            };
            let retained = std::mem::size_of::<NativeLayout>()
                + self.key.retained_bytes()
                + self.font_bytes
                + self.key.source.text.len().saturating_mul(128)
                + shaped.outline.len() * std::mem::size_of::<TextSegment>();
            LAYOUTS.with(|cache| {
                let mut cache = cache.borrow_mut();
                if let Some(native) = cache.get(&self.key) {
                    cache.insert(self.key.clone(), native, retained);
                }
            });
            self.shaped
                .set(shaped)
                .map_err(|_| "native outline initialized twice")?;
        }
        Ok(self.shaped.get().expect("initialized native outline"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn variation_key_clones_share_and_charge_their_immutable_payload() {
        let font = Font::default();
        let plain = key(&font, "same text".into(), &Options::new(18.)).unwrap();
        let mut options = Options::new(18.);
        options.variations = vec![("wdth".into(), 90.), ("opsz".into(), 24.)];
        let varied = key(&font, "same text".into(), &options).unwrap();
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
    #[test]
    fn constrained_wrap_never_fabricates_hyphens_and_shares_source_storage() {
        let font = Font::default();
        let text: Arc<str> = "A".repeat(100).into();
        let mut options = Options::new(14.);
        options.width = Some(80.);
        options.wrap = pango::WrapMode::WordChar;
        let first_key = key(&font, text.clone(), &options).unwrap();
        options.width = Some(120.);
        let second_key = key(&font, text.clone(), &options).unwrap();
        assert!(Arc::ptr_eq(
            first_key.source.text(),
            second_key.source.text()
        ));
        let native = layout_shared(&font, &Source::new(text.clone()).unwrap(), &options).unwrap();
        let mut runs = native.layout().iter();
        let mut glyphs = 0;
        loop {
            if let Some(run) = runs.run_readonly() {
                glyphs += run.glyph_string().glyph_info().len();
            }
            if !runs.next_run() {
                break;
            }
        }
        assert_eq!(
            glyphs,
            text.len(),
            "emergency line breaks must not insert visible hyphens"
        );
        assert!(native.layout().line_count() > 1);
        assert!(Arc::ptr_eq(
            native.shaped().unwrap(),
            layout_shared(&font, &Source::new(text).unwrap(), &options)
                .unwrap()
                .shaped()
                .unwrap()
        ));
    }

    #[test]
    fn shared_cache_keys_hash_fixed_metadata_and_measurement_defers_outlines() {
        #[derive(Default)]
        struct Counter(usize);
        impl Hasher for Counter {
            fn finish(&self) -> u64 {
                self.0 as u64
            }
            fn write(&mut self, bytes: &[u8]) {
                self.0 += bytes.len();
            }
        }
        let font = Font::default();
        let options = Options::new(14.);
        let short = key(&font, Arc::from("small"), &options).unwrap();
        let large = key(&font, Arc::from("a".repeat(50_000)), &options).unwrap();
        let mut a = Counter::default();
        let mut b = Counter::default();
        short.hash(&mut a);
        large.hash(&mut b);
        assert_eq!(
            a.0, b.0,
            "retained source hashing must not depend on byte length"
        );
        assert!(short == short.clone());
        let other = key(&font, Arc::from("small"), &options).unwrap();
        assert!(
            short == other,
            "equal content from independent allocations must share one cache domain"
        );
        let layout = layout_shared(
            &font,
            &Source::new(Arc::from("only metrics are needed")).unwrap(),
            &options,
        )
        .unwrap();
        assert!(layout.metrics.advance > 0.);
        assert!(
            layout.shaped.get().is_none(),
            "paragraph measurement must build no unused outline"
        );
        assert!(!layout.shaped().unwrap().outline.is_empty());
        assert!(layout.shaped.get().is_some());
    }

    #[test]
    fn new_default_options_reuse_shared_layout_and_empty_style_metadata() {
        let font = Font::default();
        let source = Source::new(Arc::from("same retained label")).unwrap();
        let options = || {
            let mut options = Options::new(14.);
            options.width = Some(120.);
            options
        };
        let a = layout_shared(&font, &source, &options()).unwrap();
        let b = layout_shared(&font, &source, &options()).unwrap();
        assert!(Rc::ptr_eq(&a, &b));
        assert!(Arc::ptr_eq(a.shaped().unwrap(), b.shaped().unwrap()));
    }

    #[test]
    fn independent_sources_and_regular_native_calls_share_the_same_artifacts() {
        let font = Font::default();
        let mut options = Options::new(16.);
        options.width = Some(120.);
        let first = Source::new(Arc::from("shared across views")).unwrap();
        let second = Source::new(Arc::from("shared across views")).unwrap();
        assert!(!Arc::ptr_eq(first.text(), second.text()));
        let a = layout_shared(&font, &first, &options).unwrap();
        let b = layout_shared(&font, &second, &options).unwrap();
        assert!(
            Rc::ptr_eq(&a, &b),
            "independent equal sources must share native metrics"
        );
        let ordinary = shape(&font, "shared across views", &options).unwrap();
        assert!(Arc::ptr_eq(a.shaped().unwrap(), &ordinary));
        assert!(Rc::ptr_eq(
            &a,
            &layout(&font, "shared across views", &options).unwrap()
        ));
        let mut collision = Source::new(Arc::from("different contents")).unwrap();
        collision.text_hash = first.text_hash;
        collision.spans_hash = first.spans_hash;
        assert!(
            first != collision,
            "fingerprint collisions must still compare actual source contents"
        );
    }
}
