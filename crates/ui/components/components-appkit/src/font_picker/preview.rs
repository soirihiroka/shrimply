use super::FontPickerItem;
use skia_safe::{Color, Font, Paint, Typeface, surfaces};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, OnceLock, mpsc};

pub(super) const SPECIMEN_EDGE: f64 = 180.0;
const SPECIMEN_SIZE: f32 = 100.0;
const SPECIMEN_INSET: f32 = 18.0;
const CACHE_ENTRIES: usize = 128;
const WORKERS: usize = 2;
pub(super) type Loader = Arc<dyn Fn(&FontPickerItem) -> Result<Typeface, String> + Send + Sync>;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct Key {
    pub id: String,
    pub scale: u32,
    pub dark: bool,
}

type Pixels = Arc<Vec<u8>>;
type Cache = VecDeque<(Key, Pixels)>;
static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();

pub(super) struct Previews {
    jobs: Option<mpsc::SyncSender<(Key, FontPickerItem)>>,
    results: mpsc::Receiver<(Key, Result<Vec<u8>, String>)>,
    wanted: Arc<Mutex<HashSet<Key>>>,
    pending: HashSet<Key>,
    failures: HashMap<Key, String>,
}

impl Previews {
    pub fn new(load: Loader) -> Self {
        let (jobs, receiver) = mpsc::sync_channel::<(Key, FontPickerItem)>(WORKERS);
        let receiver = Arc::new(Mutex::new(receiver));
        let (sender, results) = mpsc::channel();
        let wanted = Arc::new(Mutex::new(HashSet::new()));
        for _ in 0..WORKERS {
            let receiver = receiver.clone();
            let sender = sender.clone();
            let wanted = wanted.clone();
            let load = load.clone();
            std::thread::spawn(move || {
                loop {
                    let Ok((key, item)) = receiver.lock().expect("preview receiver lock").recv()
                    else {
                        break;
                    };
                    let result = if wanted.lock().expect("preview viewport lock").contains(&key) {
                        load(&item).and_then(|face| render(face, &key))
                    } else {
                        Err(String::new())
                    };
                    if sender.send((key, result)).is_err() {
                        break;
                    }
                }
            });
        }
        Self {
            jobs: Some(jobs),
            results,
            wanted,
            pending: HashSet::new(),
            failures: HashMap::new(),
        }
    }

    pub fn visible(&mut self, keys: HashSet<Key>) -> HashSet<Key> {
        let mut changed = HashSet::new();
        self.failures.retain(|key, _| keys.contains(key));
        *self.wanted.lock().expect("preview viewport lock") = keys;
        while let Ok((key, result)) = self.results.try_recv() {
            self.pending.remove(&key);
            match result {
                Ok(bytes) => {
                    changed.insert(key.clone());
                    let mut cache = CACHE
                        .get_or_init(Mutex::default)
                        .lock()
                        .expect("preview cache lock");
                    cache.retain(|(cached, _)| cached != &key);
                    cache.push_front((key, Arc::new(bytes)));
                    cache.truncate(CACHE_ENTRIES);
                }
                Err(error) if !error.is_empty() => {
                    changed.insert(key.clone());
                    self.failures.insert(key, error);
                }
                Err(_) => {}
            }
        }
        changed
    }

    pub fn cached(&self, key: &Key) -> Option<Result<Pixels, String>> {
        let mut cache = CACHE
            .get_or_init(Mutex::default)
            .lock()
            .expect("preview cache lock");
        if let Some(index) = cache.iter().position(|(cached, _)| cached == key) {
            let entry = cache.remove(index).expect("cached preview index");
            let bytes = entry.1.clone();
            cache.push_front(entry);
            return Some(Ok(bytes));
        }
        drop(cache);
        self.failures.get(key).map(|error| Err(error.clone()))
    }

    pub fn request(&mut self, key: &Key, item: &FontPickerItem) {
        if self.cached(key).is_none()
            && !self.pending.contains(key)
            && let Some(jobs) = &self.jobs
            && jobs.try_send((key.clone(), item.clone())).is_ok()
        {
            self.pending.insert(key.clone());
        }
    }

    pub fn close(&mut self) {
        self.wanted.lock().expect("preview viewport lock").clear();
        self.jobs.take();
    }
}

impl Drop for Previews {
    fn drop(&mut self) {
        self.close();
    }
}

fn render(face: Typeface, key: &Key) -> Result<Vec<u8>, String> {
    // Check coverage before drawing: Skia must never substitute a different face.
    let specimen = [
        "Aa", "漢", "あ", "한", "ع", "א", "क", "অ", "ਕ", "ક", "க", "క", "ಕ", "മ", "සි", "ก", "ກ",
        "က", "ក", "Ꭰ", "⠿", "😀", "☎", "∑",
    ]
    .into_iter()
    .find(|sample| sample.chars().all(|c| face.unichar_to_glyph(c as i32) != 0))
    .map(str::to_string)
    .or_else(|| {
        (0x21..=0xffff)
            .filter_map(char::from_u32)
            .find(|c| {
                !c.is_control() && !c.is_whitespace() && face.unichar_to_glyph(*c as i32) != 0
            })
            .map(|c| c.to_string())
    })
    .ok_or_else(|| "This font has no displayable specimen".to_string())?;
    let scale = key.scale as f32;
    let edge = SPECIMEN_EDGE as f32;
    let pixels = (edge * scale) as i32;
    let mut surface =
        surfaces::raster_n32_premul((pixels, pixels)).ok_or("Could not allocate a font preview")?;
    let canvas = surface.canvas();
    canvas.clear(Color::TRANSPARENT);
    canvas.scale((scale, scale));
    let mut paint = Paint::default();
    paint.set_anti_alias(true);
    paint.set_color(if key.dark {
        Color::from_rgb(235, 235, 235)
    } else {
        Color::from_rgb(40, 40, 40)
    });
    let mut font = Font::new(face, SPECIMEN_SIZE);
    let (_, bounds) = font.measure_str(&specimen, Some(&paint));
    let available = edge - SPECIMEN_INSET * 2.0;
    let fit = (available / bounds.width().max(1.0))
        .min(available / bounds.height().max(1.0))
        .min(1.0);
    font.set_size(SPECIMEN_SIZE * fit);
    let (_, bounds) = font.measure_str(&specimen, Some(&paint));
    canvas.draw_str(
        &specimen,
        (
            (edge - bounds.width()) / 2.0 - bounds.left,
            (edge - bounds.height()) / 2.0 - bounds.top,
        ),
        &font,
        &paint,
    );
    skia_safe::png_encoder::encode_image(None, &surface.image_snapshot(), &Default::default())
        .map(|data| data.as_bytes().to_vec())
        .ok_or_else(|| "Could not encode a font preview".to_string())
}
