use hashbrown::{HashMap, HashSet};
use std::hash::Hash;
use std::ops::Range;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use cached::{Cached, stores::LruCache};
use skia_safe::{Canvas, Font, FontHinting, Paint, Typeface, font};

use crate::{Color, Rect, Vec2, button, spinner};

pub const CELL_HEIGHT: f32 = 224.0;
const OUTER_PADDING: f32 = 16.0;
const CELL_GAP: f32 = 8.0;
const MIN_CELL_WIDTH: f32 = 240.0;
const SPECIMEN: &str = "Aa";
const SPECIMEN_SIZE: f32 = 128.0;
const LABEL_SIZE: f32 = 15.0;
const SPINNER_SIZE: f32 = 38.0;

struct LoadJob<K> {
    key: K,
    generation: u64,
}

struct LoadResult<K> {
    key: K,
    generation: u64,
    typeface: Result<Typeface, String>,
}

pub struct FontLoader<K> {
    jobs: async_channel::Sender<LoadJob<K>>,
    results: async_channel::Receiver<LoadResult<K>>,
    label_result: async_channel::Receiver<Result<Typeface, String>>,
    label_typeface: Option<Typeface>,
    label_pending: bool,
    visible: Arc<Mutex<HashMap<K, u64>>>,
    current_visible: HashSet<K>,
    pending: HashMap<K, u64>,
    failed: HashSet<K>,
    cache: LruCache<K, Typeface>,
    generation: u64,
}

impl<K> FontLoader<K>
where
    K: Clone + Eq + Hash + Send + 'static,
{
    pub fn new(
        worker_count: usize,
        cache_entries: usize,
        load: impl Fn(&K) -> Result<Typeface, String> + Send + Sync + 'static,
        load_label: impl FnOnce() -> Result<Typeface, String> + Send + 'static,
    ) -> Self {
        let worker_count = worker_count.max(1);
        let (jobs, job_receiver) = async_channel::bounded::<LoadJob<K>>(worker_count);
        let (result_sender, results) = async_channel::unbounded::<LoadResult<K>>();
        let visible = Arc::new(Mutex::new(HashMap::new()));
        let (label_sender, label_result) = async_channel::bounded(1);
        thread::spawn(move || {
            let _ = label_sender.send_blocking(load_label());
        });
        let load = Arc::new(load);
        for _ in 0..worker_count {
            let jobs = job_receiver.clone();
            let results = result_sender.clone();
            let visible = visible.clone();
            let load = load.clone();
            thread::spawn(move || {
                while let Ok(job) = jobs.recv_blocking() {
                    let wanted = visible
                        .lock()
                        .unwrap_or_else(|_| panic!("visible font lock died"))
                        .get(&job.key)
                        .is_some_and(|generation| *generation == job.generation);
                    let typeface = if wanted {
                        load(&job.key)
                    } else {
                        Err("font left the viewport".to_string())
                    };
                    let _ = results.send_blocking(LoadResult {
                        key: job.key,
                        generation: job.generation,
                        typeface,
                    });
                }
            });
        }
        Self {
            jobs,
            results,
            label_result,
            label_typeface: None,
            label_pending: true,
            visible,
            current_visible: HashSet::new(),
            pending: HashMap::new(),
            failed: HashSet::new(),
            cache: LruCache::builder()
                .max_size(cache_entries.max(1))
                .build()
                .expect("valid font cache size"),
            generation: 0,
        }
    }

    pub fn prepare_visible(
        &mut self,
        width: f32,
        scroll: f32,
        viewport_height: f32,
        item_count: usize,
        mut key_at: impl FnMut(usize) -> K,
    ) -> Range<usize> {
        self.drain_results();
        let range = visible_range(width, scroll, viewport_height, item_count);
        let next = range.clone().map(&mut key_at).collect::<HashSet<_>>();
        if next != self.current_visible {
            self.generation = self.generation.wrapping_add(1);
            self.current_visible = next;
            self.pending
                .retain(|key, _| self.current_visible.contains(key));
            let mut visible = self
                .visible
                .lock()
                .unwrap_or_else(|_| panic!("visible font lock died"));
            visible.clear();
            visible.extend(
                self.current_visible
                    .iter()
                    .cloned()
                    .map(|key| (key, self.generation)),
            );
        }
        range
    }

    pub fn typeface(&mut self, key: &K) -> Option<Typeface> {
        self.drain_results();
        if let Some(typeface) = self.cache.cache_get(key).cloned() {
            return Some(typeface);
        }
        if self.current_visible.contains(key)
            && !self.pending.contains_key(key)
            && !self.failed.contains(key)
        {
            let job = LoadJob {
                key: key.clone(),
                generation: self.generation,
            };
            if self.jobs.try_send(job).is_ok() {
                self.pending.insert(key.clone(), self.generation);
            }
        }
        None
    }

    pub fn is_loading(&self) -> bool {
        self.label_pending || !self.pending.is_empty()
    }

    pub fn label_typeface(&mut self) -> Option<Typeface> {
        self.drain_results();
        self.label_typeface.clone()
    }

    fn drain_results(&mut self) {
        if self.label_pending
            && let Ok(result) = self.label_result.try_recv()
        {
            self.label_pending = false;
            self.label_typeface = result.ok();
        }
        while let Ok(result) = self.results.try_recv() {
            if self.pending.get(&result.key) == Some(&result.generation) {
                self.pending.remove(&result.key);
            }
            if self.generation == result.generation && self.current_visible.contains(&result.key) {
                match result.typeface {
                    Ok(typeface) => {
                        self.cache.cache_set(result.key, typeface);
                    }
                    Err(_) => {
                        self.failed.insert(result.key);
                    }
                }
            }
        }
    }
}

pub struct Specimen<'a> {
    pub label: &'a str,
    pub label_typeface: Option<&'a Typeface>,
    pub typeface: Option<&'a Typeface>,
    pub selected: bool,
    pub button: &'a mut button::Button,
}

pub fn columns(width: f32) -> usize {
    (((width - OUTER_PADDING * 2.0 + CELL_GAP) / (MIN_CELL_WIDTH + CELL_GAP)).floor() as usize)
        .max(1)
}

pub fn content_height(width: f32, item_count: usize) -> f32 {
    let rows = item_count.div_ceil(columns(width));
    OUTER_PADDING * 2.0 + rows as f32 * CELL_HEIGHT
}

pub fn visible_range(
    width: f32,
    scroll: f32,
    viewport_height: f32,
    item_count: usize,
) -> Range<usize> {
    let columns = columns(width);
    let first_row = ((scroll - OUTER_PADDING).max(0.0) / CELL_HEIGHT).floor() as usize;
    let last_row =
        ((scroll + viewport_height - OUTER_PADDING).max(0.0) / CELL_HEIGHT).ceil() as usize;
    first_row.saturating_mul(columns).min(item_count)
        ..last_row.saturating_mul(columns).min(item_count)
}

pub fn cell_bounds(width: f32, scroll: f32, index: usize) -> Rect {
    let columns = columns(width);
    let usable_width = (width - OUTER_PADDING * 2.0).max(1.0);
    let cell_width = (usable_width - CELL_GAP * (columns - 1) as f32) / columns as f32;
    let column = index % columns;
    let row = index / columns;
    Rect::from_xywh(
        OUTER_PADDING + column as f32 * (cell_width + CELL_GAP),
        OUTER_PADDING + row as f32 * CELL_HEIGHT - scroll,
        cell_width,
        CELL_HEIGHT - CELL_GAP,
    )
}

pub fn hit_test(width: f32, scroll: f32, point: Vec2, item_count: usize) -> Option<usize> {
    let columns = columns(width);
    let usable_width = (width - OUTER_PADDING * 2.0).max(1.0);
    let cell_width = (usable_width - CELL_GAP * (columns - 1) as f32) / columns as f32;
    let x = point.x - OUTER_PADDING;
    let y = point.y + scroll - OUTER_PADDING;
    if x < 0.0 || y < 0.0 {
        return None;
    }
    let column = (x / (cell_width + CELL_GAP)).floor() as usize;
    let row = (y / CELL_HEIGHT).floor() as usize;
    if column >= columns {
        return None;
    }
    let index = row.checked_mul(columns)?.checked_add(column)?;
    (index < item_count && button::Button::hit_test(cell_bounds(width, scroll, index), point))
        .then_some(index)
}

pub fn draw_specimen(
    canvas: &Canvas,
    bounds: Rect,
    specimen: Specimen<'_>,
    foreground: Color,
    elapsed: Duration,
) -> bool {
    let draw = specimen.button.draw(
        canvas,
        button::Config::new(bounds, foreground)
            .style(button::Style::Flat)
            .radius(button::Radius::Fixed(12.0))
            .padding(button::Padding::None)
            .checked(specimen.selected),
    );
    let specimen_area = Rect::from_xywh(
        bounds.left() + 8.0,
        bounds.top() + 6.0,
        (bounds.width() - 16.0).max(0.0),
        (bounds.height() - 48.0).max(0.0),
    );
    if let Some(typeface) = specimen.typeface {
        draw_centered_text(
            canvas,
            SPECIMEN,
            typeface,
            SPECIMEN_SIZE.min(specimen_area.height() * 0.78),
            specimen_area,
            foreground,
        );
    } else {
        spinner::draw(
            canvas,
            spinner::Config::new(Rect::from_xywh(
                specimen_area.left() + (specimen_area.width() - SPINNER_SIZE) / 2.0,
                specimen_area.top() + (specimen_area.height() - SPINNER_SIZE) / 2.0,
                SPINNER_SIZE,
                SPINNER_SIZE,
            ))
            .color(foreground)
            .elapsed(elapsed),
        );
    }

    if let Some(typeface) = specimen.label_typeface {
        draw_centered_text(
            canvas,
            specimen.label,
            typeface,
            LABEL_SIZE,
            Rect::from_xywh(
                bounds.left() + 8.0,
                bounds.bottom() - 38.0,
                (bounds.width() - 16.0).max(0.0),
                28.0,
            ),
            foreground,
        );
    }
    draw.animating
}

fn draw_centered_text(
    canvas: &Canvas,
    text: &str,
    typeface: &Typeface,
    size: f32,
    bounds: Rect,
    color: Color,
) {
    let mut font = Font::from_typeface(typeface.clone(), size.max(1.0));
    font.set_subpixel(true);
    font.set_linear_metrics(true);
    font.set_embedded_bitmaps(false);
    font.set_baseline_snap(false);
    font.set_hinting(FontHinting::Slight);
    font.set_edging(font::Edging::AntiAlias);
    let (width, _) = font.measure_str(text, None);
    if width > bounds.width() && width > 0.0 {
        font.set_size((font.size() * bounds.width() / width).max(8.0));
    }
    let (_, measured) = font.measure_str(text, None);
    let x = bounds.left() + (bounds.width() - measured.width()) / 2.0 - measured.left;
    let y = bounds.top() + (bounds.height() - measured.height()) / 2.0 - measured.top;
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_color(color);
    canvas.draw_str(text, (x, y), &font, &paint);
}
