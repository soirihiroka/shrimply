//! Shared native still-media preparation for editor previews, paint, and Sanim.
use shrimply_resource_pipeline::cache::Cache;
use skia_safe::{Canvas, ConditionallySend, Data, FontMgr, Image, Paint, Rect, Sendable, svg::Dom};
use std::sync::{Arc, Mutex, OnceLock, Weak};

const MAX_CACHE_BYTES: usize = 128 * 1024 * 1024;
const MAX_ENTRIES: usize = 128;
pub const MAX_SOURCE_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_PIXELS: usize = 67_108_864;
const DEFAULT_SVG_SIZE: [u32; 2] = [300, 150];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind { Bitmap, Svg }
enum Content { Bitmap(Image), Svg(Mutex<Option<Sendable<Dom>>>) }
pub struct Prepared {
    content: Content,
    size: [u32; 2],
    _source: Arc<Vec<u8>>,
    retained_bytes: usize,
    font_epoch: Option<shrimply_fonts::native::FontEpoch>,
}
impl Prepared {
    pub fn kind(&self) -> Kind {
        match &self.content { Content::Bitmap(_) => Kind::Bitmap, Content::Svg(_) => Kind::Svg }
    }
    pub fn dimensions(&self) -> [u32; 2] { self.size }
    pub fn retained_bytes(&self) -> usize { self.retained_bytes }
    pub fn bitmap(&self) -> Option<&Image> {
        match &self.content { Content::Bitmap(image) => Some(image), _ => None }
    }
    /// Access thread-affine SVG internals while the cache holds exclusive ownership.
    ///
    /// # Safety
    /// The callback must not retain or return DOM/node clones. Such references
    /// would outlive this lock and invalidate Skia's conditional-send ownership.
    pub unsafe fn with_svg_dom<T>(&self, operation: impl FnOnce(&Dom) -> T) -> Result<T, String> {
        let Content::Svg(stored) = &self.content else { return Err("media is not an SVG".into()) };
        let mut stored = stored.lock().expect("shared SVG mutex poisoned");
        let dom = stored.take().expect("shared SVG disappeared").into_inner();
        let result = operation(&dom);
        *stored = Some(dom.wrap_send().unwrap_or_else(|_| panic!("shared SVG escaped its native cache")));
        Ok(result)
    }
    /// Hit testing for an image element. Bitmap alpha is read from the retained
    /// CPU raster, never from a GPU. SVG uses its external-element viewport.
    pub fn contains(&self, point: [f32; 2]) -> bool {
        if point.iter().any(|value| !value.is_finite() || *value < 0. || *value >= 1.) { return false; }
        let Some(image) = self.bitmap() else { return true };
        let info = skia_safe::ImageInfo::new((1, 1), skia_safe::ColorType::RGBA8888, skia_safe::AlphaType::Unpremul, None);
        let mut pixel = [0u8; 4];
        image.read_pixels(&info, &mut pixel, 4, ((point[0] * self.size[0] as f32) as i32, (point[1] * self.size[1] as f32) as i32), skia_safe::image::CachingHint::Allow) && pixel[3] != 0
    }
    /// Replay vectors or a retained CPU-decoded image onto the destination canvas.
    /// GPU canvases upload images normally; SVG is never rasterized for replay.
    pub fn draw(&self, canvas: &Canvas, destination: &Rect, opacity: f32) -> Result<(), String> {
        match &self.content {
            Content::Bitmap(image) => {
                let mut paint = Paint::default();
                paint.set_alpha_f(opacity);
                canvas.draw_image_rect(image, None, destination, &paint);
            }
            Content::Svg(_) => {
                let save = canvas.save();
                canvas.translate((destination.left, destination.top));
                if opacity != 1. { canvas.save_layer_alpha_f(None, opacity); }
                // The callback only draws; all native node handles drop before it returns.
                unsafe { self.with_svg_dom(|dom| {
                    let mut root = dom.root();
                    root.set_width(skia_safe::svg::Length::new(destination.width(), skia_safe::svg::LengthUnit::PX));
                    root.set_height(skia_safe::svg::Length::new(destination.height(), skia_safe::svg::LengthUnit::PX));
                    dom.render(canvas);
                })?; }
                canvas.restore_to_count(save);
            }
        }
        Ok(())
    }
}
pub use shrimply_resource_pipeline::fingerprint::digest as fingerprint;
fn verify_fingerprint(bytes: &Arc<Vec<u8>>, expected: &str) -> Result<(), String> {
    struct Source { bytes: Weak<Vec<u8>>, fingerprint: String }
    static SOURCES: OnceLock<Mutex<Cache<usize, Source>>> = OnceLock::new();
    const MAX_SOURCE_HINT_BYTES: usize = 256 * 1024;
    const MAX_SOURCE_HINTS: usize = 1024;
    let sources = SOURCES.get_or_init(|| Mutex::new(Cache::new(MAX_SOURCE_HINT_BYTES, MAX_SOURCE_HINTS)));
    let identity = Arc::as_ptr(bytes) as usize;
    if let Some(source) = sources.lock().expect("media source identity cache poisoned").get(&identity)
        && source.fingerprint == expected
        && source.bytes.upgrade().is_some_and(|stored| Arc::ptr_eq(&stored, bytes)) { return Ok(()); }
    let actual = fingerprint(bytes);
    if actual != expected { return Err("media fingerprint does not match its immutable source bytes".into()); }
    let size = std::mem::size_of::<Source>() + actual.len();
    sources.lock().expect("media source identity cache poisoned").insert(identity, Arc::new(Source {bytes:Arc::downgrade(bytes),fingerprint:actual}), size);
    Ok(())
}
pub fn prepare(bytes: Arc<Vec<u8>>) -> Result<Arc<Prepared>, String> {
    let key = fingerprint(&bytes);
    prepare_hashed(&bytes, &key)
}
/// The caller must supply the SHA-256 of these immutable source bytes.
pub fn prepare_hashed(bytes: &Arc<Vec<u8>>, fingerprint: &str) -> Result<Arc<Prepared>, String> {
    prepare_hashed_with_limit(bytes, fingerprint, MAX_PIXELS)
}
pub fn prepare_hashed_with_limit(bytes: &Arc<Vec<u8>>, fingerprint: &str, max_pixels: usize) -> Result<Arc<Prepared>, String> {
    static CACHE: OnceLock<Mutex<Cache<String, Prepared>>> = OnceLock::new();
    if bytes.len() > MAX_SOURCE_BYTES { return Err("image source exceeds its byte budget".into()); }
    verify_fingerprint(bytes, fingerprint)?;
    let key = fingerprint.to_owned();
    let cache = CACHE.get_or_init(|| Mutex::new(Cache::new(MAX_CACHE_BYTES, MAX_ENTRIES)));
    let cached = cache.lock().expect("shared media cache poisoned").get(&key);
    if let Some(prepared) = cached {
        validate_size(prepared.size, max_pixels)?;
        if prepared.kind() == Kind::Bitmap || prepared.font_epoch == shrimply_fonts::native::system_font_epoch() {return Ok(prepared);}
    }
    let mut font_epoch=None;
    let (content, size) = if let Some(image) = Image::from_encoded(Data::new_copy(bytes)) {
        let width = u32::try_from(image.width()).map_err(|_| "invalid image width")?;
        let height = u32::try_from(image.height()).map_err(|_| "invalid image height")?;
        validate_size([width, height], max_pixels)?;
        let image = image.make_raster_image(None, skia_safe::image::CachingHint::Allow).ok_or("image cannot be decoded")?;
        (Content::Bitmap(image), [width, height])
    } else {
        font_epoch=shrimply_fonts::native::system_font_epoch();
        let dom = Dom::from_bytes(bytes, FontMgr::new()).map_err(|_| "source is neither a supported bitmap nor a valid SVG")?;
        let root = dom.root();
        let intrinsic = root.intrinsic_size();
        let intrinsic = if intrinsic.width > 0. && intrinsic.height > 0. { intrinsic }
            else { root.view_box().map(|rect| rect.size()).unwrap_or(skia_safe::Size::new(DEFAULT_SVG_SIZE[0] as f32, DEFAULT_SVG_SIZE[1] as f32)) };
        if !intrinsic.width.is_finite() || !intrinsic.height.is_finite() || intrinsic.width <= 0. || intrinsic.height <= 0. || intrinsic.width > u32::MAX as f32 || intrinsic.height > u32::MAX as f32 { return Err("SVG has invalid intrinsic dimensions".into()); }
        let size = [intrinsic.width.ceil() as u32, intrinsic.height.ceil() as u32];
        validate_size(size, max_pixels)?;
        drop(root);
        (Content::Svg(Mutex::new(Some(dom.wrap_send().map_err(|_| "SVG was not uniquely owned")?))), size)
    };
    let retained = bytes.len().saturating_add(match &content { Content::Bitmap(_) => size[0] as usize * size[1] as usize * 4, Content::Svg(_) => bytes.len().saturating_mul(8) }).saturating_add(std::mem::size_of::<Prepared>() + fingerprint.len());
    let prepared = Arc::new(Prepared {content, size, _source: bytes.clone(), retained_bytes: retained, font_epoch});
    cache.lock().expect("shared media cache poisoned").insert(key, prepared.clone(), retained);
    Ok(prepared)
}
fn validate_size(size: [u32; 2], max_pixels: usize) -> Result<(), String> {
    if size[0] == 0 || size[1] == 0 || (size[0] as usize).checked_mul(size[1] as usize).is_none_or(|pixels| pixels > max_pixels.min(MAX_PIXELS)) { return Err("image dimensions exceed the pixel budget".into()); }
    Ok(())
}
