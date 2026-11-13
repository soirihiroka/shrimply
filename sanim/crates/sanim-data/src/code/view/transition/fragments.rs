//! Sub-token edit identity, prepared once with the transition plan. Native runs
//! stay intact everywhere except the changed tokens, preserving ligatures and
//! avoiding the per-character draw overhead of globally splitting the document.
use super::{at, boundaries, correspondence, identify, prefix, text};
use imbl::Vector;
use sanim_core::{Geometry, code::view::Token};
use std::sync::Arc;

type Prepared = (Vector<Token>, Vector<Token>, Vec<Option<usize>>);

pub(super) fn prepare(from: &Vector<Token>, to: &Vector<Token>) -> Result<Prepared, String> {
    let matches = correspondence::matches(from, to);
    // Same-source fold/selection changes already have exact source coordinates.
    // Never match a hidden token to an unrelated visible repeated word.
    if from
        .front()
        .zip(to.front())
        .is_some_and(|(a, b)| a.revision == b.revision)
    {
        return Ok((from.clone(), to.clone(), matches));
    }
    let mut retained = vec![false; from.len()];
    for previous in matches.iter().flatten() {
        retained[*previous] = true;
    }
    let mut replacements = vec![None; to.len()];
    let mut cursor = 0;
    let mut previous = None;
    while cursor < to.len() {
        if let Some(index) = matches[cursor] {
            previous = Some(index);
            cursor += 1;
            continue;
        }
        let start = cursor;
        while cursor < to.len() && matches[cursor].is_none() {
            cursor += 1;
        }
        let old_start = previous.map_or(0, |index| index + 1);
        let old_end = matches.get(cursor).copied().flatten().unwrap_or(from.len());
        // Only pair an equally sized, ordered replacement span bounded by
        // unchanged context. Moved/ambiguous blocks retain the ordinary diff.
        if old_end < old_start || old_end - old_start != cursor - start {
            continue;
        }
        for (new, old) in (start..cursor).zip(old_start..old_end) {
            if !retained[old] {
                replacements[new] = Some(old);
            }
        }
    }
    let mut old_parts = vec![None; from.len()];
    let mut new_parts = vec![None; to.len()];
    let mut split = false;
    for (new, old) in replacements.into_iter().enumerate() {
        let Some(old) = old else {
            continue;
        };
        if from[old].key.split(':').next() != to[new].key.split(':').next() {
            continue;
        }
        let (Some(a), Some(b)) = (text(&from[old].element), text(&to[new].element)) else {
            continue;
        };
        // Most replacements share no visible edge. Avoid native grapheme work
        // for those whole-token substitutions.
        if a.chars().next() != b.chars().next() && a.chars().next_back() != b.chars().next_back() {
            continue;
        }
        let a_ends = boundaries(a)?;
        let b_ends = boundaries(b)?;
        let a_starts: Vec<_> = std::iter::once(0).chain(a_ends.iter().copied()).collect();
        let b_starts: Vec<_> = std::iter::once(0).chain(b_ends.iter().copied()).collect();
        let mut head = 0;
        while head < a_ends.len().min(b_ends.len())
            && a[a_starts[head]..a_ends[head]] == b[b_starts[head]..b_ends[head]]
        {
            head += 1;
        }
        let mut tail = 0;
        while tail < a_ends.len().min(b_ends.len()) - head {
            let ai = a_ends.len() - 1 - tail;
            let bi = b_ends.len() - 1 - tail;
            if a[a_starts[ai]..a_ends[ai]] != b[b_starts[bi]..b_ends[bi]] {
                break;
            }
            tail += 1;
        }
        if head == 0 && tail == 0 {
            continue;
        }
        let old_cuts = [0, a_starts[head], a_starts[a_ends.len() - tail], a.len()];
        let new_cuts = [0, b_starts[head], b_starts[b_ends.len() - tail], b.len()];
        old_parts[old] = Some(parts(&from[old], &old_cuts)?);
        new_parts[new] = Some(parts(&to[new], &new_cuts)?);
        split = true;
    }
    if !split {
        return Ok((from.clone(), to.clone(), matches));
    }
    let expand = |tokens: &Vector<Token>, fragments: Vec<Option<Vec<Token>>>| {
        // Keep all untouched vector branches shared with the endpoint; a one-
        // word edit must not rebuild the entire token vector's storage.
        let mut expanded = tokens.clone();
        for (index, parts) in fragments.into_iter().enumerate().rev() {
            if let Some(parts) = parts {
                expanded.remove(index);
                for part in parts.into_iter().rev() {
                    expanded.insert(index, part);
                }
            }
        }
        expanded
    };
    let from = expand(from, old_parts);
    let to = expand(to, new_parts);
    let matches = correspondence::matches(&from, &to);
    Ok((from, to, matches))
}

fn parts(token: &Token, cuts: &[usize; 4]) -> Result<Vec<Token>, String> {
    let source = text(&token.element).ok_or("code transition token has no native text")?;
    let mut output = Vec::new();
    for range in cuts.windows(2) {
        let (start, end) = (range[0], range[1]);
        if start == end {
            continue;
        }
        let mut element = at(token);
        let offset = if start == 0 {
            0.
        } else {
            let mut preceding = element.clone();
            prefix(&mut preceding, &source[..start], &token.style)?
        };
        let advance = prefix(&mut element, &source[start..end], &token.style)?;
        element.position.values[0] += offset;
        identify(&mut element, &format!("/@fragment/{start}:{end}"));
        if matches!(element.geometry.as_ref(), Geometry::Box { .. }) {
            element.layout.width = Some(sanim_core::Number {
                value: sanim_core::math::from_f64(advance)?,
                unit: sanim_core::Unit::Length,
            });
        }
        let mut fragment = token.clone();
        fragment.key = format!(
            "{}:{}",
            token.key.split(':').next().unwrap_or("token"),
            &source[start..end]
        )
        .into();
        fragment.column += source[..start].chars().count();
        fragment.position = element.position;
        fragment.element = Arc::new(element);
        output.push(fragment);
    }
    Ok(output)
}
