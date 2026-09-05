use std::{
    cell::{Cell, RefCell},
    ops::Range,
    path::{Path as FsPath, PathBuf},
    rc::Rc,
    sync::{
        OnceLock, RwLock,
        atomic::{AtomicU64, Ordering},
    },
};

use cached::{Cached, stores::LruCache};
use cairo::PathSegment;
use glib::translate::from_glib;
use pango::prelude::FontMapExt;
use pango::{Alignment, FontColor, FontDescription, Gravity, GravityHint};
use skia_safe::{
    AlphaType, ColorType, Data, IRect, Image, ImageInfo, Matrix, Path, PathBuilder, Rect, Region,
};

use shrimply_project::project::{
    Color, FontFamily, TextDirection, TextFontStyle, TextHorizontalAlign, TextItem, VerticalAlign,
};

pub struct TextLayout {
    pub path: Path,
    pub subpaths: Vec<Path>,
    pub word_subpaths: Vec<Path>,
    pub mask_units: Vec<TextMaskUnit>,
    color_glyphs: Option<ColorGlyphLayout>,
    pub size: glam::Vec2,
}

pub struct TextMaskUnit {
    pub path: Path,
}

const MAX_LAYOUT_CACHE_ENTRIES: usize = 1024;
const MAX_COLOR_GLYPH_CACHE_ENTRIES: usize = 8;
const PANGO_GLYPH_IS_COLOR_BIT: u32 = 1 << 1;

struct ColorGlyphLayout {
    layout: pango::Layout,
    direction: TextDirection,
    clips: Vec<Rect>,
    min_x: f64,
    min_y: f64,
    offset: glam::Vec2,
    size: glam::Vec2,
    silhouette: Path,
    images: RefCell<LruCache<u32, Rc<Image>>>,
}

pub struct ColorGlyphImage {
    pub image: Rc<Image>,
    pub offset: glam::Vec2,
    pub silhouette: Path,
}

impl TextLayout {
    pub fn color_glyphs(&self, color: Color<u8>) -> Option<ColorGlyphImage> {
        let color = Color {
            a: u8::MAX,
            ..color
        };
        self.color_glyphs.as_ref().map(|glyphs| ColorGlyphImage {
            image: glyphs.image(color),
            offset: glyphs.offset,
            silhouette: glyphs.silhouette.clone(),
        })
    }
}

impl ColorGlyphLayout {
    fn image(&self, color: Color<u8>) -> Rc<Image> {
        let key = color.to_rgba_u32();
        if let Some(image) = self.images.borrow_mut().cache_get(&key).map(Rc::clone) {
            return image;
        }

        let (pixels, width, height) = self.rasterize(color);
        let image = Rc::new(
            skia_safe::images::raster_from_data(
                &ImageInfo::new(
                    (width, height),
                    ColorType::RGBA8888,
                    AlphaType::Premul,
                    None,
                ),
                Data::new_copy(&pixels),
                width as usize * 4,
            )
            .expect("create native color glyph image"),
        );
        self.images.borrow_mut().cache_set(key, Rc::clone(&image));
        image
    }

    fn silhouette(&self) -> Path {
        let (pixels, width, height) = self.rasterize(Color::<u8>::WHITE);
        let mut spans = Vec::new();
        for (y, row) in pixels
            .chunks_exact(width as usize * 4)
            .take(height as usize)
            .enumerate()
        {
            let mut x = 0;
            while x < width as usize {
                if row[x * 4 + 3] == 0 {
                    x += 1;
                    continue;
                }
                let start = x;
                while x < width as usize && row[x * 4 + 3] != 0 {
                    x += 1;
                }
                spans.push(IRect::from_xywh(
                    start as i32,
                    y as i32,
                    (x - start) as i32,
                    1,
                ));
            }
        }
        let mut region = Region::new();
        if spans.is_empty() || !region.set_rects(&spans) {
            return Path::default();
        }
        region
            .boundary_path()
            .map(|path| path.with_transform(&Matrix::translate((self.offset.x, self.offset.y))))
            .unwrap_or_default()
    }

    fn rasterize(&self, color: Color<u8>) -> (Vec<u8>, i32, i32) {
        let width = self.size.x.ceil().max(1.0) as i32;
        let height = self.size.y.ceil().max(1.0) as i32;
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, width, height)
            .expect("create native color glyph surface");
        let cairo = cairo::Context::new(&surface).expect("create native color glyph context");
        for clip in &self.clips {
            cairo.rectangle(
                f64::from(clip.left - self.offset.x),
                f64::from(clip.top - self.offset.y),
                f64::from(clip.width()),
                f64::from(clip.height()),
            );
        }
        cairo.clip();
        cairo.translate(
            -self.min_x - f64::from(self.offset.x),
            -self.min_y - f64::from(self.offset.y),
        );
        if self.direction == TextDirection::Vertical {
            cairo.rotate(std::f64::consts::FRAC_PI_2);
        }
        cairo.set_source_rgba(
            f64::from(color.r) / 255.0,
            f64::from(color.g) / 255.0,
            f64::from(color.b) / 255.0,
            f64::from(color.a) / 255.0,
        );
        cairo.move_to(0.0, 0.0);
        pangocairo::functions::show_layout(&cairo, &self.layout);
        drop(cairo);
        surface.flush();

        let stride = surface.stride() as usize;
        let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
        surface
            .with_data(|data| {
                for row in data.chunks_exact(stride).take(height as usize) {
                    for pixel in row.chunks_exact(4).take(width as usize) {
                        let argb = u32::from_ne_bytes(pixel.try_into().expect("ARGB32 pixel"));
                        pixels.extend_from_slice(&[
                            (argb >> 16) as u8,
                            (argb >> 8) as u8,
                            argb as u8,
                            (argb >> 24) as u8,
                        ]);
                    }
                }
            })
            .expect("read native color glyph pixels");
        (pixels, width, height)
    }
}

static APPLICATION_FONT_REVISION: AtomicU64 = AtomicU64::new(0);
static APPLICATION_FONT_FILES: OnceLock<RwLock<Vec<PathBuf>>> = OnceLock::new();

thread_local! {
    static LAYOUTS: RefCell<LruCache<Key, Rc<TextLayout>>> = RefCell::new(
        LruCache::builder()
            .max_size(MAX_LAYOUT_CACHE_ENTRIES)
            .build()
            .expect("valid Pango text layout cache size"),
    );
    static REGISTERED_FONT_REVISION: Cell<u64> = const { Cell::new(0) };
}

pub fn register_application_font(path: impl AsRef<FsPath>) -> Result<(), String> {
    let path = path.as_ref();
    shrimply_text_3d::register_application_font(path)?;
    pangocairo::FontMap::default()
        .add_font_file(path)
        .map_err(|error| format!("could not register font {}: {error}", path.display()))?;
    let fonts = APPLICATION_FONT_FILES.get_or_init(|| RwLock::new(Vec::new()));
    let mut fonts = fonts
        .write()
        .unwrap_or_else(|_| panic!("application font registry lock died"));
    if fonts.iter().any(|registered| registered == path) {
        return Ok(());
    }
    fonts.push(path.to_path_buf());
    let revision = APPLICATION_FONT_REVISION.fetch_add(1, Ordering::AcqRel) + 1;
    REGISTERED_FONT_REVISION.set(revision);
    LAYOUTS.with(|layouts| layouts.borrow_mut().cache_clear());
    Ok(())
}

pub fn layout(
    text: &TextItem,
    content: &str,
    font_size: f32,
    font_weight: f32,
    tracking: f32,
    line_height: f32,
    time: shrimply_project::project::Time,
) -> Rc<TextLayout> {
    register_pending_application_fonts();
    let key = Key {
        text: content.to_string(),
        h_align: text.h_align.value_at(time),
        v_align: text.v_align.value_at(time),
        direction: text.direction.value_at(time),
        font_families: text.font_families.clone(),
        font_style: text.font_style.value_at(time),
        font_variations: normalized_variations(text),
        font_size: font_size.to_bits(),
        font_weight: font_weight.round().clamp(1.0, 1000.0) as i32,
        tracking: tracking.to_bits(),
        line_height: line_height.to_bits(),
    };
    if let Some(layout) =
        LAYOUTS.with(|layouts| layouts.borrow_mut().cache_get(&key).map(Rc::clone))
    {
        return layout;
    }

    let layout = Rc::new(build(
        text,
        content,
        font_size,
        font_weight,
        tracking,
        line_height,
        time,
    ));
    LAYOUTS.with(|layouts| {
        layouts.borrow_mut().cache_set(key, Rc::clone(&layout));
    });
    layout
}

fn register_pending_application_fonts() {
    let revision = APPLICATION_FONT_REVISION.load(Ordering::Acquire);
    if REGISTERED_FONT_REVISION.get() == revision {
        return;
    }
    let Some(fonts) = APPLICATION_FONT_FILES.get() else {
        REGISTERED_FONT_REVISION.set(revision);
        return;
    };
    let fonts = fonts
        .read()
        .unwrap_or_else(|_| panic!("application font registry lock died"));
    let font_map = pangocairo::FontMap::default();
    for path in fonts.iter() {
        if let Err(error) = font_map.add_font_file(path) {
            tracing::warn!(path = %path.display(), "Could not register application font: {error}");
        }
    }
    REGISTERED_FONT_REVISION.set(revision);
    LAYOUTS.with(|layouts| layouts.borrow_mut().cache_clear());
}

#[derive(Clone, Eq, Hash, PartialEq)]
struct Key {
    text: String,
    h_align: TextHorizontalAlign,
    v_align: VerticalAlign,
    direction: TextDirection,
    font_families: Vec<FontFamily>,
    font_style: TextFontStyle,
    font_variations: Vec<(String, u32)>,
    font_size: u32,
    font_weight: i32,
    tracking: u32,
    line_height: u32,
}

fn build(
    text: &TextItem,
    content: &str,
    font_size: f32,
    font_weight: f32,
    tracking: f32,
    line_height: f32,
    time: shrimply_project::project::Time,
) -> TextLayout {
    let direction = text.direction.value_at(time);
    let h_align = text.h_align.value_at(time);
    let v_align = text.v_align.value_at(time);
    let font_style = text.font_style.value_at(time);
    let surface = cairo::RecordingSurface::create(cairo::Content::Alpha, None)
        .expect("create text path surface");
    let cairo = cairo::Context::new(&surface).expect("create text layout context");
    if direction == TextDirection::Vertical {
        cairo.rotate(std::f64::consts::FRAC_PI_2);
    }

    let layout = pangocairo::functions::create_layout(&cairo);
    let context = layout.context();
    context.set_base_gravity(match direction {
        TextDirection::Horizontal => Gravity::South,
        TextDirection::Vertical => Gravity::East,
    });
    context.set_gravity_hint(GravityHint::Natural);

    let mut font = FontDescription::new();
    let font_families = text
        .font_families
        .iter()
        .map(FontFamily::name)
        .collect::<Vec<_>>();
    if !font_families.is_empty() {
        font.set_family(&font_families.join(", "));
    }
    font.set_color(FontColor::DontCare);
    font.set_style(match font_style {
        TextFontStyle::Normal => pango::Style::Normal,
        TextFontStyle::Italic => pango::Style::Italic,
        TextFontStyle::Oblique => pango::Style::Oblique,
    });
    font.set_absolute_size(font_size.max(1.0) as f64 * pango::SCALE as f64);
    font.set_weight(unsafe { from_glib(font_weight.round().clamp(1.0, 1000.0) as i32) });
    let mut variations = normalized_variations(text)
        .into_iter()
        .map(|(axis, value)| format!("{axis}={}", f32::from_bits(value)))
        .collect::<Vec<_>>();
    variations.push(format!("wght={}", font_weight.clamp(1.0, 1000.0)));
    if font_style == TextFontStyle::Italic
        && !variations
            .iter()
            .any(|variation| variation.starts_with("ital="))
    {
        variations.push("ital=1".to_string());
    }
    font.set_variations(Some(&variations.join(",")));
    layout.set_font_description(Some(&font));
    layout.set_text(if content.is_empty() { " " } else { content });
    if tracking != 0.0 {
        let attrs = pango::AttrList::new();
        attrs.insert(pango::AttrInt::new_letter_spacing(
            (tracking * pango::SCALE as f32).round() as i32,
        ));
        layout.set_attributes(Some(&attrs));
    }
    if line_height != 1.0 {
        layout.set_line_spacing(line_height.max(f32::EPSILON));
    }

    let (_, natural) = layout.extents();
    layout.set_width(natural.width().max(1));
    let alignment = match direction {
        TextDirection::Horizontal => match h_align {
            TextHorizontalAlign::Left | TextHorizontalAlign::Fill => Alignment::Left,
            TextHorizontalAlign::Center => Alignment::Center,
            TextHorizontalAlign::Right => Alignment::Right,
        },
        TextDirection::Vertical => match v_align {
            VerticalAlign::Top => Alignment::Left,
            VerticalAlign::Middle => Alignment::Center,
            VerticalAlign::Bottom => Alignment::Right,
        },
    };
    layout.set_alignment(alignment);
    let justify = direction == TextDirection::Horizontal && h_align == TextHorizontalAlign::Fill;
    layout.set_justify(justify);
    layout.set_justify_last_line(justify);

    let color_ranges = color_glyph_ranges(&layout);
    let is_color = |index: usize| color_ranges.iter().any(|range| range.contains(&index));
    let mut color_cluster_rects = Vec::new();
    let mut cluster_iter = layout.iter();
    loop {
        if is_color(cluster_iter.index().max(0) as usize) {
            let (ink, logical) = cluster_iter.cluster_extents();
            color_cluster_rects.push(if ink.width() > 0 && ink.height() > 0 {
                ink
            } else {
                logical
            });
        }
        if !cluster_iter.next_cluster() {
            break;
        }
    }

    let mut word_indices = vec![None; content.len() + 1];
    let mut current_word = None;
    let mut word_count = 0;
    for (offset, character) in content.char_indices() {
        if character.is_whitespace() {
            current_word = None;
        } else {
            let index = *current_word.get_or_insert_with(|| {
                let index = word_count;
                word_count += 1;
                index
            });
            word_indices[offset] = Some(index);
        }
    }

    let mut character_rects = Vec::new();
    let mut iter = layout.iter();
    loop {
        let rect = iter.char_extents();
        if rect.width() > 0 && rect.height() > 0 {
            let word_index = word_indices
                .get(iter.index().max(0) as usize)
                .copied()
                .flatten();
            character_rects.push((rect, word_index, is_color(iter.index().max(0) as usize)));
        }
        if !iter.next_char() {
            break;
        }
    }

    let (ink, logical) = layout.extents();
    cairo.move_to(0.0, 0.0);
    pangocairo::functions::layout_path(&cairo, &layout);
    cairo.identity_matrix();
    let path_bounds = cairo.path_extents().expect("measure text path");
    let logical_bounds = pango_rect_bounds(&logical, direction);
    let ink_bounds = pango_rect_bounds(&ink, direction);
    let min_x = logical_bounds.0.min(ink_bounds.0).min(path_bounds.0);
    let min_y = logical_bounds.1.min(ink_bounds.1).min(path_bounds.1);
    let max_x = logical_bounds.2.max(ink_bounds.2).max(path_bounds.2);
    let max_y = logical_bounds.3.max(ink_bounds.3).max(path_bounds.3);

    let cairo_path = cairo.copy_path().expect("copy text path");
    let mut path = PathBuilder::new();
    let point = |(x, y): (f64, f64)| ((x - min_x) as f32, (y - min_y) as f32);
    for segment in cairo_path.iter() {
        match segment {
            PathSegment::MoveTo(to) => {
                path.move_to(point(to));
            }
            PathSegment::LineTo(to) => {
                path.line_to(point(to));
            }
            PathSegment::CurveTo(control_1, control_2, to) => {
                path.cubic_to(point(control_1), point(control_2), point(to));
            }
            PathSegment::ClosePath => {
                path.close();
            }
        }
    }

    let base_path: Path = path.into();
    let character_rects = character_rects
        .into_iter()
        .map(|(rect, word_index, color)| CharacterRect {
            rect: normalized_pango_rect(&rect, direction, min_x, min_y),
            word_index,
            color,
        })
        .collect::<Vec<_>>();
    let color_clips = color_cluster_rects
        .iter()
        .map(|rect| normalized_pango_rect(rect, direction, min_x, min_y))
        .map(|rect| {
            Rect::from_ltrb(
                rect.left.floor(),
                rect.top.floor(),
                rect.right.ceil(),
                rect.bottom.ceil(),
            )
        })
        .collect::<Vec<_>>();
    let color_bounds = color_clips.first().copied().map(|mut bounds| {
        for clip in &color_clips[1..] {
            bounds.join(clip);
        }
        bounds
    });
    let mut color_glyphs = color_bounds.map(|bounds| ColorGlyphLayout {
        layout: layout.clone(),
        direction,
        clips: color_clips,
        min_x,
        min_y,
        offset: glam::Vec2::new(bounds.left, bounds.top),
        size: glam::Vec2::new(bounds.width(), bounds.height()),
        silhouette: Path::default(),
        images: RefCell::new(
            LruCache::builder()
                .max_size(MAX_COLOR_GLYPH_CACHE_ENTRIES)
                .build()
                .expect("valid native color glyph cache size"),
        ),
    });

    let mut subpaths = character_rects
        .iter()
        .map(|_| PathBuilder::new())
        .collect::<Vec<_>>();
    let mut word_subpaths = (0..word_count)
        .map(|_| PathBuilder::new())
        .collect::<Vec<_>>();
    let mut path = PathBuilder::new();
    for contour in crate::path::contours(&base_path) {
        let Some(index) = nearest_character(&contour, &character_rects, false) else {
            continue;
        };
        if character_rects[index].color {
            continue;
        }
        path.add_path(&contour, None);
        subpaths[index].add_path(&contour, None);
        if let Some(word_index) = character_rects[index].word_index {
            word_subpaths[word_index].add_path(&contour, None);
        }
    }

    if let Some(color_glyphs) = &mut color_glyphs {
        let silhouette = color_glyphs.silhouette();
        for contour in crate::path::contours(&silhouette) {
            let Some(index) = nearest_character(&contour, &character_rects, true) else {
                continue;
            };
            path.add_path(&contour, None);
            subpaths[index].add_path(&contour, None);
            if let Some(word_index) = character_rects[index].word_index {
                word_subpaths[word_index].add_path(&contour, None);
            }
        }
        color_glyphs.silhouette = silhouette;
    }
    let path = path.detach();
    let subpaths = subpaths
        .into_iter()
        .map(|mut builder| builder.detach())
        .collect::<Vec<_>>();
    let mask_units = subpaths
        .iter()
        .map(|path| TextMaskUnit { path: path.clone() })
        .collect();
    let subpaths = subpaths
        .into_iter()
        .filter(|path| !path.is_empty())
        .collect::<Vec<_>>();
    let word_subpaths = word_subpaths
        .into_iter()
        .map(|mut builder| builder.detach())
        .filter(|path| !path.is_empty())
        .collect::<Vec<_>>();

    TextLayout {
        subpaths: if subpaths.is_empty() {
            vec![path.clone()]
        } else {
            subpaths
        },
        word_subpaths: if word_subpaths.is_empty() {
            vec![path.clone()]
        } else {
            word_subpaths
        },
        mask_units,
        color_glyphs,
        path,
        size: glam::Vec2::new((max_x - min_x) as f32, (max_y - min_y) as f32),
    }
}

struct CharacterRect {
    rect: Rect,
    word_index: Option<usize>,
    color: bool,
}

fn color_glyph_ranges(layout: &pango::Layout) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut iter = layout.iter();
    loop {
        if let Some(run) = iter.run_readonly() {
            let item = run.item();
            let glyphs = run.glyph_string();
            let mut clusters = Vec::<(i32, bool)>::new();
            for (cluster, glyph) in glyphs.log_clusters().iter().zip(glyphs.glyph_info()) {
                if let Some((_, color)) = clusters.iter_mut().find(|(offset, _)| offset == cluster)
                {
                    *color |= glyph_is_color(glyph);
                } else {
                    clusters.push((*cluster, glyph_is_color(glyph)));
                }
            }
            clusters.sort_unstable_by_key(|(offset, _)| *offset);
            for (index, (start, color)) in clusters.iter().enumerate() {
                if !color {
                    continue;
                }
                let end = clusters
                    .get(index + 1)
                    .map_or(item.length(), |(offset, _)| *offset);
                let start = item.offset() + start;
                let end = item.offset() + end;
                if start >= 0 && end > start {
                    ranges.push(start as usize..end as usize);
                }
            }
        }
        if !iter.next_run() {
            break;
        }
    }
    ranges.sort_unstable_by_key(|range| range.start);
    let mut merged: Vec<Range<usize>> = Vec::new();
    for range in ranges {
        if let Some(last) = merged.last_mut()
            && range.start <= last.end
        {
            last.end = last.end.max(range.end);
        } else {
            merged.push(range);
        }
    }
    merged
}

fn glyph_is_color(glyph: &pango::GlyphInfo) -> bool {
    // pango-rs exposes the storage for PangoGlyphVisAttr's C bitfield but not its is_color bit.
    unsafe { (*glyph.as_ptr()).attr.is_cluster_start & PANGO_GLYPH_IS_COLOR_BIT != 0 }
}

fn pango_rect_bounds(rect: &pango::Rectangle, direction: TextDirection) -> (f64, f64, f64, f64) {
    let scale = pango::SCALE as f64;
    let x = rect.x() as f64 / scale;
    let y = rect.y() as f64 / scale;
    let width = rect.width() as f64 / scale;
    let height = rect.height() as f64 / scale;
    match direction {
        TextDirection::Horizontal => (x, y, x + width, y + height),
        TextDirection::Vertical => (-(y + height), x, -y, x + width),
    }
}

fn normalized_pango_rect(
    rect: &pango::Rectangle,
    direction: TextDirection,
    min_x: f64,
    min_y: f64,
) -> Rect {
    let (left, top, right, bottom) = pango_rect_bounds(rect, direction);
    Rect::from_ltrb(
        (left - min_x) as f32,
        (top - min_y) as f32,
        (right - min_x) as f32,
        (bottom - min_y) as f32,
    )
}

fn nearest_character(
    contour: &Path,
    characters: &[CharacterRect],
    color_only: bool,
) -> Option<usize> {
    let center = contour.compute_tight_bounds().center();
    characters
        .iter()
        .enumerate()
        .filter(|(_, character)| !color_only || character.color)
        .min_by(|(_, left), (_, right)| {
            let distance = |rect: &Rect| {
                let dx = center.x - center.x.clamp(rect.left, rect.right);
                let dy = center.y - center.y.clamp(rect.top, rect.bottom);
                dx * dx + dy * dy
            };
            distance(&left.rect).total_cmp(&distance(&right.rect))
        })
        .map(|(index, _)| index)
}

fn normalized_variations(text: &TextItem) -> Vec<(String, u32)> {
    let mut variations = text
        .font_variations
        .iter()
        .filter(|variation| {
            variation.value.is_finite()
                && variation.axis.len() == 4
                && variation.axis.bytes().all(|byte| byte.is_ascii_graphic())
                && variation.axis != "wght"
                && variation.axis != "ital"
        })
        .map(|variation| (variation.axis.clone(), variation.value.to_bits()))
        .collect::<Vec<_>>();
    variations.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    variations.dedup_by(|left, right| left.0 == right.0);
    variations
}

pub fn anchor(
    size: glam::Vec2,
    h_align: TextHorizontalAlign,
    v_align: VerticalAlign,
) -> glam::Vec2 {
    glam::Vec2::new(
        match h_align {
            TextHorizontalAlign::Left => 0.0,
            TextHorizontalAlign::Center | TextHorizontalAlign::Fill => size.x * 0.5,
            TextHorizontalAlign::Right => size.x,
        },
        match v_align {
            VerticalAlign::Top => 0.0,
            VerticalAlign::Middle => size.y * 0.5,
            VerticalAlign::Bottom => size.y,
        },
    )
}
