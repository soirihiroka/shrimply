//! Actual font data for native preview engines with a different font registry.
use super::{FontFamily, FontSource, GoogleFamily};
use std::{
    collections::{HashMap, VecDeque},
    hash::{Hash, Hasher},
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock, Weak},
};

const MAX_PREVIEW_FAMILIES: usize = 128;
type Key = (FontSource, String, i64);

struct FontFile {
    _directory: tempfile::TempDir,
    path: PathBuf,
}
struct Face {
    file: Arc<FontFile>,
    index: usize,
}

/// Keeps advertised font bytes alive until the native host adopts a newer document.
#[derive(Clone)]
pub struct NativeFontPreview {
    pub path: PathBuf,
    pub index: usize,
    _file: Arc<FontFile>,
}
#[derive(Default)]
struct Cache {
    faces: HashMap<Key, Face>,
    files: HashMap<u64, Weak<FontFile>>,
    order: VecDeque<Key>,
}
static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();

pub fn native_preview_source(family: &FontFamily) -> Option<NativeFontPreview> {
    let cache = CACHE
        .get_or_init(Mutex::default)
        .lock()
        .expect("font preview cache poisoned");
    let face = cache.faces.get(&key(family))?;
    Some(NativeFontPreview {
        path: face.file.path.clone(),
        index: face.index,
        _file: face.file.clone(),
    })
}

pub fn native_preview_ready(family: &FontFamily) -> bool {
    let key = key(family);
    let mut cache = CACHE
        .get_or_init(Mutex::default)
        .lock()
        .expect("font preview cache poisoned");
    if !cache.faces.contains_key(&key) {
        return false;
    }
    cache.order.retain(|entry| entry != &key);
    cache.order.push_back(key);
    true
}

pub fn prepare_native_preview(
    family: &FontFamily,
    lookup: Option<&GoogleFamily>,
) -> Result<(), String> {
    if native_preview_ready(family) {
        return Ok(());
    }
    let typeface = match family.source {
        FontSource::Local => skia_safe::FontMgr::new()
            .match_family_style(&family.name, skia_safe::FontStyle::default())
            .ok_or("The installed font family is unavailable")?,
        FontSource::Google if family.revision < 0 => super::preview_google_family(
            lookup
                .filter(|lookup| lookup.name.eq_ignore_ascii_case(&family.name))
                .ok_or("The Google font preview changed")?,
        )?,
        FontSource::Google => super::preview_typeface(&family.name)?,
    };
    let (data, index) = typeface
        .to_font_bytes()
        .ok_or("The font does not expose preview data")?;
    let index = index as usize;
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    data.hash(&mut hash);
    let digest = hash.finish();
    let shared = CACHE.get_or_init(Mutex::default);
    let existing = shared
        .lock()
        .expect("font preview cache poisoned")
        .files
        .get(&digest)
        .and_then(Weak::upgrade);
    let prepared = if let Some(file) = existing {
        file
    } else {
        let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
        let path = directory.path().join("preview.font");
        std::fs::write(&path, data).map_err(|error| error.to_string())?;
        Arc::new(FontFile {
            _directory: directory,
            path,
        })
    };
    let mut cache = shared.lock().expect("font preview cache poisoned");
    let file = cache
        .files
        .get(&digest)
        .and_then(Weak::upgrade)
        .unwrap_or_else(|| prepared.clone());
    cache.files.insert(digest, Arc::downgrade(&file));
    let key = key(family);
    let mut evicted = Vec::new();
    if let Some(old) = cache.faces.insert(key.clone(), Face { file, index }) {
        evicted.push(old);
    }
    cache.order.retain(|entry| entry != &key);
    cache.order.push_back(key);
    while cache.faces.len() > MAX_PREVIEW_FAMILIES {
        let oldest = cache
            .order
            .pop_front()
            .expect("cached preview has an access order");
        if let Some(face) = cache.faces.remove(&oldest) {
            evicted.push(face);
        }
    }
    cache.files.retain(|_, file| file.strong_count() > 0);
    drop(cache);
    drop(evicted);
    Ok(())
}

fn key(family: &FontFamily) -> Key {
    (family.source, family.name.to_lowercase(), family.revision)
}
