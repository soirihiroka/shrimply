//! Shared native still-media preparation for editor previews, paint, and Sanim.
use shrimply_resource_pipeline::cache::Cache;
use skia_safe::{Canvas, ConditionallySend, Data, FontMgr, Image, Paint, Rect, Sendable, svg::Dom};
use std::sync::{Arc, Mutex, OnceLock, Weak};

pub mod animation;
mod gif;
mod heif;
mod jxl;
mod raster;

const MAX_CACHE_BYTES: usize = 128 * 1024 * 1024;
const MAX_ENTRIES: usize = 128;
pub const MAX_SOURCE_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_PIXELS: usize = 67_108_864;
const DEFAULT_SVG_SIZE: [u32; 2] = [300, 150];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Bitmap,
    Svg,
}
enum Content {
    Bitmap(Image),
    Svg(Mutex<Option<Sendable<Dom>>>),
}
enum Source {
    Encoded { _bytes: Arc<Vec<u8>> },
    Rgba { _pixels: Arc<[u8]> },
}
#[derive(Clone, PartialEq, Eq, Hash)]
enum Key {
    Encoded(String),
    Rgba { identity: usize, size: [u32; 2] },
}
fn cache() -> &'static Mutex<Cache<Key, Prepared>> {
    static CACHE: OnceLock<Mutex<Cache<Key, Prepared>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Cache::new(MAX_CACHE_BYTES, MAX_ENTRIES)))
}
pub struct Prepared {
    content: Content,
    size: [u32; 2],
    _source: Source,
    retained_bytes: usize,
    font_epoch: Option<shrimply_fonts::native::FontEpoch>,
}
impl Prepared {
    pub fn kind(&self) -> Kind {
        match &self.content {
            Content::Bitmap(_) => Kind::Bitmap,
            Content::Svg(_) => Kind::Svg,
        }
    }
    pub fn dimensions(&self) -> [u32; 2] {
        self.size
    }
    pub fn retained_bytes(&self) -> usize {
        self.retained_bytes
    }
    pub fn bitmap(&self) -> Option<&Image> {
        match &self.content {
            Content::Bitmap(image) => Some(image),
            _ => None,
        }
    }
    /// Access thread-affine SVG internals while the cache holds exclusive ownership.
    ///
    /// # Safety
    /// The callback must not retain or return DOM/node clones. Such references
    /// would outlive this lock and invalidate Skia's conditional-send ownership.
    pub unsafe fn with_svg_dom<T>(&self, operation: impl FnOnce(&Dom) -> T) -> Result<T, String> {
        let Content::Svg(stored) = &self.content else {
            return Err("media is not an SVG".into());
        };
        let mut stored = stored.lock().expect("shared SVG mutex poisoned");
        let dom = stored.take().expect("shared SVG disappeared").into_inner();
        let result = operation(&dom);
        *stored = Some(
            dom.wrap_send()
                .unwrap_or_else(|_| panic!("shared SVG escaped its native cache")),
        );
        Ok(result)
    }
    /// Hit testing for an image element. Bitmap alpha is read from the retained
    /// CPU raster, never from a GPU. SVG uses its external-element viewport.
    pub fn contains(&self, point: [f32; 2]) -> bool {
        if point
            .iter()
            .any(|value| !value.is_finite() || *value < 0. || *value >= 1.)
        {
            return false;
        }
        let Some(image) = self.bitmap() else {
            return true;
        };
        let info = skia_safe::ImageInfo::new(
            (1, 1),
            skia_safe::ColorType::RGBA8888,
            skia_safe::AlphaType::Unpremul,
            None,
        );
        let mut pixel = [0u8; 4];
        image.read_pixels(
            &info,
            &mut pixel,
            4,
            (
                (point[0] * self.size[0] as f32) as i32,
                (point[1] * self.size[1] as f32) as i32,
            ),
            skia_safe::image::CachingHint::Allow,
        ) && pixel[3] != 0
    }
    /// Rasterize a bounded, color-managed preview, filtering bitmap sources
    /// and drawing vectors directly at their final pixel size.
    pub fn thumbnail(
        &self,
        max_edge: u32,
        color_type: skia_safe::ColorType,
        color_space: skia_safe::ColorSpace,
    ) -> Result<skia_safe::Surface, String> {
        let [width, height] = shrimply_math_geometry::fit_pixel_dimensions(self.size, max_edge);
        let info = skia_safe::ImageInfo::new(
            (width as i32, height as i32),
            color_type,
            skia_safe::AlphaType::Premul,
            color_space,
        );
        let mut surface =
            skia_safe::surfaces::raster(&info, None, None).ok_or("Allocate image preview")?;
        let bounds = Rect::from_wh(width as f32, height as f32);
        if let Some(image) = self.bitmap() {
            surface.canvas().draw_image_rect_with_sampling_options(
                image,
                None,
                bounds,
                skia_safe::FilterMode::Linear,
                &Paint::default(),
            );
        } else {
            self.draw(surface.canvas(), &bounds, 1.)?;
        }
        Ok(surface)
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
                if opacity != 1. {
                    canvas.save_layer_alpha_f(None, opacity);
                }
                // The callback only draws; all native node handles drop before it returns.
                unsafe {
                    self.with_svg_dom(|dom| {
                        let mut root = dom.root();
                        root.set_width(skia_safe::svg::Length::new(
                            destination.width(),
                            skia_safe::svg::LengthUnit::PX,
                        ));
                        root.set_height(skia_safe::svg::Length::new(
                            destination.height(),
                            skia_safe::svg::LengthUnit::PX,
                        ));
                        dom.render(canvas);
                    })?;
                }
                canvas.restore_to_count(save);
            }
        }
        Ok(())
    }
}
pub use shrimply_resource_pipeline::fingerprint::digest as fingerprint;
fn verify_fingerprint(bytes: &Arc<Vec<u8>>, expected: &str) -> Result<(), String> {
    struct Source {
        bytes: Weak<Vec<u8>>,
        fingerprint: String,
    }
    static SOURCES: OnceLock<Mutex<Cache<usize, Source>>> = OnceLock::new();
    const MAX_SOURCE_HINT_BYTES: usize = 256 * 1024;
    const MAX_SOURCE_HINTS: usize = 1024;
    let sources =
        SOURCES.get_or_init(|| Mutex::new(Cache::new(MAX_SOURCE_HINT_BYTES, MAX_SOURCE_HINTS)));
    let identity = Arc::as_ptr(bytes) as usize;
    if let Some(source) = sources
        .lock()
        .expect("media source identity cache poisoned")
        .get(&identity)
        && source.fingerprint == expected
        && source
            .bytes
            .upgrade()
            .is_some_and(|stored| Arc::ptr_eq(&stored, bytes))
    {
        return Ok(());
    }
    let actual = fingerprint(bytes);
    if actual != expected {
        return Err("media fingerprint does not match its immutable source bytes".into());
    }
    let size = std::mem::size_of::<Source>() + actual.len();
    sources
        .lock()
        .expect("media source identity cache poisoned")
        .insert(
            identity,
            Arc::new(Source {
                bytes: Arc::downgrade(bytes),
                fingerprint: actual,
            }),
            size,
        );
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
pub fn prepare_hashed_with_limit(
    bytes: &Arc<Vec<u8>>,
    fingerprint: &str,
    max_pixels: usize,
) -> Result<Arc<Prepared>, String> {
    if bytes.len() > MAX_SOURCE_BYTES {
        return Err("image source exceeds its byte budget".into());
    }
    verify_fingerprint(bytes, fingerprint)?;
    let key = Key::Encoded(fingerprint.to_owned());
    let cache = cache();
    let cached = cache.lock().expect("shared media cache poisoned").get(&key);
    if let Some(prepared) = cached {
        validate_size(prepared.size, max_pixels)?;
        if prepared.kind() == Kind::Bitmap
            || prepared.font_epoch == shrimply_fonts::native::system_font_epoch()
        {
            return Ok(prepared);
        }
    }
    let mut font_epoch = None;
    let (content, size) = if jxl::recognizes(bytes) {
        let decoder = jxl::JpegXl::open(bytes, max_pixels)?;
        let size = decoder.dimensions();
        (Content::Bitmap(decoder.frame(0)?), size)
    } else if let Some(format) = raster::format(bytes) {
        let (image, size) = raster::decode(bytes, format, max_pixels)?;
        (Content::Bitmap(image), size)
    } else if bytes.get(4..8) == Some(b"ftyp") {
        let (image, size) = heif::decode(bytes, max_pixels)?;
        (Content::Bitmap(image), size)
    } else if let Some(mut codec) = skia_safe::Codec::from_data(Data::new_copy(bytes)) {
        let mut info = codec.info();
        if info.alpha_type() != skia_safe::AlphaType::Opaque {
            info = info.with_alpha_type(skia_safe::AlphaType::Unpremul);
        }
        let origin = codec.origin();
        validate_size([info.width() as u32, info.height() as u32], max_pixels)?;
        let mut bitmap = skia_safe::Bitmap::new();
        if !bitmap.try_alloc_pixels_flags(&info) {
            return Err("could not allocate decoded image".into());
        }
        let result = {
            let mut pixels = bitmap.peek_pixels().ok_or("decoded image has no pixels")?;
            let stride = pixels.row_bytes();
            codec.get_pixels_with_options(
                &info,
                pixels
                    .bytes_mut()
                    .ok_or("decoded image has no writable pixels")?,
                stride,
                None,
            )
        };
        // get_image() accepts incomplete/corrupt input and fills missing pixels.
        // Import and preview must only publish a complete decoded bitmap.
        if result != skia_safe::codec::Result::Success {
            return Err(format!("image cannot be decoded completely: {result:?}"));
        }
        let (image, size) = bitmap_image(orient(bitmap, origin)?);
        (Content::Bitmap(image), size)
    } else {
        font_epoch = shrimply_fonts::native::system_font_epoch();
        let dom = Dom::from_bytes(bytes, FontMgr::new())
            .map_err(|_| "source is neither a supported bitmap nor a valid SVG")?;
        let root = dom.root();
        let intrinsic = shrimply_math_geometry::intrinsic_size(
            [
                shrimply_math_geometry::svg_length_px(*root.width()),
                shrimply_math_geometry::svg_length_px(*root.height()),
            ],
            root.view_box()
                .map(|rect| glam::vec2(rect.width(), rect.height())),
            glam::vec2(DEFAULT_SVG_SIZE[0] as f32, DEFAULT_SVG_SIZE[1] as f32),
        );
        if !intrinsic.is_finite()
            || intrinsic.x <= 0.
            || intrinsic.y <= 0.
            || intrinsic.x > u32::MAX as f32
            || intrinsic.y > u32::MAX as f32
        {
            return Err("SVG has invalid intrinsic dimensions".into());
        }
        let size = [intrinsic.x.ceil() as u32, intrinsic.y.ceil() as u32];
        validate_size(size, max_pixels)?;
        drop(root);
        (
            Content::Svg(Mutex::new(Some(
                dom.wrap_send().map_err(|_| "SVG was not uniquely owned")?,
            ))),
            size,
        )
    };
    let retained = bytes
        .len()
        .saturating_add(match &content {
            Content::Bitmap(image) => image.image_info().compute_min_byte_size(),
            Content::Svg(_) => bytes.len().saturating_mul(8),
        })
        .saturating_add(std::mem::size_of::<Prepared>() + fingerprint.len());
    let prepared = Arc::new(Prepared {
        content,
        size,
        _source: Source::Encoded {
            _bytes: bytes.clone(),
        },
        retained_bytes: retained,
        font_epoch,
    });
    cache
        .lock()
        .expect("shared media cache poisoned")
        .insert(key, prepared.clone(), retained);
    Ok(prepared)
}

/// Prepare immutable straight-alpha RGBA pixels in the same bounded media cache.
/// A raster is copied into owned Skia storage once; no PNG encode/decode or GPU
/// readback is involved. Holding its source proves pointer-key identity safely.
pub fn prepare_rgba(pixels: &Arc<[u8]>, size: [u32; 2]) -> Result<Arc<Prepared>, String> {
    validate_size(size, MAX_PIXELS)?;
    let expected = (size[0] as usize)
        .checked_mul(size[1] as usize)
        .and_then(|count| count.checked_mul(4))
        .ok_or("RGBA source dimensions overflow")?;
    if pixels.len() != expected || expected > MAX_SOURCE_BYTES {
        return Err(
            "RGBA source does not match its dimensions or exceeds the media byte budget".into(),
        );
    }
    let retained = expected
        .saturating_mul(2)
        .saturating_add(std::mem::size_of::<Prepared>() + std::mem::size_of::<Key>());
    if retained > MAX_CACHE_BYTES {
        return Err("RGBA source and native raster exceed the shared media cache budget; reduce its resolution".into());
    }
    let key = Key::Rgba {
        identity: Arc::as_ptr(pixels) as *const u8 as usize,
        size,
    };
    let cache = cache();
    if let Some(prepared) = cache.lock().expect("shared media cache poisoned").get(&key) {
        return Ok(prepared);
    }
    let width = i32::try_from(size[0]).map_err(|_| "RGBA width exceeds the native image range")?;
    let height =
        i32::try_from(size[1]).map_err(|_| "RGBA height exceeds the native image range")?;
    let info = skia_safe::ImageInfo::new(
        (width, height),
        skia_safe::ColorType::RGBA8888,
        skia_safe::AlphaType::Unpremul,
        skia_safe::ColorSpace::new_srgb(),
    );
    let image =
        skia_safe::images::raster_from_data(&info, Data::new_copy(pixels), size[0] as usize * 4)
            .ok_or("could not prepare the immutable RGBA raster")?;
    let prepared = Arc::new(Prepared {
        content: Content::Bitmap(image),
        size,
        _source: Source::Rgba {
            _pixels: pixels.clone(),
        },
        retained_bytes: retained,
        font_epoch: None,
    });
    cache
        .lock()
        .expect("shared media cache poisoned")
        .insert(key, prepared.clone(), retained);
    Ok(prepared)
}
fn validate_size(size: [u32; 2], max_pixels: usize) -> Result<(), String> {
    if size[0] == 0
        || size[1] == 0
        || (size[0] as usize)
            .checked_mul(size[1] as usize)
            .is_none_or(|pixels| pixels > max_pixels.min(MAX_PIXELS))
    {
        return Err("image dimensions exceed the pixel budget".into());
    }
    Ok(())
}

/// Lossless storage of a completed, straight-alpha RGBA frame.
pub fn encode_rgba_png(width: i32, height: i32, pixels: &[u8]) -> Result<Vec<u8>, String> {
    let info = skia_safe::ImageInfo::new(
        (width, height),
        skia_safe::ColorType::RGBA8888,
        skia_safe::AlphaType::Unpremul,
        None,
    );
    let stride = usize::try_from(width)
        .map_err(|_| "Invalid captured image width")?
        .checked_mul(std::mem::size_of::<u32>())
        .ok_or("Captured image is too large")?;
    if width <= 0 || height <= 0 || info.compute_byte_size(stride) != pixels.len() {
        return Err("Captured image has an invalid RGBA layout".into());
    }
    let image =
        skia_safe::images::raster_from_data(&info, skia_safe::Data::new_copy(pixels), stride)
            .ok_or("Could not construct the captured image")?;
    image
        .encode(None, skia_safe::EncodedImageFormat::PNG, None)
        .map(|data| data.as_bytes().to_vec())
        .ok_or_else(|| "Could not encode the captured image".into())
}

pub fn rgba_pixels(image: &Image) -> Result<Vec<u8>, String> {
    let width = usize::try_from(image.width()).map_err(|_| "Image width is negative")?;
    let height = usize::try_from(image.height()).map_err(|_| "Image height is negative")?;
    let row_bytes = width
        .checked_mul(size_of::<u32>())
        .ok_or("Image row size overflow")?;
    let mut pixels = vec![0; row_bytes.checked_mul(height).ok_or("Image size overflow")?];
    let info = skia_safe::ImageInfo::new(
        image.dimensions(),
        skia_safe::ColorType::RGBA8888,
        skia_safe::AlphaType::Unpremul,
        skia_safe::ColorSpace::new_srgb(),
    );
    if !image.read_pixels(
        &info,
        &mut pixels,
        row_bytes,
        (0, 0),
        skia_safe::image::CachingHint::Allow,
    ) {
        return Err("Could not read RGBA image pixels".into());
    }
    Ok(pixels)
}

fn orient(
    mut bitmap: skia_safe::Bitmap,
    origin: skia_safe::EncodedOrigin,
) -> Result<skia_safe::Bitmap, String> {
    if origin != skia_safe::EncodedOrigin::TopLeft {
        let output_info = if origin.swaps_width_height() {
            skia_safe::codec::pixmap_utils::swap_width_height(bitmap.info())
        } else {
            bitmap.info().clone()
        };
        let mut oriented = skia_safe::Bitmap::new();
        if !oriented.try_alloc_pixels_flags(&output_info) {
            return Err("could not allocate oriented image".into());
        }
        {
            let source = bitmap.peek_pixels().ok_or("decoded image has no pixels")?;
            let mut target = oriented
                .peek_pixels()
                .ok_or("oriented image has no pixels")?;
            // Both pixmaps borrow uniquely owned, initialized local bitmap storage.
            if !unsafe { skia_safe::codec::pixmap_utils::orient(&mut target, &source, origin) } {
                return Err("could not apply image orientation".into());
            }
        }
        bitmap = oriented;
    }
    Ok(bitmap)
}

fn bitmap_image(mut bitmap: skia_safe::Bitmap) -> (Image, [u32; 2]) {
    bitmap.set_immutable();
    (
        bitmap.as_image(),
        [bitmap.width() as u32, bitmap.height() as u32],
    )
}
