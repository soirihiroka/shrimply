//! One application registry and one synchronized Pango map per render thread.
use crate::{Font, descriptor::validate_family};
use pango::prelude::*;
use serde::{Deserialize, Serialize};
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock, RwLock, Weak,
        atomic::{AtomicU64, Ordering},
    },
};

static REVISION: AtomicU64 = AtomicU64::new(0);
static FILES: OnceLock<RwLock<Vec<PathBuf>>> = OnceLock::new();
pub type FontChange = Arc<dyn Fn() + Send + Sync>;
type Subscribers = Mutex<Vec<Weak<dyn Fn() + Send + Sync>>>;
static SUBSCRIBERS: OnceLock<Subscribers> = OnceLock::new();

/// Keep the returned handle alive to receive registry changes. Dropped hosts
/// disappear on the next subscription/change, with no worker thread or polling.
pub fn subscribe(callback: FontChange) -> FontChange {
    let mut subscribers = SUBSCRIBERS
        .get_or_init(Default::default)
        .lock()
        .expect("font change subscribers lock died");
    subscribers.retain(|callback| callback.strong_count() > 0);
    subscribers.push(Arc::downgrade(&callback));
    callback
}

fn notify_subscribers() {
    let callbacks = {
        let mut subscribers = SUBSCRIBERS
            .get_or_init(Default::default)
            .lock()
            .expect("font change subscribers lock died");
        let mut callbacks = Vec::new();
        subscribers.retain(|callback| match callback.upgrade() {
            Some(callback) => {
                callbacks.push(callback);
                true
            }
            None => false,
        });
        callbacks
    };
    for callback in callbacks {
        callback();
    }
}

struct ThreadMap {
    map: pango::FontMap,
    revision: u64,
    registered: usize,
}

thread_local! {
    static MAP: RefCell<Option<ThreadMap>> = const { RefCell::new(None) };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FontEpoch {
    pub revision: u64,
    pub serial: u32,
}

/// File-capable backends also resolve installed native families. Prefer these so
/// downloaded application fonts work on macOS without CoreText add-file errors.
pub fn file_map() -> Result<pango::FontMap, String> {
    [cairo::FontType::FontTypeWin32, cairo::FontType::FontTypeFt]
        .into_iter()
        .find_map(pangocairo::FontMap::for_font_type)
        .ok_or_else(|| "Pango has no backend supporting application font files; install Pango with Fontconfig/FreeType or Win32 support".into())
}

fn with_map<T>(f: impl FnOnce(&ThreadMap) -> T) -> Result<T, String> {
    MAP.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            let map = file_map().or_else(|_| {
                pangocairo::FontMap::for_font_type(cairo::FontType::FontTypeQuartz)
                    .ok_or("Pango has no native font backend")
            })?;
            *slot = Some(ThreadMap {
                map,
                revision: u64::MAX,
                registered: 0,
            });
        }
        let state = slot.as_mut().expect("initialized native font map");
        let revision = REVISION.load(Ordering::Acquire);
        if state.revision != revision {
            if let Some(files) = FILES.get() {
                let files = files.read().expect("application font registry lock died");
                for path in &files[state.registered..] {
                    state.map.add_font_file(path).map_err(|error| {
                        format!("could not register font {}: {error}", path.display())
                    })?;
                    state.registered += 1;
                }
            }
            state.revision = revision;
        }
        Ok(f(state))
    })
}

pub fn system_map() -> Result<pango::FontMap, String> {
    with_map(|state| state.map.clone())
}

pub fn system_font_epoch() -> Option<FontEpoch> {
    with_map(|state| FontEpoch {
        revision: state.revision,
        serial: state.map.serial(),
    })
    .ok()
}

pub fn register_application_font(path: impl AsRef<Path>) -> Result<(), String> {
    register_application_fonts([path])
}

/// Validate the whole family before publishing any of its files. Failed font
/// activation can then release its temporary directory and be retried safely.
pub fn register_application_fonts<P: AsRef<Path>>(
    paths: impl IntoIterator<Item = P>,
) -> Result<(), String> {
    let mut paths = paths
        .into_iter()
        .map(|path| {
            path.as_ref().canonicalize().map_err(|error| {
                format!("could not open font {}: {error}", path.as_ref().display())
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let files = FILES.get_or_init(|| RwLock::new(Vec::new()));
    {
        let mut files = files.write().expect("application font registry lock died");
        paths.sort();
        paths.dedup();
        paths.retain(|path| !files.contains(path));
        if paths.is_empty() {
            return Ok(());
        }
        let map = file_map()?;
        for path in &paths {
            map.add_font_file(path)
                .map_err(|error| format!("could not register font {}: {error}", path.display()))?;
        }
        files.extend(paths);
        REVISION.fetch_add(1, Ordering::Release);
    }
    system_map()?;
    notify_subscribers();
    Ok(())
}

/// Ordered CSS-style family fallback shared by editor text, 3D text, and Sanim.
pub fn set_families<'a>(
    description: &mut pango::FontDescription,
    families: impl IntoIterator<Item = &'a str>,
) -> Result<(), String> {
    let families = families.into_iter().collect::<Vec<_>>();
    for family in &families {
        validate_family(family)?;
    }
    let names = families.join(",");
    description.set_family(if families.is_empty() {
        Font::DEFAULT_FAMILY
    } else {
        &names
    });
    Ok(())
}

pub fn layout(description: &pango::FontDescription) -> Result<pango::Layout, String> {
    Ok(layout_for(&system_map()?, description, true))
}

fn layout_for(
    map: &pango::FontMap,
    description: &pango::FontDescription,
    fallback: bool,
) -> pango::Layout {
    let context = map.create_context();
    context.set_round_glyph_positions(false);
    context.set_language(Some(&pango::Language::from_string("und")));
    let layout = pango::Layout::new(&context);
    layout.set_font_description(Some(description));
    let attrs = pango::AttrList::new();
    attrs.insert(pango::AttrInt::new_fallback(fallback));
    layout.set_attributes(Some(&attrs));
    layout
}

pub fn validate_asset(bytes: &[u8]) -> Result<(), String> {
    typeface(bytes).map(|_| ())
}

fn typeface(bytes: &[u8]) -> Result<skia_safe::Typeface, String> {
    skia_safe::FontMgr::new()
        .new_from_data(skia_safe::Data::new_copy(bytes), None)
        .ok_or_else(|| "invalid font resource: Skia could not parse this font".into())
}

/// A private asset map keeps bundled files isolated from installed faces with
/// the same name. Its temporary file lives as long as the layout can use it.
pub struct PreparedFont {
    pub layout: pango::Layout,
    _asset: Option<tempfile::NamedTempFile>,
}

pub fn prepare(font: &Font, size: f64) -> Result<PreparedFont, String> {
    use std::io::Write;
    font.validate()?;
    if !size.is_finite() || size <= 0. || size * f64::from(pango::SCALE) > f64::from(i32::MAX) {
        return Err("font size exceeds Pango layout range".into());
    }
    let mut description = pango::FontDescription::new();
    description.set_absolute_size(size * f64::from(pango::SCALE));
    let mut asset = None;
    let layout = match font {
        Font::Family(families) => {
            set_families(&mut description, families.iter().map(String::as_str))?;
            layout(&description)?
        }
        Font::Asset(bytes) => {
            let face = typeface(bytes)?;
            let mut file = tempfile::Builder::new()
                .suffix(".ttf")
                .tempfile()
                .map_err(|error| error.to_string())?;
            file.write_all(bytes).map_err(|error| error.to_string())?;
            file.flush().map_err(|error| error.to_string())?;
            let map = file_map()?;
            map.add_font_file(file.path())
                .map_err(|error| format!("Pango could not load font asset: {error}"))?;
            description.set_family(&face.family_name());
            let style = face.font_style();
            description.set_stretch(pango::Stretch::__Unknown((*style.width() - 1).clamp(0, 8)));
            description.set_weight(pango::Weight::__Unknown(*style.weight()));
            description.set_style(match style.slant() {
                skia_safe::font_style::Slant::Upright => pango::Style::Normal,
                skia_safe::font_style::Slant::Italic => pango::Style::Italic,
                skia_safe::font_style::Slant::Oblique => pango::Style::Oblique,
            });
            let layout = layout_for(&map, &description, false);
            asset = Some(file);
            layout
        }
    };
    Ok(PreparedFont {
        layout,
        _asset: asset,
    })
}

pub fn glyph_is_color(glyph: &pango::GlyphInfo) -> bool {
    const COLOR_BIT: u32 = 1 << 1;
    // PangoGlyphVisAttr's C bitfield storage exposed by pango-rs.
    unsafe { (*glyph.as_ptr()).attr.is_cluster_start & COLOR_BIT != 0 }
}
