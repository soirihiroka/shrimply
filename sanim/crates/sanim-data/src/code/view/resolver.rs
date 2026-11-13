//! Resolve changed code descriptors; timeline sampling never interprets Sanim source.
use super::prepare;
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

const MAX_LAYOUT_BYTES: usize = 64 * 1024 * 1024;
const MAX_LAYOUT_ENTRIES: usize = 256;
const MAX_LAYOUT_ITEMS: usize = 1_000_000;
const MAX_LAYOUT_DEPTH: usize = 256;
const MAX_LAYOUT_HINT_BYTES: usize = 256 * 1024;

struct Endpoint {
    document: Arc<Document>,
    options: Arc<Options>,
    tokens: imbl::Vector<Token>,
    size: [f64; 2],
}

type Identity = (String, usize, Option<crate::font::FontEpoch>);
struct Hint {
    snapshot: Weak<Snapshot>,
    key: String,
}
struct Layouts {
    content: Cache<String, Endpoint>,
    identities: Cache<Identity, Hint>,
}

#[derive(Serialize)]
enum FontKey<'a> {
    // The cached endpoint retains the asset, preventing address reuse under a hit.
    Asset(usize),
    Family(&'a Arc<[String]>),
}

fn endpoint(id: &str, snapshot: &Arc<Snapshot>) -> Result<Arc<Endpoint>, String> {
    static CACHE: OnceLock<Mutex<Layouts>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| {
        Mutex::new(Layouts {
            content: Cache::new(MAX_LAYOUT_BYTES, MAX_LAYOUT_ENTRIES),
            identities: Cache::new(MAX_LAYOUT_HINT_BYTES, MAX_LAYOUT_ENTRIES),
        })
    });
    let epoch = crate::font::system_font_epoch();
    let identity = (id.to_owned(), Arc::as_ptr(snapshot) as usize, epoch);
    {
        let mut cache = cache.lock().expect("code layout cache poisoned");
        if let Some(hint) = cache.identities.get(&identity)
            && hint
                .snapshot
                .upgrade()
                .is_some_and(|root| Arc::ptr_eq(&root, snapshot))
            && let Some(endpoint) = cache.content.get(&hint.key)
        {
            return Ok(endpoint);
        }
    }
    let options = &snapshot.options;
    let font = match &options.font {
        Font::Asset(bytes) => FontKey::Asset(Arc::as_ptr(bytes) as usize),
        Font::Family(names) => FontKey::Family(names),
    };
    let key = sanim_core::digest(
        &serde_json::to_vec(&(
            id,
            snapshot.document.visual_recipe(),
            &snapshot.document.path,
            options.visual_recipe(font),
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
        if let Some(endpoint) = cache.content.get(&key) {
            cache.identities.insert(identity, hint, hint_bytes);
            return Ok(endpoint);
        }
    }
    let document = Arc::new(crate::code::parse(snapshot.document.as_ref().clone())?);
    let prepared = prepare(
        id,
        &document,
        options,
        &snapshot.folded,
        &snapshot.selection,
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
    let endpoint = Arc::new(Endpoint {
        document,
        options: options.clone(),
        tokens,
        size: prepared.size,
    });
    let mut cache = cache.lock().expect("code layout cache poisoned");
    cache
        .content
        .insert(key, endpoint.clone(), MAX_LAYOUT_BYTES - bytes);
    cache.identities.insert(identity, hint, hint_bytes);
    Ok(endpoint)
}

pub fn resolve_pending(element: &mut Element, _time: Fraction) -> Result<(), String> {
    let Geometry::Code {
        document,
        options,
        pending: true,
        presentation,
        ..
    } = element.geometry.as_ref()
    else {
        return Ok(());
    };
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
            let resolved = endpoint(&element.id, &from)?;
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
            let resolved = endpoint(&element.id, &to)?;
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
            let from = endpoint(&element.id, &from)?;
            let to = endpoint(&element.id, &to)?;
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
    if element.layout.scroll.is_none() {
        for (target, size) in [
            (&mut element.layout.width, size[0]),
            (&mut element.layout.height, size[1]),
        ] {
            *target = Some(Number {
                value: math::from_f64(size)?,
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
    Ok(())
}
