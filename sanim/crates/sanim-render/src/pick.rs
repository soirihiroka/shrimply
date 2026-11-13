//! Picking records the exact drawing paths on a null Skia canvas. No pixels or pictures.
use crate::{Options, Renderer, Scene, draw};
use sanim_core::{Element, Fraction, Geometry, Reveal};
use skia_safe::{Canvas, Contains, Matrix, Paint, Path, PathBuilder, Point, Rect, paint::Style};
use std::sync::Arc;
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub id: String,
    pub kind: &'static str,
    /// Outer to inner container identities, excluding this element.
    pub ancestors: Vec<String>,
    /// Full geometry bounds transformed to source canvas pixels, clockwise.
    pub quad: [[f32; 2]; 4],
}
#[derive(Clone)]
struct Clip {
    inverse: Matrix,
    bounds: Rect,
}
struct Entry {
    id: String,
    kind: &'static str,
    inherited_clips: Arc<Vec<Clip>>,
    hit: Option<Arc<Hit>>,
}
struct PaintedPath {
    path: Path,
    paint: Paint,
    fill: sanim_core::Paint,
    bounds: Rect,
    broad: Rect,
    matrix: Matrix,
    inverse: Matrix,
    clips: Arc<Vec<Clip>>,
    hit: Arc<Hit>,
    media: Option<Arc<shrimply_drawing_skia::media::Prepared>>,
}
pub(crate) struct Frame {
    paths: Vec<PaintedPath>,
}
#[derive(Default)]
struct Collector {
    entries: Vec<Entry>,
    clips: Arc<Vec<Clip>>,
    paths: Vec<PaintedPath>,
    media_seen: std::collections::BTreeSet<usize>,
    media_bytes: usize,
}
impl draw::Output for Collector {
    fn enter(&mut self, element: &Element) {
        let kind = match element.geometry.as_ref() {
            Geometry::Circle { .. } => "Circle",
            Geometry::Rectangle { .. } => "Rect",
            Geometry::Line { .. } => "Line",
            Geometry::Polyline { .. } => "Polyline",
            Geometry::Polygon { .. } => "Polygon",
            Geometry::Text { .. } => "Text",
            Geometry::Box { .. } => "Box",
            Geometry::Media { asset, .. } => match asset.kind {
                sanim_core::media::Kind::Bitmap => "Image",
                sanim_core::media::Kind::Svg => "SVG",
            },
            Geometry::Code { .. } => "Code",
        };
        self.entries.push(Entry {
            id: element.id.clone(),
            kind,
            inherited_clips: self.clips.clone(),
            hit: None,
        });
    }
    fn leave(&mut self) {
        self.clips = self
            .entries
            .pop()
            .expect("active pick element")
            .inherited_clips;
    }
    fn clip(&mut self, canvas: &Canvas, bounds: &Rect) {
        if let Some(inverse) = canvas.local_to_device_as_3x3().invert() {
            Arc::make_mut(&mut self.clips).push(Clip {
                inverse,
                bounds: *bounds,
            });
        }
    }
    fn media(
        &mut self,
        canvas: &Canvas,
        prepared: &Arc<shrimply_drawing_skia::media::Prepared>,
        bounds: &Rect,
        opacity: f32,
    ) -> Result<(), String> {
        if self.media_seen.insert(Arc::as_ptr(prepared) as usize) {
            self.media_bytes = self
                .media_bytes
                .checked_add(prepared.retained_bytes())
                .ok_or("media pick byte budget overflow")?;
            if self.media_bytes > sanim_data::Limits::default().max_bytes {
                return Err("native media pick exceeds its retained byte budget".into());
            }
        }
        let mut builder = PathBuilder::new();
        builder.add_rect(bounds, None, None);
        let path = builder.detach();
        let mut paint = Paint::default();
        paint.set_alpha_f(opacity);
        self.path(
            canvas,
            &path,
            &paint,
            &sanim_core::Color::parse("white")?.into(),
            bounds,
        )?;
        if let Some(record) = self.paths.last_mut() {
            record.media = Some(prepared.clone());
        }
        Ok(())
    }
    fn path(
        &mut self,
        canvas: &Canvas,
        path: &Path,
        paint: &Paint,
        fill: &sanim_core::Paint,
        bounds: &Rect,
    ) -> Result<(), String> {
        if paint.alpha_f() <= 0.0 || path.is_empty() {
            return Ok(());
        }
        let matrix = canvas.local_to_device_as_3x3();
        let Some(inverse) = matrix.invert() else {
            return Ok(());
        };
        let ancestors = self.entries[..self.entries.len() - 1]
            .iter()
            .map(|entry| entry.id.clone())
            .collect();
        let entry = self.entries.last_mut().expect("active pick element");
        let hit = entry
            .hit
            .get_or_insert_with(|| {
                Arc::new(Hit {
                    id: entry.id.clone(),
                    kind: entry.kind,
                    ancestors,
                    quad: matrix
                        .map_rect_to_quad(bounds)
                        .map(|point| [point.x, point.y]),
                })
            })
            .clone();
        let mut broad = *bounds;
        if paint.style() != Style::Fill {
            let reach = paint.stroke_width() * paint.stroke_miter().max(1.0) / 2.0;
            broad.outset((reach, reach));
        }
        self.paths.push(PaintedPath {
            path: path.clone(),
            paint: paint.clone(),
            fill: fill.clone(),
            bounds: *bounds,
            broad: matrix.map_rect(broad).0,
            matrix,
            inverse,
            clips: self.clips.clone(),
            hit,
            media: None,
        });
        Ok(())
    }
}
impl Frame {
    fn pick(&self, point: Point, tolerance: f32) -> Result<Option<Hit>, String> {
        for path in self.paths.iter().rev() {
            let stroked = path.paint.style() != Style::Fill;
            let hairline = stroked && path.paint.stroke_width() == 0.0;
            let tolerance = tolerance.max(if hairline { 0.5 } else { 0.0 });
            let mut broad = path.broad;
            if stroked {
                broad.outset((tolerance, tolerance));
            }
            if !broad.contains(point)
                || path
                    .clips
                    .iter()
                    .any(|clip| !clip.bounds.contains(clip.inverse.map_point(point)))
            {
                continue;
            }
            let local = path.inverse.map_point(point);
            if let Some(media) = &path.media
                && !media.contains([
                    (local.x - path.bounds.left) / path.bounds.width(),
                    (local.y - path.bounds.top) / path.bounds.height(),
                ])
            {
                continue;
            }
            if sanim_core::math::paint::alpha_at(
                &path.fill,
                [
                    path.bounds.left as f64,
                    path.bounds.top as f64,
                    path.bounds.right as f64,
                    path.bounds.bottom as f64,
                ],
                [local.x as f64, local.y as f64],
            )? <= 0.0
            {
                continue;
            }
            let contains = if stroked {
                let mut outline = PathBuilder::new();
                let filled = skia_safe::path_utils::fill_path_with_paint(
                    &path.path,
                    &path.paint,
                    &mut outline,
                    None,
                    path.matrix,
                );
                let world = if filled {
                    outline.detach().with_transform(&path.matrix)
                } else {
                    path.path.with_transform(&path.matrix)
                };
                if world.contains(point) && filled {
                    true
                } else if tolerance > 0.0 {
                    let mut paint = Paint::default();
                    paint
                        .set_style(Style::Stroke)
                        .set_stroke_width(tolerance * 2.0);
                    let mut expanded = PathBuilder::new();
                    skia_safe::path_utils::fill_path_with_paint(
                        &world,
                        &paint,
                        &mut expanded,
                        None,
                        None,
                    );
                    expanded.detach().contains(point)
                } else {
                    false
                }
            } else {
                path.path.contains(local)
            };
            if contains {
                return Ok(Some(path.hit.as_ref().clone()));
            }
        }
        Ok(None)
    }
    fn bytes(&self) -> usize {
        let mut media_seen = std::collections::BTreeSet::new();
        let media_bytes: usize = self
            .paths
            .iter()
            .filter_map(|path| path.media.as_ref())
            .filter(|asset| media_seen.insert(Arc::as_ptr(asset) as usize))
            .map(|asset| asset.retained_bytes())
            .sum();
        media_bytes
            + self.paths.capacity() * std::mem::size_of::<PaintedPath>()
            + self
                .paths
                .iter()
                .map(|path| {
                    path.path.approximate_bytes_used()
                        + path.clips.capacity() * std::mem::size_of::<Clip>()
                        + path.hit.id.capacity()
                        + path
                            .hit
                            .ancestors
                            .iter()
                            .map(String::capacity)
                            .sum::<usize>()
                })
                .sum::<usize>()
    }
}
impl Renderer {
    /// Source-canvas pixels. Reuses bounded sampled-state, geometry and native pick-command caches.
    pub fn pick(
        &mut self,
        scene: &Scene,
        time: Fraction,
        options: &Options,
        point: [f32; 2],
        tolerance: f32,
    ) -> Result<Option<Hit>, String> {
        if point.iter().any(|value| !value.is_finite()) || !tolerance.is_finite() || tolerance < 0.0
        {
            return Err("pick point and nonnegative tolerance must be finite".into());
        }
        if options.width == 0 || options.height == 0 {
            return Err("pick canvas dimensions must be positive".into());
        }
        if point[0] < 0.0
            || point[1] < 0.0
            || point[0] >= options.width as f32
            || point[1] >= options.height as f32
        {
            return Ok(None);
        }
        let key = crate::record_key(scene, time, options)?;
        let frame = if let Some(frame) = self.picks.get(&key) {
            frame
        } else {
            let state = self.frame_state(scene, time, options)?;
            let width = i32::try_from(options.width)
                .map_err(|_| "pick width exceeds Skia numeric range")?;
            let height = i32::try_from(options.height)
                .map_err(|_| "pick height exceeds Skia numeric range")?;
            let mut surface =
                skia_safe::surfaces::null((width, height)).ok_or("invalid pick viewport")?;
            let canvas = surface.canvas();
            canvas.scale((
                options.width as f32 / sanim_core::math::to_f64(options.logical_width)? as f32,
                options.height as f32 / sanim_core::math::to_f64(options.logical_height)? as f32,
            ));
            let mut drawing = draw::Drawing {
                renderer: self,
                pixels: 0,
                work: options.max_work,
                output: Collector::default(),
            };
            for element in state.elements() {
                draw::element_draw(canvas, element, Reveal::default(), &mut drawing)?;
            }
            let frame = Frame {
                paths: drawing.output.paths,
            };
            let bytes = frame.bytes();
            // SkPath is Send but not Sync; the shared cache guards each native snapshot.
            let frame = Arc::new(std::sync::Mutex::new(frame));
            self.picks.insert(key, frame.clone(), bytes);
            frame
        };
        frame
            .lock()
            .expect("Sanim pick frame poisoned")
            .pick(Point::new(point[0], point[1]), tolerance)
    }
}
