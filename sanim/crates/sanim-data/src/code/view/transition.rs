//! One transition engine for every native code view, including signal-controlled styles.
mod correspondence;
mod fragments;
use imbl::Vector;
use pango::glib::translate::ToGlibPtr;
use sanim_core::{
    Element, Fraction, Geometry, List, Number, Type, Unit, Value,
    cache::Cache,
    code::view::{PatchAnimation, Style, Token},
    math,
};
use std::sync::{Arc, Mutex, OnceLock};

const MAX_TRANSITION_BYTES: usize = 64 * 1024 * 1024;
const MAX_TRANSITION_ENTRIES: usize = 128;
const MAX_TRANSITION_ITEMS: usize = 1_000_000;
const MAX_TRANSITION_DEPTH: usize = 256;
// PangoLogAttr::is_cursor_position is the fifth guint bitfield in pango-break.h.
const CURSOR_POSITION: u32 = 1 << 4;

struct Incoming {
    previous: Option<usize>,
    boundaries: Arc<[usize]>,
    before: usize,
}
struct Outgoing {
    index: usize,
    boundaries: Arc<[usize]>,
    before: usize,
}
struct Plan {
    source: Vector<Token>,
    target: Vector<Token>,
    from: Vector<Token>,
    to: Vector<Token>,
    incoming: Vec<Incoming>,
    outgoing: Vec<Outgoing>,
    characters: usize,
    deletion_characters: usize,
}

fn text(element: &Element) -> Option<&str> {
    if let Geometry::Text { text, .. } = element.geometry.as_ref() {
        return Some(text);
    }
    element.geometry.children()?.iter().find_map(text)
}

fn boundaries(text: &str) -> Result<Arc<[usize]>, String> {
    let length =
        i32::try_from(text.len()).map_err(|_| "typewriter text exceeds Pango's byte range")?;
    let offsets: Vec<_> = text
        .char_indices()
        .map(|(offset, _)| offset)
        .chain(std::iter::once(text.len()))
        .collect();
    let count = i32::try_from(offsets.len())
        .map_err(|_| "typewriter text exceeds Pango's character range")?;
    // The C struct is one guint of bitfields. pango-sys intentionally exposes a
    // truncated struct, whose Rust stride must not be used for the native array.
    let mut attrs = vec![0u32; offsets.len()];
    let language = pango::Language::default();
    // SAFETY: text is valid UTF-8, both lengths fit the C API, and attrs has the
    // documented n_chars + 1 guint records. Pango initializes the complete array.
    unsafe {
        pango::ffi::pango_get_log_attrs(
            text.as_ptr().cast(),
            length,
            0,
            language.to_glib_none().0,
            attrs.as_mut_ptr().cast(),
            count,
        );
    }
    let ends: Vec<_> = offsets
        .into_iter()
        .zip(attrs)
        .skip(1)
        .filter_map(|(offset, attrs)| (attrs & CURSOR_POSITION != 0).then_some(offset))
        .collect();
    if ends.last().copied() != Some(text.len()) && !text.is_empty() {
        return Err("Pango did not provide a final typewriter cursor boundary".into());
    }
    Ok(ends.into())
}

fn plan(
    source: &Vector<Token>,
    target: &Vector<Token>,
    animation: PatchAnimation,
) -> Result<Arc<Plan>, String> {
    type Key = (usize, usize, u8);
    static CACHE: OnceLock<Mutex<Cache<Key, Plan>>> = OnceLock::new();
    let key = (
        std::ptr::from_ref(source) as usize,
        std::ptr::from_ref(target) as usize,
        animation as u8,
    );
    let cache =
        CACHE.get_or_init(|| Mutex::new(Cache::new(MAX_TRANSITION_BYTES, MAX_TRANSITION_ENTRIES)));
    if let Some(plan) = cache
        .lock()
        .expect("code transition cache poisoned")
        .get(&key)
        && plan.source.ptr_eq(source)
        && plan.target.ptr_eq(target)
    {
        return Ok(plan);
    }
    let typing = animation == PatchAnimation::Typewriter;
    let (from, to, matches) = if animation == PatchAnimation::Morph {
        fragments::prepare(source, target)?
    } else {
        (
            source.clone(),
            target.clone(),
            correspondence::matches(source, target),
        )
    };
    let mut retained = vec![false; from.len()];
    let mut incoming = Vec::with_capacity(to.len());
    let mut characters = 0usize;
    let mut last_typed: Option<(usize, usize)> = None;
    for (token, previous) in to.iter().zip(matches) {
        if let Some(index) = previous {
            retained[index] = true;
        }
        let mut ends: Arc<[usize]> = Arc::from([]);
        if typing && !token.is_gutter() {
            if previous.is_some() {
                last_typed = None;
            } else if let Some(word) = text(&token.element) {
                if let Some((line, column)) = last_typed {
                    let gap = if line == token.line {
                        token.column.saturating_sub(column)
                    } else {
                        token.column
                    };
                    characters = characters
                        .checked_add(gap)
                        .ok_or("typewriter character count overflow")?;
                }
                ends = boundaries(word)?;
                last_typed = Some((token.line, token.column + word.chars().count()));
            }
        }
        let before = characters;
        characters = characters
            .checked_add(ends.len())
            .ok_or("typewriter character count overflow")?;
        incoming.push(Incoming {
            previous,
            boundaries: ends,
            before,
        });
    }
    let mut outgoing = Vec::new();
    let mut deletion_characters = 0usize;
    for index in (0..from.len()).rev().filter(|&index| !retained[index]) {
        let token = &from[index];
        let ends = if typing && !token.is_gutter() {
            text(&token.element)
                .map(boundaries)
                .transpose()?
                .unwrap_or_else(|| Arc::from([]))
        } else {
            Arc::from([])
        };
        let before = deletion_characters;
        deletion_characters = deletion_characters
            .checked_add(ends.len())
            .ok_or("typewriter deletion count overflow")?;
        outgoing.push(Outgoing {
            index,
            boundaries: ends,
            before,
        });
    }
    characters = characters
        .checked_add(deletion_characters)
        .ok_or("typewriter edit count overflow")?;
    let mut seen = std::collections::HashSet::new();
    let retained_tokens: Vec<_> = source
        .iter()
        .chain(target)
        .chain(&from)
        .chain(&to)
        .filter(|token| seen.insert(Arc::as_ptr(&token.element)))
        .collect();
    let retained = Value::List(List::typed(
        Type::Element,
        retained_tokens
            .iter()
            .map(|token| Value::Element(token.element.clone()))
            .collect(),
    ));
    let mut bytes = MAX_TRANSITION_BYTES;
    let mut items = MAX_TRANSITION_ITEMS;
    sanim_core::value_budget::charge(&retained, &mut bytes, &mut items, MAX_TRANSITION_DEPTH)?;
    let metadata = std::mem::size_of::<Plan>()
        + incoming.len() * std::mem::size_of::<Incoming>()
        + outgoing.len() * std::mem::size_of::<Outgoing>()
        + retained_tokens
            .iter()
            .map(|token| {
                token.key.len() + std::mem::size_of::<Style>() + std::mem::size_of::<Token>()
            })
            .sum::<usize>()
        + incoming
            .iter()
            .map(|item| std::mem::size_of_val(item.boundaries.as_ref()))
            .sum::<usize>()
        + outgoing
            .iter()
            .map(|item| std::mem::size_of_val(item.boundaries.as_ref()))
            .sum::<usize>();
    let size = (MAX_TRANSITION_BYTES - bytes)
        .checked_add(metadata)
        .ok_or("code transition byte size overflow")?;
    if size > MAX_TRANSITION_BYTES {
        return Err("code transition exceeds its retained byte budget".into());
    }
    let value = Arc::new(Plan {
        source: source.clone(),
        target: target.clone(),
        from,
        to,
        incoming,
        outgoing,
        characters,
        deletion_characters,
    });
    cache
        .lock()
        .expect("code transition cache poisoned")
        .insert(key, value.clone(), size);
    Ok(value)
}

fn at(token: &Token) -> Element {
    let mut element = token.element.as_ref().clone();
    element.position = token.position;
    element
}
fn identify(element: &mut Element, suffix: &str) {
    element.id.push_str(suffix);
    if let Some(children) = element.geometry.children()
        && !children.is_empty()
    {
        for child in Arc::make_mut(&mut element.geometry)
            .children_mut()
            .expect("container children")
        {
            identify(child, suffix);
        }
    }
}

fn prefix(element: &mut Element, value: &str, style: &Style) -> Result<f64, String> {
    if let Geometry::Text {
        font, size, bounds, ..
    } = element.geometry.as_ref()
    {
        let shaped = crate::font::cached_shape(
            font,
            value,
            *size,
            style.bold.unwrap_or(false),
            style.italic.unwrap_or(false),
        )?;
        let difference = [
            crate::font::bounds(&shaped).min.values[0] - bounds.min.values[0],
            crate::font::bounds(&shaped).min.values[1] - bounds.min.values[1],
        ];
        for (axis, delta) in difference.into_iter().enumerate() {
            element.position.values[axis] += delta;
        }
        let Geometry::Text {
            text,
            bounds,
            outline,
            pending,
            ..
        } = Arc::make_mut(&mut element.geometry)
        else {
            unreachable!("typed glyph")
        };
        *text = value.into();
        *bounds = crate::font::bounds(&shaped);
        *outline = shaped.outline.clone();
        *pending = false;
        return Ok(shaped.advance);
    }
    let children = Arc::make_mut(&mut element.geometry)
        .children_mut()
        .ok_or("typewriter token contains no native text")?;
    let glyph = children
        .iter_mut()
        .find(|child| matches!(child.geometry.as_ref(), Geometry::Text { .. }))
        .ok_or("typewriter token contains no native glyph")?;
    let advance = prefix(glyph, value, style)?;
    for child in children {
        match Arc::make_mut(&mut child.geometry) {
            Geometry::Rectangle { width, .. } => *width = math::from_f64(advance)?,
            Geometry::Line { end, .. } => end.values[0] = advance,
            _ => {}
        }
    }
    Ok(advance)
}

// Native code tokens use solid paints. Update existing glyph/decorations in
// place so a theme or selection change does not fade unchanged source text.
fn recolor(element: &mut Element, previous: &Element, progress: Fraction) -> Result<(), String> {
    let previous = if matches!(element.geometry.as_ref(), Geometry::Text { .. }) {
        previous
            .geometry
            .children()
            .and_then(|children| {
                children
                    .iter()
                    .find(|child| matches!(child.geometry.as_ref(), Geometry::Text { .. }))
            })
            .unwrap_or(previous)
    } else {
        previous
    };
    if std::mem::discriminant(element.geometry.as_ref())
        == std::mem::discriminant(previous.geometry.as_ref())
        && let (sanim_core::Paint::Solid(from), sanim_core::Paint::Solid(to)) =
            (&previous.fill, &element.fill)
        && from != to
    {
        element.fill = math::paint::color_lerp(from, to, progress)?.into();
    }
    if element
        .geometry
        .children()
        .is_some_and(|children| !children.is_empty())
    {
        let role = |element: &Element| {
            element
                .id
                .split("/@fragment/")
                .next()
                .unwrap_or(&element.id)
                .rsplit('/')
                .next()
                .unwrap_or("")
                .to_owned()
        };
        for child in Arc::make_mut(&mut element.geometry)
            .children_mut()
            .expect("code children")
        {
            let previous = previous
                .geometry
                .children()
                .and_then(|children| children.iter().find(|old| role(old) == role(child)))
                .unwrap_or(previous);
            recolor(child, previous, progress)?;
        }
    }
    Ok(())
}

/// Blend immutable native endpoints. Typewriter reveals complete Unicode cursor
/// clusters; the completed token uses the exact original target outline.
pub fn transition_tokens(
    from: &Vector<Token>,
    to: &Vector<Token>,
    progress: Fraction,
    appearance: Fraction,
    raw_progress: Fraction,
    animation: PatchAnimation,
) -> Result<Vector<Element>, String> {
    if raw_progress <= 0.into() {
        return Ok(from.iter().map(at).collect());
    }
    if raw_progress >= 1.into() {
        return Ok(to.iter().map(at).collect());
    }
    let plan = plan(from, to, animation)?;
    let from = &plan.from;
    let to = &plan.to;
    let typed = if animation == PatchAnimation::Typewriter {
        math::functions::evaluate(
            "floor",
            &[Number {
                value: raw_progress * Fraction::from(plan.characters as u64),
                unit: Unit::Number,
            }],
        )?
        .usize()?
    } else {
        0
    };
    let [removed_opacity, inserted_opacity] = if animation == PatchAnimation::Morph {
        math::paint::edit_opacities(appearance)?
    } else {
        [math::arithmetic("-", 1.into(), appearance)?, appearance]
    };
    let mut children = Vector::new();
    for (token, item) in to.iter().zip(&plan.incoming) {
        let mut element = at(token);
        if let Some(previous) = item.previous {
            let previous = &from[previous];
            if animation == PatchAnimation::Fade
                && (previous.position != token.position
                    || previous.element.opacity != token.element.opacity
                    || previous.style != token.style)
            {
                let mut old = at(previous);
                identify(&mut old, "/@leaving");
                old.opacity = math::visual_product(
                    old.opacity,
                    math::arithmetic("-", 1.into(), appearance)?,
                )?;
                children.push_back(old);
                element.opacity = math::visual_product(element.opacity, appearance)?;
            } else if animation != PatchAnimation::Fade {
                if previous.position != token.position {
                    for axis in 0..2 {
                        if previous.position.values[axis] != token.position.values[axis] {
                            element.position.values[axis] = math::motion_lerp(
                                previous.position.values[axis],
                                token.position.values[axis],
                                progress,
                                true,
                            )?;
                        }
                    }
                }
                if previous.element.opacity != element.opacity {
                    element.opacity =
                        math::visual_lerp(previous.element.opacity, element.opacity, appearance)?;
                }
                if previous.element.rotation != element.rotation {
                    element.rotation =
                        math::visual_lerp(previous.element.rotation, element.rotation, progress)?;
                }
                if previous.style != token.style {
                    recolor(&mut element, &previous.element, appearance)?;
                }
            }
        } else if animation == PatchAnimation::Typewriter && !item.boundaries.is_empty() {
            let count = typed
                .saturating_sub(plan.deletion_characters)
                .saturating_sub(item.before)
                .min(item.boundaries.len());
            if count == 0 {
                continue;
            }
            if count < item.boundaries.len() {
                let full = text(&element).ok_or("typewriter token has no native text")?;
                let value = full[..item.boundaries[count - 1]].to_owned();
                prefix(&mut element, &value, &token.style)?;
            }
        } else if animation != PatchAnimation::Typewriter {
            element.opacity = math::visual_product(
                element.opacity,
                if token.is_gutter() {
                    appearance
                } else {
                    inserted_opacity
                },
            )?;
        }
        children.push_back(element);
    }
    for item in &plan.outgoing {
        let token = &from[item.index];
        let mut old = at(token);
        identify(&mut old, "/@leaving");
        if animation == PatchAnimation::Typewriter && !item.boundaries.is_empty() {
            let count = item
                .boundaries
                .len()
                .saturating_sub(typed.saturating_sub(item.before));
            if count == 0 {
                continue;
            }
            if count < item.boundaries.len() {
                let full = text(&old).ok_or("typewriter token has no native text")?;
                let value = full[..item.boundaries[count - 1]].to_owned();
                prefix(&mut old, &value, &token.style)?;
            }
        } else {
            old.opacity = math::visual_product(
                old.opacity,
                if token.is_gutter() {
                    math::arithmetic("-", 1.into(), appearance)?
                } else {
                    removed_opacity
                },
            )?;
        }
        children.push_back(old);
    }
    Ok(children)
}

#[cfg(test)]
mod tests {
    use super::boundaries;
    #[test]
    fn cursor_boundaries_keep_zwj_flags_and_combining_sequences_whole() {
        let text = "👩‍💻🇺🇳e\u{301}";
        assert_eq!(
            boundaries(text).unwrap().as_ref(),
            &["👩‍💻".len(), "👩‍💻🇺🇳".len(), text.len()]
        );
        assert!(boundaries("").unwrap().is_empty());
    }
}
