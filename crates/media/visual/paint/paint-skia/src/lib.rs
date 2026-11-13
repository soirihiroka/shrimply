use std::fmt;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use glam::Vec2;
use shrimply_asset::{Asset, AssetSnapshot};
use shrimply_math_color::Color;
use shrimply_math_geometry::ResolvedTransform2D;
pub use shrimply_paint_geometry::ResolvedPathOffset;
use shrimply_paint_geometry::{GeometryKey, OutlineKey, PreparedGeometry, PreparedOutlines};
use shrimply_paint_model::{
    PaintDrawing, ResolvedPaintFillOptions, ResolvedPaintStrokeOptions, ResolvedPaintTextureOptions,
};
use skia_safe::image::CachingHint;
use skia_safe::{
    AlphaType, BlendMode, Canvas, ColorType, Data, FilterMode, Image, ImageInfo, Matrix,
    MipmapMode, Paint, Path, PathBuilder, PathFillType, Point, SamplingOptions, TileMode,
    color_filters,
};

const MAX_CACHED_TEXTURES: usize = 2;

pub type TextureFingerprint = AssetSnapshot;

#[derive(Debug)]
pub enum PaintRenderError {
    MissingTexture(PathBuf),
    UnreadableTexture { path: PathBuf, source: String },
    UndecodableTexture(PathBuf),
    UnreadableTexturePixels(PathBuf),
    InvalidTextureDimensions(PathBuf),
    InvalidTextureScale(f32),
    InvalidTextureRotation(f32),
    TextureMaskCreation(PathBuf),
    TextureShaderCreation(PathBuf),
}

impl fmt::Display for PaintRenderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingTexture(path) => {
                write!(
                    formatter,
                    "paint texture does not exist: {}",
                    path.display()
                )
            }
            Self::UnreadableTexture { path, source } => {
                write!(
                    formatter,
                    "cannot read paint texture {}: {source}",
                    path.display()
                )
            }
            Self::UndecodableTexture(path) => {
                write!(
                    formatter,
                    "Skia cannot decode paint texture: {}",
                    path.display()
                )
            }
            Self::UnreadableTexturePixels(path) => write!(
                formatter,
                "Skia cannot read decoded paint texture pixels: {}",
                path.display()
            ),
            Self::InvalidTextureDimensions(path) => write!(
                formatter,
                "paint texture has invalid dimensions: {}",
                path.display()
            ),
            Self::InvalidTextureScale(scale) => {
                write!(
                    formatter,
                    "paint texture repeat scale must be positive: {scale}"
                )
            }
            Self::InvalidTextureRotation(rotation) => write!(
                formatter,
                "paint texture rotation must be finite: {rotation}"
            ),
            Self::TextureMaskCreation(path) => write!(
                formatter,
                "Skia cannot create paint texture mask: {}",
                path.display()
            ),
            Self::TextureShaderCreation(path) => write!(
                formatter,
                "Skia cannot create paint texture shader: {}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for PaintRenderError {}

#[derive(Clone, Debug)]
pub struct PreparedTexture {
    pub fingerprint: TextureFingerprint,
    mask: Arc<Image>,
    repeat_scale: f32,
    rotation_degrees: f32,
}

#[derive(Clone, Debug)]
pub struct PreparedPaintFrame {
    pub geometry: Arc<PreparedGeometry>,
    pub outlines: Arc<PreparedOutlines>,
    stroke_options: ResolvedPaintStrokeOptions,
}

#[derive(Clone, Debug)]
pub struct ResolvedPaintTexture {
    pub image_path: Asset,
    pub options: ResolvedPaintTextureOptions,
}

#[derive(Clone, Copy, Debug)]
pub struct ResolvedPaintAppearance<'a> {
    pub palette: &'a [ResolvedPaintPaletteEntry],
    pub reveal: Option<PaintReveal<'a>>,
}

#[derive(Clone, Copy, Debug)]
pub struct PaintReveal<'a> {
    pub stroke_progress: &'a [f32],
    pub fill_opacity: &'a [f32],
}

#[derive(Clone, Debug)]
pub struct ResolvedPaintPaletteEntry {
    pub color: Color<u8>,
    pub texture: Option<ResolvedPaintTexture>,
}

/// Staged rendering data for one logical paint item.
///
/// Paint revisions are item-local, so different items should not share this cache.
#[derive(Default)]
pub struct PaintCache {
    geometry: Option<(GeometryKey, Arc<PreparedGeometry>)>,
    outlines: Option<(OutlineKey, Arc<PreparedOutlines>)>,
    fill_paths: Option<(GeometryKey, Rc<Vec<Path>>)>,
    stroke_paths: Option<(OutlineKey, Rc<Vec<Path>>)>,
    textures: Vec<(TextureFingerprint, Arc<Image>)>,
}

impl PaintCache {
    pub fn clear(&mut self) {
        self.geometry = None;
        self.outlines = None;
        self.fill_paths = None;
        self.stroke_paths = None;
        self.textures.clear();
    }
}

pub fn prepare_frame(
    cache: &mut PaintCache,
    content: (&PaintDrawing, u64),
    stroke: &ResolvedPaintStrokeOptions,
    fill: ResolvedPaintFillOptions,
    path_offsets: &[ResolvedPathOffset],
    transform: ResolvedTransform2D,
    canvas_size: Vec2,
) -> PreparedPaintFrame {
    let geometry = prepare_geometry(
        cache,
        content,
        stroke,
        fill,
        path_offsets,
        transform,
        canvas_size,
    );
    let outlines = prepare_outlines(cache, stroke, &geometry);
    PreparedPaintFrame {
        geometry,
        outlines,
        stroke_options: *stroke,
    }
}

pub fn prepare_geometry(
    cache: &mut PaintCache,
    content: (&PaintDrawing, u64),
    stroke: &ResolvedPaintStrokeOptions,
    fill: ResolvedPaintFillOptions,
    path_offsets: &[ResolvedPathOffset],
    transform: ResolvedTransform2D,
    canvas_size: Vec2,
) -> Arc<PreparedGeometry> {
    let (drawing, revision) = content;
    let key = shrimply_paint_geometry::render_geometry_key_with_path_offsets(
        drawing,
        revision,
        stroke,
        fill,
        transform,
        canvas_size,
        path_offsets,
    );
    if let Some((_, geometry)) = cache.geometry.as_ref().filter(|(cached, _)| cached == &key) {
        return Arc::clone(geometry);
    }
    let geometry = Arc::new(
        shrimply_paint_geometry::prepare_render_geometry_with_path_offsets(
            drawing,
            revision,
            stroke,
            fill,
            transform,
            canvas_size,
            path_offsets,
        ),
    );
    cache.geometry = Some((key, Arc::clone(&geometry)));
    cache.outlines = None;
    cache.fill_paths = None;
    cache.stroke_paths = None;
    geometry
}

pub fn prepare_outlines(
    cache: &mut PaintCache,
    stroke: &ResolvedPaintStrokeOptions,
    geometry: &PreparedGeometry,
) -> Arc<PreparedOutlines> {
    let key = shrimply_paint_geometry::OutlineKey {
        centerlines: geometry.key.centerlines.clone(),
        shape: shrimply_paint_geometry::StrokeShapeKey::from(stroke),
    };
    if let Some((_, outlines)) = cache.outlines.as_ref().filter(|(cached, _)| cached == &key) {
        return Arc::clone(outlines);
    }
    let outlines = Arc::new(shrimply_paint_geometry::prepare_outlines(
        &geometry.centerlines,
        geometry.key.centerlines.clone(),
        stroke,
    ));
    cache.outlines = Some((key, Arc::clone(&outlines)));
    cache.stroke_paths = None;
    outlines
}

pub fn prepare_texture(
    cache: &mut PaintCache,
    texture: &ResolvedPaintTexture,
) -> Result<PreparedTexture, PaintRenderError> {
    if !texture.options.repeat_scale.is_finite() || texture.options.repeat_scale <= 0.0 {
        return Err(PaintRenderError::InvalidTextureScale(
            texture.options.repeat_scale,
        ));
    }
    if !texture.options.rotation_degrees.is_finite() {
        return Err(PaintRenderError::InvalidTextureRotation(
            texture.options.rotation_degrees,
        ));
    }

    let fingerprint = texture_fingerprint(&texture.image_path)?;
    let mask = match cache
        .textures
        .iter()
        .position(|(cached, _)| cached == &fingerprint)
    {
        Some(index) => {
            let entry = cache.textures.remove(index);
            let mask = Arc::clone(&entry.1);
            cache.textures.push(entry);
            mask
        }
        None => {
            let mask = Arc::new(decode_texture_mask(&texture.image_path, &fingerprint)?);
            cache
                .textures
                .retain(|(key, _)| key.asset() != fingerprint.asset());
            cache
                .textures
                .push((fingerprint.clone(), Arc::clone(&mask)));
            if cache.textures.len() > MAX_CACHED_TEXTURES {
                cache.textures.remove(0);
            }
            mask
        }
    };
    Ok(PreparedTexture {
        fingerprint,
        mask,
        repeat_scale: texture.options.repeat_scale,
        rotation_degrees: texture.options.rotation_degrees,
    })
}

pub fn texture_fingerprint(asset: &Asset) -> Result<TextureFingerprint, PaintRenderError> {
    asset
        .snapshot()
        .map_err(|source| PaintRenderError::UnreadableTexture {
            path: asset.path().to_path_buf(),
            source,
        })
}

pub fn draw(
    cache: &mut PaintCache,
    canvas: &Canvas,
    frame: &PreparedPaintFrame,
    appearance: ResolvedPaintAppearance<'_>,
    path_effect: Option<&skia_safe::PathEffect>,
) -> Result<(), PaintRenderError> {
    let textures: Vec<_> = appearance
        .palette
        .iter()
        .map(|entry| {
            entry
                .texture
                .as_ref()
                .map(|texture| prepare_texture(cache, texture))
                .transpose()
        })
        .collect::<Result<_, _>>()?;
    let fill_paths = prepare_fill_paths(cache, &frame.geometry);
    let stroke_paths = prepare_stroke_paths(cache, &frame.outlines);
    if let Some(reveal) = appearance.reveal {
        assert_eq!(
            reveal.stroke_progress.len(),
            frame.geometry.centerlines.len()
        );
        assert_eq!(reveal.fill_opacity.len(), frame.geometry.fills.len());
    }

    for (index, (fill, path)) in frame
        .geometry
        .fills
        .iter()
        .zip(fill_paths.iter())
        .enumerate()
    {
        let opacity = appearance
            .reveal
            .map_or(1.0, |reveal| reveal.fill_opacity[index])
            .clamp(0.0, 1.0);
        if opacity <= 0.0 {
            continue;
        }
        let entry = appearance
            .palette
            .get(fill.color_index)
            .expect("paint fill palette index is invalid");
        let fill_paint = drawing_paint(
            entry.color.alpha_multiply(opacity),
            textures[fill.color_index].as_ref(),
            path_effect,
        )?;
        canvas.draw_path(path, &fill_paint);
    }
    for (index, (centerline, (outline, path))) in frame
        .geometry
        .centerlines
        .iter()
        .zip(frame.outlines.outlines.iter().zip(stroke_paths.iter()))
        .enumerate()
    {
        let progress = appearance
            .reveal
            .map_or(1.0, |reveal| reveal.stroke_progress[index])
            .clamp(0.0, 1.0);
        if progress <= 0.0 {
            continue;
        }
        let entry = appearance
            .palette
            .get(outline.color_index)
            .expect("paint stroke palette index is invalid");
        let stroke_paint = drawing_paint(
            entry.color,
            textures[outline.color_index].as_ref(),
            path_effect,
        )?;
        if progress >= 1.0 {
            canvas.draw_path(path, &stroke_paint);
        } else {
            let partial = shrimply_paint_geometry::PreparedCenterline {
                stroke_points: shrimply_paint_geometry::partial_stroke_points(
                    &centerline.stroke_points,
                    progress,
                ),
                completed: false,
                ..centerline.clone()
            };
            let outline = shrimply_paint_geometry::prepare_outline(&partial, &frame.stroke_options);
            canvas.draw_path(&outline_path(&outline.points), &stroke_paint);
        }
    }
    Ok(())
}

pub fn prepare_fill_paths(cache: &mut PaintCache, geometry: &PreparedGeometry) -> Rc<Vec<Path>> {
    if let Some((_, paths)) = cache
        .fill_paths
        .as_ref()
        .filter(|(cached, _)| cached == &geometry.key)
    {
        return Rc::clone(paths);
    }
    let paths = Rc::new(
        geometry
            .fills
            .iter()
            .map(|fill| {
                let mut builder = PathBuilder::new_with_fill_type(PathFillType::EvenOdd);
                for boundary in &fill.loops {
                    let points: Vec<_> = boundary
                        .iter()
                        .map(|point| Point::new(point.x, point.y))
                        .collect();
                    builder.add_polygon(&points, true);
                }
                builder.detach()
            })
            .collect(),
    );
    cache.fill_paths = Some((geometry.key.clone(), Rc::clone(&paths)));
    paths
}

pub fn prepare_stroke_paths(cache: &mut PaintCache, outlines: &PreparedOutlines) -> Rc<Vec<Path>> {
    if let Some((_, paths)) = cache
        .stroke_paths
        .as_ref()
        .filter(|(cached, _)| cached == &outlines.key)
    {
        return Rc::clone(paths);
    }
    let paths = Rc::new(
        outlines
            .outlines
            .iter()
            .map(|outline| outline_path(&outline.points))
            .collect(),
    );
    cache.stroke_paths = Some((outlines.key.clone(), Rc::clone(&paths)));
    paths
}

pub fn morph_paint(
    cache: &mut PaintCache,
    entry: &ResolvedPaintPaletteEntry,
) -> Result<Paint, PaintRenderError> {
    let texture = entry
        .texture
        .as_ref()
        .map(|texture| prepare_texture(cache, texture))
        .transpose()?;
    drawing_paint(entry.color, texture.as_ref(), None)
}

/// Converts a perfect-freehand outline into the midpoint quadratic path its
/// point filtering is designed for.
pub fn outline_path(points: &[Vec2]) -> Path {
    let mut builder = PathBuilder::new();
    let Some(first) = points.first() else {
        return builder.detach();
    };
    builder.move_to(Point::new(first.x, first.y));
    for pair in points[1..].windows(2) {
        let control = pair[0];
        let end = (pair[0] + pair[1]) * 0.5;
        builder.quad_to(Point::new(control.x, control.y), Point::new(end.x, end.y));
    }
    builder.close();
    builder.detach()
}

fn decode_texture_mask(asset: &Asset, snapshot: &AssetSnapshot) -> Result<Image, PaintRenderError> {
    let path = asset.path();
    let encoded = snapshot
        .read()
        .map_err(|source| PaintRenderError::UnreadableTexture {
            path: path.to_path_buf(),
            source,
        })?;
    let prepared = shrimply_drawing_skia::media::prepare(std::sync::Arc::new(encoded))
        .map_err(|_| PaintRenderError::UndecodableTexture(path.to_path_buf()))?;
    let decoded = prepared
        .bitmap()
        .ok_or_else(|| PaintRenderError::UndecodableTexture(path.to_path_buf()))?;
    let dimensions = decoded.dimensions();
    let width = usize::try_from(dimensions.width)
        .ok()
        .filter(|width| *width > 0)
        .ok_or_else(|| PaintRenderError::InvalidTextureDimensions(path.to_path_buf()))?;
    let height = usize::try_from(dimensions.height)
        .ok()
        .filter(|height| *height > 0)
        .ok_or_else(|| PaintRenderError::InvalidTextureDimensions(path.to_path_buf()))?;
    let pixel_count = width
        .checked_mul(height)
        .ok_or_else(|| PaintRenderError::InvalidTextureDimensions(path.to_path_buf()))?;
    let rgba_size = pixel_count
        .checked_mul(4)
        .ok_or_else(|| PaintRenderError::InvalidTextureDimensions(path.to_path_buf()))?;
    let rgba_info = ImageInfo::new(dimensions, ColorType::RGBA8888, AlphaType::Unpremul, None);
    let mut rgba = vec![0_u8; rgba_size];
    if !decoded.read_pixels(
        &rgba_info,
        &mut rgba,
        width * 4,
        (0, 0),
        CachingHint::Disallow,
    ) {
        return Err(PaintRenderError::UnreadableTexturePixels(
            path.to_path_buf(),
        ));
    }

    let coverage: Vec<_> = rgba
        .chunks_exact(4)
        .map(|pixel| {
            (Color::<f32>::from_rgba8(pixel[0], pixel[1], pixel[2], pixel[3]).rec709_luma()
                * f32::from(pixel[3]))
            .round() as u8
        })
        .collect();
    skia_safe::images::raster_from_data(
        &ImageInfo::new_a8(dimensions),
        Data::new_copy(&coverage),
        width,
    )
    .ok_or_else(|| PaintRenderError::TextureMaskCreation(path.to_path_buf()))
}

fn drawing_paint(
    color: Color<u8>,
    texture: Option<&PreparedTexture>,
    path_effect: Option<&skia_safe::PathEffect>,
) -> Result<Paint, PaintRenderError> {
    let color: skia_safe::Color = color.into();
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_path_effect(path_effect.cloned());
    match texture {
        None => {
            paint.set_color(color);
        }
        Some(texture) => {
            let local_matrix = Matrix::rotate_deg(texture.rotation_degrees)
                * Matrix::scale((texture.repeat_scale, texture.repeat_scale));
            let shader = texture
                .mask
                .to_shader(
                    (TileMode::Repeat, TileMode::Repeat),
                    SamplingOptions::new(FilterMode::Linear, MipmapMode::None),
                    &local_matrix,
                )
                .ok_or_else(|| {
                    PaintRenderError::TextureShaderCreation(
                        texture.fingerprint.path().to_path_buf(),
                    )
                })?;
            let tint = color_filters::blend(color, BlendMode::SrcIn).ok_or_else(|| {
                PaintRenderError::TextureShaderCreation(texture.fingerprint.path().to_path_buf())
            })?;
            paint.set_shader(shader.with_color_filter(tint));
        }
    }
    Ok(paint)
}
