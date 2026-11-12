//! Skia command recording and explicit headless raster output. No SVG or GUI toolkit.
use sanim_core::{math::to_f64, *};
use skia_safe::{Canvas, Picture, PictureRecorder, Rect};
use std::{collections::BTreeMap, sync::Arc};
mod draw;

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
    animation: Animation,
    key: String,
    paths: BTreeMap<String, shrimply_drawing_skia::path::PreparedPath>,
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
    picture: Picture,
}
impl Recording {
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

#[derive(Default)]
pub struct Renderer {
    recordings: BTreeMap<String, Arc<Recording>>,
    frames: BTreeMap<String, Arc<Frame>>,
    pub cache_hits: u64,
    pub geometry_cache_hits: u64,
    geometry_cache: BTreeMap<String, shrimply_drawing_skia::path::PreparedPath>,
    geometry_cache_bytes: usize,
}
impl Renderer {
    /// Preserve immutable path geometry across source, style, timing and viewport edits.
    pub fn prepare(&mut self, animation: Animation) -> Result<Scene, String> {
        let key = digest(&serde_json::to_vec(&animation).map_err(|error| error.to_string())?);
        let mut paths = BTreeMap::new();
        for layer in &animation.layers {
            draw::prepare_paths(&layer.element, &mut paths, self)?;
        }
        Ok(Scene {
            animation,
            key,
            paths,
        })
    }
    pub fn clear_cache(&mut self) {
        self.recordings.clear();
        self.frames.clear();
        self.geometry_cache.clear();
        self.geometry_cache_bytes = 0;
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
        let key = cache_key(scene, time, options, false);
        if let Some(recording) = self.recordings.get(&key) {
            self.cache_hits += 1;
            return Ok(recording.clone());
        }
        let elements =
            scene
                .animation
                .layout(time, options.logical_width, options.logical_height)?;
        let mut recorder = PictureRecorder::new();
        let canvas = recorder.begin_recording(
            Rect::from_wh(options.width as f32, options.height as f32),
            true,
        );
        canvas.clip_rect(
            Rect::from_wh(options.width as f32, options.height as f32),
            None,
            false,
        );
        let logical_width = to_f64(options.logical_width)? as f32;
        let logical_height = to_f64(options.logical_height)? as f32;
        canvas.scale((
            options.width as f32 / logical_width,
            options.height as f32 / logical_height,
        ));
        let background = draw::paint(&scene.animation.background, Fraction::from(1))?;
        canvas.draw_rect(Rect::from_wh(logical_width, logical_height), &background);
        let mut drawing = draw::Drawing {
            paths: &scene.paths,
            pixels,
            work: options.max_work,
        };
        for element in &elements {
            draw::element_draw(canvas, element, Reveal::default(), &mut drawing)?;
        }
        let picture = recorder
            .finish_recording_as_picture(None)
            .ok_or("Skia recording failed")?;
        let recording = Arc::new(Recording {
            width: options.width,
            height: options.height,
            picture,
            record_micros: started.elapsed().as_micros().try_into().unwrap_or(u64::MAX),
        });
        if self.recordings.len() >= 8 {
            self.recordings.clear();
        }
        self.recordings.insert(key, recording.clone());
        Ok(recording)
    }
    pub fn render(
        &mut self,
        scene: &Scene,
        time: Fraction,
        options: &Options,
    ) -> Result<Arc<Frame>, String> {
        let started = std::time::Instant::now();
        // Validate limits through record even when a raster result is cached.
        let recording = self.record(scene, time, options)?;
        let key = cache_key(scene, time, options, options.encode_png);
        if let Some(frame) = self.frames.get(&key) {
            return Ok(frame.clone());
        }
        let mut frame = recording.rasterize(options.encode_png)?;
        frame.render_micros = started.elapsed().as_micros().try_into().unwrap_or(u64::MAX);
        let frame = Arc::new(frame);
        if self.frames.len() >= 8 {
            self.frames.clear();
        }
        self.frames.insert(key, frame.clone());
        Ok(frame)
    }
}
fn cache_key(scene: &Scene, time: Fraction, options: &Options, png: bool) -> String {
    digest(
        format!(
            "{}:{time}:{}:{}:{}:{}:{}:{}:{png}",
            scene.key,
            options.width,
            options.height,
            options.logical_width,
            options.logical_height,
            options.max_work,
            options.max_pixels
        )
        .as_bytes(),
    )
}
