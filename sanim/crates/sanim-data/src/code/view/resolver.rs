//! Resolve changed code descriptors; timeline sampling never interprets Sanim source.
use super::prepare_layout;
use sanim_core::{
    Element, Font, Fraction, Geometry, List, Number, Type, Unit, Value,
    cache::Cache,
    code::{
        Document,
        view::{Options, Snapshot, Token},
    },
    math,
};
use serde::Serialize;
use std::sync::{Arc, Mutex, OnceLock, Weak};

pub(super) const MAX_LAYOUT_BYTES: usize = 64 * 1024 * 1024;
const MAX_LAYOUT_ENTRIES: usize = 256;
pub(super) const MAX_LAYOUT_ITEMS: usize = 1_000_000;
pub(super) const MAX_LAYOUT_DEPTH: usize = 256;
const MAX_LAYOUT_HINT_BYTES: usize = 256 * 1024;

struct Endpoint {
    // Retains every Arc payload used by the native identity, including pre-parse annotations.
    _source: Arc<Document>,
    document: Arc<Document>,
    options: Arc<Options>,
    tokens: imbl::Vector<Token>,
    size: [f64; 2],
}

type Identity = (
    String,
    usize,
    Option<crate::font::FontEpoch>,
    sanim_core::text::Recipe,
);
struct Hint {
    snapshot: Weak<Snapshot>,
    key: String,
}
struct Layouts {
    content: Cache<String, Artifact>,
    identities: Cache<Identity, Hint>,
}

enum Artifact {
    Endpoint(Arc<Endpoint>),
    Flow(Arc<super::Flow>),
    Text {
        _source: Arc<str>,
        text: shrimply_fonts::native::shaping::Source,
    },
}
fn layouts() -> &'static Mutex<Layouts> {
    static CACHE: OnceLock<Mutex<Layouts>> = OnceLock::new();
    CACHE.get_or_init(|| {
        Mutex::new(Layouts {
            content: Cache::new(MAX_LAYOUT_BYTES, MAX_LAYOUT_ENTRIES),
            identities: Cache::new(MAX_LAYOUT_HINT_BYTES, MAX_LAYOUT_ENTRIES),
        })
    })
}
pub(super) fn flow(
    key: &str,
    value: Option<super::Flow>,
) -> Result<Option<Arc<super::Flow>>, String> {
    let mut cache = layouts().lock().expect("code layout cache poisoned");
    if let Some(artifact) = cache.content.get(&key.to_owned())
        && let Artifact::Flow(flow) = artifact.as_ref()
    {
        return Ok(Some(flow.clone()));
    }
    let Some(value) = value else {
        return Ok(None);
    };
    let bytes = value
        .retained_bytes()?
        .saturating_add(key.len())
        .saturating_add(std::mem::size_of::<Artifact>());
    if bytes > MAX_LAYOUT_BYTES {
        return Err("code source flow exceeds its retained byte budget".into());
    }
    let flow = Arc::new(value);
    cache.content.insert(
        key.to_owned(),
        Arc::new(Artifact::Flow(flow.clone())),
        bytes,
    );
    Ok(Some(flow))
}

pub(crate) fn normalized_text(
    source: Arc<str>,
    mode: sanim_core::text::WhiteSpace,
) -> Result<shrimply_fonts::native::shaping::Source, String> {
    let key = format!(
        "text:{}:{}",
        Arc::as_ptr(&source) as *const () as usize,
        mode.name()
    );
    if let Some(artifact) = layouts()
        .lock()
        .expect("native content cache poisoned")
        .content
        .get(&key)
        && let Artifact::Text { text, .. } = artifact.as_ref()
    {
        return Ok(text.clone());
    }
    let normalized = shrimply_fonts::native::shaping::Source::new(
        sanim_core::text::normalize_plain(&source, mode).into(),
    )?;
    let bytes = std::mem::size_of::<Artifact>()
        + 2 * key.len()
        + source.len()
        + normalized.retained_bytes();
    if bytes > MAX_LAYOUT_BYTES {
        return Err("normalized text exceeds its retained byte budget".into());
    }
    layouts()
        .lock()
        .expect("native content cache poisoned")
        .content
        .insert(
            key,
            Arc::new(Artifact::Text {
                _source: source,
                text: normalized.clone(),
            }),
            bytes,
        );
    Ok(normalized)
}

// Native-only identity: each cached artifact retains these immutable payloads,
// so addresses cannot be recycled while a matching cache entry is alive.
pub(super) fn document_identity(document: &Document) -> impl Serialize {
    (
        document.language,
        Arc::as_ptr(&document.text) as *const () as usize,
        document.text.len(),
        document.syntax_parsed,
        Arc::as_ptr(&document.highlights) as *const () as usize,
        document.highlights.len(),
        Arc::as_ptr(&document.scopes) as *const () as usize,
        document.scopes.len(),
    )
}

#[derive(Serialize)]
pub(super) enum FontKey<'a> {
    // The cached endpoint retains the asset, preventing address reuse under a hit.
    Asset(usize),
    Family(&'a Arc<[String]>),
}

pub(super) fn font_identity(font: &Font) -> FontKey<'_> {
    match font {
        Font::Asset(bytes) => FontKey::Asset(Arc::as_ptr(bytes) as usize),
        Font::Family(names) => FontKey::Family(names),
    }
}

fn endpoint(
    id: &str,
    snapshot: &Arc<Snapshot>,
    recipe: sanim_core::text::Recipe,
) -> Result<Arc<Endpoint>, String> {
    let cache = layouts();
    let epoch = crate::font::system_font_epoch();
    let identity = (id.to_owned(), Arc::as_ptr(snapshot) as usize, epoch, recipe);
    {
        let mut cache = cache.lock().expect("code layout cache poisoned");
        if let Some(hint) = cache.identities.get(&identity)
            && hint
                .snapshot
                .upgrade()
                .is_some_and(|root| Arc::ptr_eq(&root, snapshot))
            && let Some(artifact) = cache.content.get(&hint.key)
            && let Artifact::Endpoint(endpoint) = artifact.as_ref()
        {
            return Ok(endpoint.clone());
        }
    }
    let options = &snapshot.options;
    let font = font_identity(&options.font);
    let key = sanim_core::digest(
        &serde_json::to_vec(&(
            id,
            document_identity(&snapshot.document),
            &snapshot.document.path,
            options.visual_recipe(font),
            recipe,
            epoch,
            &snapshot.folded,
            &snapshot.selection,
        ))
        .map_err(|error| error.to_string())?,
    );
    let hint = Arc::new(Hint {
        snapshot: Arc::downgrade(snapshot),
        key: key.clone(),
    });
    let hint_bytes =
        std::mem::size_of::<Hint>() + std::mem::size_of::<Identity>() + 2 * id.len() + key.len();
    {
        let mut cache = cache.lock().expect("code layout cache poisoned");
        if let Some(artifact) = cache.content.get(&key)
            && let Artifact::Endpoint(endpoint) = artifact.as_ref()
        {
            cache.identities.insert(identity, hint, hint_bytes);
            return Ok(endpoint.clone());
        }
    }
    let document = Arc::new(crate::code::parse(snapshot.document.as_ref().clone())?);
    let prepared = prepare_layout(
        id,
        &document,
        options,
        &snapshot.folded,
        &snapshot.selection,
        recipe,
    )?;
    let tokens = prepared.tokens;
    // Shared value-budget traversal counts font assets once across the whole glyph list.
    let empty = tokens.is_empty();
    let children = List::typed(
        Type::Element,
        tokens
            .iter()
            .map(|token| Value::Element(token.element.clone()))
            .collect(),
    );
    let mut bytes = MAX_LAYOUT_BYTES;
    let mut items = MAX_LAYOUT_ITEMS;
    sanim_core::value_budget::charge(
        &Value::List(children),
        &mut bytes,
        &mut items,
        MAX_LAYOUT_DEPTH,
    )?;
    sanim_core::value_budget::charge(
        &Value::Code(document.clone()),
        &mut bytes,
        &mut items,
        MAX_LAYOUT_DEPTH,
    )?;
    if empty {
        sanim_core::value_budget::charge(
            &Value::Font(options.font.clone()),
            &mut bytes,
            &mut items,
            MAX_LAYOUT_DEPTH,
        )?;
    }
    let metadata = std::mem::size_of::<Endpoint>()
        + key.len()
        + tokens
            .iter()
            .map(|token| {
                std::mem::size_of::<Token>()
                    + token.key.len()
                    + std::mem::size_of_val(token.style.as_ref())
            })
            .sum::<usize>()
        + options.spans.len() * std::mem::size_of::<sanim_core::code::view::Override>()
        + options
            .theme
            .styles
            .iter()
            .map(|(name, style)| name.len() + std::mem::size_of_val(style))
            .sum::<usize>();
    bytes = bytes
        .checked_sub(metadata)
        .ok_or("code layout exceeds its retained byte budget")?;
    sanim_core::value_budget::charge(
        &Value::Code(snapshot.document.clone()),
        &mut bytes,
        &mut items,
        MAX_LAYOUT_DEPTH,
    )?;
    let endpoint = Arc::new(Endpoint {
        _source: snapshot.document.clone(),
        document,
        options: options.clone(),
        tokens,
        size: prepared.size,
    });
    let mut cache = cache.lock().expect("code layout cache poisoned");
    cache.content.insert(
        key,
        Arc::new(Artifact::Endpoint(endpoint.clone())),
        MAX_LAYOUT_BYTES - bytes + std::mem::size_of::<Artifact>(),
    );
    cache.identities.insert(identity, hint, hint_bytes);
    Ok(endpoint)
}

pub fn resolve_pending(element: &mut Element, _time: Fraction) -> Result<(), String> {
    let Geometry::Code {
        document,
        options,
        presentation,
        ..
    } = element.geometry.as_ref()
    else {
        return Ok(());
    };
    let recipe = element.layout.text.recipe(true);
    if !crate::requires_resolution(element) {
        return Ok(());
    }
    let (from, to, progress, appearance, raw_progress, animation) =
        if let Some(presentation) = presentation {
            (
                presentation.from.clone(),
                presentation.to.clone(),
                presentation.progress,
                presentation.appearance,
                presentation.raw_progress,
                presentation.animation,
            )
        } else {
            let snapshot = Arc::new(Snapshot {
                document: document.clone(),
                options: options.clone(),
                folded: imbl::OrdSet::new(),
                selection: Arc::from([]),
            });
            (
                snapshot.clone(),
                snapshot,
                1.into(),
                1.into(),
                1.into(),
                sanim_core::code::view::PatchAnimation::Morph,
            )
        };
    let (resolved, children, mut size) =
        if progress == 0.into() && appearance == 0.into() && raw_progress == 0.into() {
            let resolved = endpoint(&element.id, &from, recipe)?;
            let children = resolved
                .tokens
                .iter()
                .map(|token| {
                    let mut element = token.element.as_ref().clone();
                    element.position = token.position;
                    element
                })
                .collect();
            let size = resolved.size;
            (resolved, children, size)
        } else if progress == 1.into() && appearance == 1.into() && raw_progress == 1.into() {
            let resolved = endpoint(&element.id, &to, recipe)?;
            let children = resolved
                .tokens
                .iter()
                .map(|token| {
                    let mut element = token.element.as_ref().clone();
                    element.position = token.position;
                    element
                })
                .collect();
            let size = resolved.size;
            (resolved, children, size)
        } else {
            let from = endpoint(&element.id, &from, recipe)?;
            let to = endpoint(&element.id, &to, recipe)?;
            let children = super::transition_tokens(
                &from.tokens,
                &to.tokens,
                progress,
                appearance,
                raw_progress,
                animation,
            )?;
            let size = [
                math::motion_lerp(from.size[0], to.size[0], progress, true)?.max(0.),
                math::motion_lerp(from.size[1], to.size[1], progress, true)?.max(0.),
            ];
            (to, children, size)
        };
    if animation == sanim_core::code::view::PatchAnimation::Typewriter {
        // A wide early grapheme can exceed a proportional box-size interpolation.
        for child in &children {
            let bounds = sanim_core::verify::bounds(child)?;
            for (axis, dimension) in size.iter_mut().enumerate() {
                *dimension = dimension.max(bounds.max.values[axis]);
            }
        }
    }
    let padding = 2. * math::to_f64(element.layout.padding)?;
    for (target, size) in [
        (&mut element.layout.width, size[0]),
        (&mut element.layout.height, size[1]),
    ] {
        if target.is_none() {
            *target = Some(Number {
                value: math::from_f64(size + padding)?,
                unit: Unit::Length,
            });
        }
    }
    let Geometry::Code {
        document,
        options,
        children: current,
        pending,
        source_changed,
        ..
    } = Arc::make_mut(&mut element.geometry)
    else {
        unreachable!("pending CodeView descriptor")
    };
    *document = resolved.document.clone();
    *options = resolved.options.clone();
    *current = children;
    *pending = false;
    *source_changed = false;
    element.layout.text.resolved = Some(recipe);
    Ok(())
}

#[cfg(test)]
mod normalization_tests {
    use super::*;
    #[test]
    fn label_width_changes_share_normalization_without_allocating_source_maps() {
        let source: Arc<str> = "  A   paragraph\nwith source spaces  ".into();
        let first = normalized_text(source.clone(), sanim_core::text::WhiteSpace::Normal).unwrap();
        assert_eq!(first.text().as_ref(), "A paragraph with source spaces");
        for _ in 0..20 {
            assert!(Arc::ptr_eq(
                first.text(),
                normalized_text(source.clone(), sanim_core::text::WhiteSpace::Normal)
                    .unwrap()
                    .text()
            ));
        }
        let preserved = normalized_text(source, sanim_core::text::WhiteSpace::PreWrap).unwrap();
        assert_ne!(first.text().as_ref(), preserved.text().as_ref());
    }
}
