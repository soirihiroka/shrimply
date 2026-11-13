//! Skia command recording and explicit headless raster output. No SVG or GUI toolkit.
use sanim_core::{math::to_f64, *};
use skia_safe::{Canvas, Picture, PictureRecorder, Rect};
use std::{collections::BTreeMap, rc::Rc, sync::Arc};
mod draw;
mod pick;
pub use pick::Hit;
use sanim_core::cache::Cache;

#[derive(Clone, Debug)]
pub struct Options {
    pub width: u32,
    pub height: u32,
    pub logical_width: Fraction,
    pub logical_height: Fraction,
    pub max_pixels: usize,
    pub max_work: usize,
    pub encode_png: bool,
}

impl Options {
    pub fn for_animation(animation: &Animation) -> Self {
        Self {
            width: animation.width,
            height: animation.height,
            logical_width: animation.logical_size().0,
            logical_height: animation.logical_size().1,
            max_pixels: 16_777_216,
            max_work: 67_108_864,
            encode_png: true,
        }
    }
}

pub struct Scene {
    animation: Arc<Animation>,
    key: String,
    times: SampleTimes,
}
impl Scene {
    pub fn animation(&self) -> &Animation {
        &self.animation
    }
    pub fn new(animation: Animation) -> Result<Self, String> {
        Renderer::default().prepare(animation)
    }
}

#[derive(Debug)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub png: Vec<u8>,
    pub render_micros: u64,
}

/// Immutable Skia drawing commands. Replayed directly on the host's GPU canvas.
/// Recording allocates no raster surface and never reads pixels from a GPU.
pub struct Recording {
    pub width: u32,
    pub height: u32,
    pub record_micros: u64,
    id: u64,
    picture: Picture,
}
impl Recording {
    pub fn id(&self) -> u64 {
        self.id
    }
    /// Record native vector commands, including host loading/diagnostic overlays.
    pub fn capture(
        width: u32,
        height: u32,
        draw: impl FnOnce(&Canvas) -> Result<(), String>,
    ) -> Result<Self, String> {
        if width == 0 || height == 0 || width > i32::MAX as u32 || height > i32::MAX as u32 {
            return Err("recording requires positive dimensions within Skia's range".into());
        }
        let started = std::time::Instant::now();
        let mut recorder = PictureRecorder::new();
        let canvas = recorder.begin_recording(Rect::from_wh(width as f32, height as f32), true);
        canvas.clip_rect(Rect::from_wh(width as f32, height as f32), None, false);
        draw(canvas)?;
        Ok(Self {
            width,
            height,
            record_micros: started.elapsed().as_micros().try_into().unwrap_or(u64::MAX),
            id: {
                use std::sync::atomic::{AtomicU64, Ordering};
                static NEXT: AtomicU64 = AtomicU64::new(1);
                NEXT.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
                    .expect("recording identity overflow")
            },
            picture: recorder
                .finish_recording_as_picture(None)
                .ok_or("Skia recording failed")?,
        })
    }
    pub fn draw(&self, canvas: &Canvas) {
        canvas.draw_picture(&self.picture, None, None);
    }
    /// Explicit CPU-only output for headless PNG export/tests. Editor paths use draw().
    pub fn rasterize(&self, encode_png: bool) -> Result<Frame, String> {
        self.rasterize_background(encode_png, skia_safe::Color::TRANSPARENT)
    }
    /// Opaque black compositing for formats without alpha, such as H.264 video.
    pub fn rasterize_opaque(&self) -> Result<Frame, String> {
        self.rasterize_background(false, skia_safe::Color::BLACK)
    }
    fn rasterize_background(
        &self,
        encode_png: bool,
        background: skia_safe::Color,
    ) -> Result<Frame, String> {
        use skia_safe::{AlphaType, ColorType, ImageInfo};
        let started = std::time::Instant::now();
        let width = i32::try_from(self.width).map_err(|_| "width exceeds Skia range")?;
        let height = i32::try_from(self.height).map_err(|_| "height exceeds Skia range")?;
        let mut surface = skia_safe::surfaces::raster_n32_premul((width, height))
            .ok_or("cannot allocate Skia raster surface")?;
        surface.canvas().clear(background);
        self.draw(surface.canvas());
        let png = if encode_png {
            let pixels = surface
                .peek_pixels()
                .ok_or("cannot access Skia raster pixels")?;
            skia_safe::png_encoder::encode_pixmap(&pixels, &Default::default())
                .ok_or("Skia PNG encoding failed")?
                .as_bytes()
                .to_vec()
        } else {
            Vec::new()
        };
        let row_bytes = (self.width as usize)
            .checked_mul(4)
            .ok_or("row size overflow")?;
        let bytes = row_bytes
            .checked_mul(self.height as usize)
            .ok_or("frame size overflow")?;
        let mut rgba = vec![0; bytes];
        let info = ImageInfo::new(
            (width, height),
            ColorType::RGBA8888,
            AlphaType::Unpremul,
            None,
        );
        if !surface.read_pixels(&info, &mut rgba, row_bytes, (0, 0)) {
            return Err("Skia raster read failed".into());
        }
        Ok(Frame {
            width: self.width,
            height: self.height,
            rgba,
            png,
            render_micros: started.elapsed().as_micros().try_into().unwrap_or(u64::MAX),
        })
    }
}

/// Independent bounds for calculated states, GPU command recordings and optional CPU pixels.
#[derive(Clone, Copy, Debug)]
pub struct CacheLimits {
    pub entries: usize,
    pub state_bytes: usize,
    pub recording_bytes: usize,
    pub raster_bytes: usize,
}
impl Default for CacheLimits {
    fn default() -> Self {
        const MIB: usize = 1024 * 1024;
        Self {
            entries: 256,
            state_bytes: 64 * MIB,
            recording_bytes: 64 * MIB,
            raster_bytes: 128 * MIB,
        }
    }
}

/// A calculated layout snapshot, independent of raster output and recording work limits.
pub struct FrameState {
    elements: Vec<Element>,
    pub compute_micros: u64,
}
impl FrameState {
    pub fn elements(&self) -> &[Element] {
        &self.elements
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct StateKey {
    scene: String,
    // Lazy text/code can acquire glyphs after the initial scene was prepared.
    // Their native font environment must also identify cached future states.
    font_epoch: Option<sanim_data::font::FontEpoch>,
    time: Fraction,
    width: Fraction,
    height: Fraction,
}
#[derive(Clone, PartialEq, Eq, Hash)]
struct RecordKey {
    state: StateKey,
    width: u32,
    height: u32,
    max_work: usize,
    max_pixels: usize,
}
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum RasterKind {
    Rgba,
    Png,
    Opaque,
}

pub struct Renderer {
    states: Cache<StateKey, FrameState>,
    picks: Cache<RecordKey, std::sync::Mutex<pick::Frame>>,
    recordings: Cache<RecordKey, Recording>,
    frames: Cache<(RecordKey, RasterKind), Frame>,
    previous: Option<(String, Arc<Animation>)>,
    pub cache_hits: u64,
    pub state_cache_hits: u64,
    pub states_computed: u64,
    pub geometry_cache_hits: u64,
    /// Paths built only after drawing or explicit geometry validation demands them.
    pub geometries_prepared: u64,
    /// Stable shared geometry avoids serialization and hashing on repeated draws.
    pub geometry_key_hits: u64,
    geometry_cache: BTreeMap<String, Rc<shrimply_drawing_skia::path::PreparedPath>>,
    geometry_hints: BTreeMap<usize, draw::GeometryHint>,
    geometry_cache_bytes: usize,
}
impl Default for Renderer {
    fn default() -> Self {
        Self::with_cache_limits(CacheLimits::default())
    }
}
impl Renderer {
    pub fn with_cache_limits(limits: CacheLimits) -> Self {
        Self {
            states: Cache::new(limits.state_bytes, limits.entries),
            picks: Cache::new(limits.recording_bytes, limits.entries),
            recordings: Cache::new(limits.recording_bytes, limits.entries),
            frames: Cache::new(limits.raster_bytes, limits.entries),
            previous: None,
            cache_hits: 0,
            state_cache_hits: 0,
            states_computed: 0,
            geometry_cache_hits: 0,
            geometries_prepared: 0,
            geometry_key_hits: 0,
            geometry_cache: BTreeMap::new(),
            geometry_hints: BTreeMap::new(),
            geometry_cache_bytes: 0,
        }
    }
    /// Preserve immutable path geometry across source, style, timing and viewport edits.
    pub fn prepare(&mut self, animation: Animation) -> Result<Scene, String> {
        self.geometry_hints
            .retain(|_, hint| hint._geometry.strong_count() > 0 && hint.path.strong_count() > 0);
        let key = digest(&animation.visual_key()?);
        let times = SampleTimes::new(&animation);
        if let Some((old_key, old)) = &self.previous
            && old_key != &key
        {
            let diff = TimelineDiff::between(old, &animation);
            let update = |mut state: StateKey| {
                if state.scene == *old_key {
                    if diff.affects(state.time) {
                        return None;
                    }
                    state.scene = key.clone();
                    state.time = times.canonical(state.time).expect("cached sample time");
                }
                Some(state)
            };
            self.states.rekey(&update);
            self.recordings.rekey(|mut record| {
                record.state = update(record.state)?;
                Some(record)
            });
            self.picks.rekey(|mut record| {
                record.state = update(record.state)?;
                Some(record)
            });
            self.frames.rekey(|(mut record, png)| {
                record.state = update(record.state)?;
                Some((record, png))
            });
        }
        let animation = Arc::new(animation);
        self.previous = Some((key.clone(), animation.clone()));
        Ok(Scene {
            animation,
            key,
            times,
        })
    }
    /// Force every current/future path for full CLI validation. Interactive scene
    /// preparation leaves geometry deferred until a frame actually draws it.
    pub fn validate_geometry(&mut self, scene: &Scene) -> Result<(), String> {
        fn visit(element: &Element, renderer: &mut Renderer) -> Result<(), String> {
            if !matches!(
                element.geometry.as_ref(),
                Geometry::Box { .. } | Geometry::Code { .. } | Geometry::Media { .. }
            ) {
                draw::prepared_path(element, renderer)?;
            }
            if let Some(children) = element.geometry.children() {
                for child in children {
                    visit(child, renderer)?;
                }
            }
            Ok(())
        }
        for layer in &scene.animation.layers {
            visit(&layer.element, self)?;
        }
        for element in scene.animation.list_views.elements() {
            visit(element, self)?;
        }
        Ok(())
    }
    pub fn clear_cache(&mut self) {
        self.recordings.clear();
        self.states.clear();
        self.picks.clear();
        self.frames.clear();
        self.geometry_cache.clear();
        self.geometry_hints.clear();
        self.geometry_cache_bytes = 0;
        self.previous = None;
    }
    pub fn frame_state(
        &mut self,
        scene: &Scene,
        time: Fraction,
        options: &Options,
    ) -> Result<Arc<FrameState>, String> {
        let key = state_key(scene, time, options)?;
        if let Some(state) = self.states.get(&key) {
            self.state_cache_hits += 1;
            return Ok(state);
        }
        let started = std::time::Instant::now();
        let limits = sanim_data::Limits::default();
        let mut generated_bytes = limits.max_bytes;
        let mut generated_items = limits.max_values;
        let mut fonts = sanim_core::value_budget::Budget::default();
        let elements =
            scene
                .animation
                .layout_with(key.time, key.width, key.height, |element, time| {
                    let pending = matches!(
                        element.geometry.as_ref(),
                        Geometry::Code { pending: true, .. } | Geometry::Text { pending: true, .. }
                    );
                    sanim_data::resolve_pending(element, time)?;
                    if pending {
                        sanim_core::value_budget::charge_with_budget(
                            &Value::Element(Arc::new(element.clone())),
                            &mut generated_bytes,
                            &mut generated_items,
                            limits.max_depth,
                            &mut fonts,
                        )
                        .map_err(|error| {
                            format!("native frame geometry for '{}': {error}", element.id)
                        })?;
                    }
                    Ok(())
                })?;
        let bytes = state_bytes(&elements, limits.max_depth)?;
        let state = Arc::new(FrameState {
            elements,
            compute_micros: started.elapsed().as_micros().try_into().unwrap_or(u64::MAX),
        });
        self.states_computed += 1;
        self.states.insert(key, state.clone(), bytes);
        Ok(state)
    }
    pub fn record(
        &mut self,
        scene: &Scene,
        time: Fraction,
        options: &Options,
    ) -> Result<Arc<Recording>, String> {
        let started = std::time::Instant::now();
        let pixels = (options.width as usize)
            .checked_mul(options.height as usize)
            .ok_or("output dimensions overflow")?;
        if pixels == 0 || pixels > options.max_pixels {
            return Err(format!(
                "output needs {pixels} pixels; limit {}",
                options.max_pixels
            ));
        }
        if options.width > i32::MAX as u32 || options.height > i32::MAX as u32 {
            return Err("output exceeds Skia dimensions".into());
        }
        let key = record_key(scene, time, options)?;
        if let Some(recording) = self.recordings.get(&key) {
            self.cache_hits += 1;
            return Ok(recording);
        }
        let state = self.frame_state(scene, time, options)?;
        let mut media_bytes = 0;
        let mut recording = Recording::capture(options.width, options.height, |canvas| {
            let logical_width = to_f64(options.logical_width)? as f32;
            let logical_height = to_f64(options.logical_height)? as f32;
            canvas.scale((
                options.width as f32 / logical_width,
                options.height as f32 / logical_height,
            ));
            let background = draw::paint(&scene.animation.background, Fraction::from(1))?;
            canvas.draw_rect(Rect::from_wh(logical_width, logical_height), &background);
            let mut drawing = draw::Drawing {
                renderer: self,
                pixels,
                work: options.max_work,
                output: draw::RecordingOutput::default(),
            };
            for element in &state.elements {
                draw::element_draw(canvas, element, Reveal::default(), &mut drawing)?;
            }
            media_bytes = drawing.output.media_bytes;
            Ok(())
        })?;
        recording.record_micros = started.elapsed().as_micros().try_into().unwrap_or(u64::MAX);
        let recording = Arc::new(recording);
        let bytes = recording.picture.approximate_bytes_used()
            + std::mem::size_of::<Recording>()
            + media_bytes;
        self.recordings.insert(key, recording.clone(), bytes);
        Ok(recording)
    }
    pub fn render(
        &mut self,
        scene: &Scene,
        time: Fraction,
        options: &Options,
    ) -> Result<Arc<Frame>, String> {
        self.render_kind(
            scene,
            time,
            options,
            if options.encode_png {
                RasterKind::Png
            } else {
                RasterKind::Rgba
            },
        )
    }
    pub fn render_opaque(
        &mut self,
        scene: &Scene,
        time: Fraction,
        options: &Options,
    ) -> Result<Arc<Frame>, String> {
        self.render_kind(scene, time, options, RasterKind::Opaque)
    }
    fn render_kind(
        &mut self,
        scene: &Scene,
        time: Fraction,
        options: &Options,
        kind: RasterKind,
    ) -> Result<Arc<Frame>, String> {
        let started = std::time::Instant::now();
        // Limits are part of the key: a cached raster already passed these exact checks.
        let key = (record_key(scene, time, options)?, kind);
        if let Some(frame) = self.frames.get(&key) {
            self.cache_hits += 1;
            return Ok(frame);
        }
        let recording = self.record(scene, time, options)?;
        let mut frame = match kind {
            RasterKind::Opaque => recording.rasterize_opaque()?,
            _ => recording.rasterize(kind == RasterKind::Png)?,
        };
        frame.render_micros = started.elapsed().as_micros().try_into().unwrap_or(u64::MAX);
        let frame = Arc::new(frame);
        let bytes = frame.rgba.capacity() + frame.png.capacity() + std::mem::size_of::<Frame>();
        self.frames.insert(key, frame.clone(), bytes);
        Ok(frame)
    }
}
fn state_key(scene: &Scene, time: Fraction, options: &Options) -> Result<StateKey, String> {
    Ok(StateKey {
        scene: scene.key.clone(),
        font_epoch: sanim_data::font::system_font_epoch(),
        time: scene.times.canonical(time)?,
        width: options.logical_width,
        height: options.logical_height,
    })
}
fn record_key(scene: &Scene, time: Fraction, options: &Options) -> Result<RecordKey, String> {
    Ok(RecordKey {
        state: state_key(scene, time, options)?,
        width: options.width,
        height: options.height,
        max_work: options.max_work,
        max_pixels: options.max_pixels,
    })
}
fn state_bytes(elements: &Vec<Element>, max_depth: usize) -> Result<usize, String> {
    let mut bytes = usize::MAX;
    let mut items = usize::MAX;
    let mut fonts = sanim_core::value_budget::Budget::default();
    for element in elements {
        sanim_core::value_budget::charge_with_budget(
            &Value::Element(Arc::new(element.clone())),
            &mut bytes,
            &mut items,
            max_depth,
            &mut fonts,
        )?;
    }
    Ok(
        usize::MAX - bytes
            + (elements.capacity() - elements.len()) * std::mem::size_of::<Element>(),
    )
}
