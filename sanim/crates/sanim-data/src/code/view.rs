mod resolver;
mod transition;
pub use resolver::resolve_pending;
use sanim_core::code::view::Token;
use sanim_core::{
    code::{
        Document,
        view::{Options, Range, Style},
    },
    *,
};
use std::collections::BTreeSet;
use std::{collections::BTreeMap, sync::Arc};
pub use transition::transition_tokens;
const MAX_CODE_TOKENS: usize = 65_536;
const MAX_CODE_SPANS: usize = 1024;
struct Run {
    geometry: Arc<Geometry>,
    advance: f64,
}
fn run(font: &Font, size: Fraction, text: &str, style: &Style) -> Result<Arc<Run>, String> {
    let shaped = crate::font::cached_shape(
        font,
        text,
        size,
        style.bold.unwrap_or(false),
        style.italic.unwrap_or(false),
    )?;
    Ok(Arc::new(Run {
        geometry: Arc::new(Geometry::Text {
            text: text.into(),
            font: font.clone(),
            size,
            outline: shaped.outline.clone(),
            bounds: crate::font::bounds(&shaped),
            pending: false,
        }),
        advance: shaped.advance,
    }))
}

pub struct Prepared {
    pub tokens: imbl::Vector<Token>,
    pub size: [f64; 2],
}
fn element(
    id: String,
    geometry: Arc<Geometry>,
    position: [f64; 2],
    fill: Color,
    opacity: Fraction,
) -> Element {
    Element {
        id,
        geometry,
        position: Vec2 {
            values: position,
            unit: Unit::Length,
        },
        rotation: 0.into(),
        scale: 1.into(),
        opacity,
        fill: fill.into(),
        stroke: None,
        reveal: Reveal::default(),
        horizontal: HorizontalTransform::default(),
        layout: Layout {
            x_align: Align::Start,
            y_align: Align::Start,
            ..Default::default()
        },
    }
}
fn local_position(geometry: &Geometry, position: [f64; 2]) -> Result<[f64; 2], String> {
    let dummy = element(
        String::new(),
        Arc::new(geometry.clone()),
        [0.; 2],
        Color::parse("light1")?,
        1.into(),
    );
    let bounds = verify::bounds(&dummy)?;
    Ok([
        position[0] + bounds.min.values[0],
        position[1] + bounds.min.values[1],
    ])
}

pub fn prepare(
    id: &str,
    document: &Document,
    options: &Options,
    folded: &imbl::OrdSet<Arc<str>>,
    selection: &[Range],
) -> Result<Prepared, String> {
    if options.spans.len() > MAX_CODE_SPANS || selection.len() > MAX_CODE_SPANS {
        return Err(format!(
            "CodeView supports at most {MAX_CODE_SPANS} style/selection ranges"
        ));
    }
    let text = document.source()?;
    let revision = digest_bytes(text.as_bytes());
    let line_offsets: Vec<_> = std::iter::once(0)
        .chain(text.match_indices('\n').map(|(at, _)| at + 1))
        .collect();
    let byte = |line: usize, column: usize| -> Result<usize, String> {
        if line == line_offsets.len() + 1 && column == 1 {
            return Ok(text.len());
        }
        let start = *line_offsets
            .get(line.checked_sub(1).ok_or("CodeRange lines are 1-based")?)
            .ok_or_else(|| format!("CodeRange line {line} exceeds {} lines", line_offsets.len()))?;
        let end = line_offsets.get(line).copied().unwrap_or(text.len());
        let content = text[start..end].trim_end_matches(['\r', '\n']);
        let count = content.chars().count();
        if column == count + 1 {
            return Ok(start + content.len());
        }
        content
            .char_indices()
            .nth(
                column
                    .checked_sub(1)
                    .ok_or("CodeRange columns are 1-based")?,
            )
            .map(|(at, _)| start + at)
            .ok_or_else(|| {
                format!(
                    "CodeRange column {column} exceeds line {line}'s {count} Unicode characters"
                )
            })
    };
    let mut events = Vec::<(usize, bool, usize, usize)>::new();
    for (index, highlight) in document.highlights.iter().enumerate() {
        if highlight.start < highlight.end && highlight.end <= text.len() {
            events.push((highlight.start, true, 0, index));
            events.push((highlight.end, false, 0, index));
        }
    }
    for (index, span) in options.spans.iter().enumerate() {
        span.range.validate()?;
        let start = byte(span.range.start_line, span.range.start_column)?;
        let end = byte(span.range.end_line, span.range.end_column)?;
        events.push((start, true, 1, index));
        events.push((end, false, 1, index));
    }
    for (index, range) in selection.iter().enumerate() {
        range.validate()?;
        let start = byte(range.start_line, range.start_column)?;
        let end = byte(range.end_line, range.end_column)?;
        events.push((start, true, 2, index));
        events.push((end, false, 2, index));
    }
    events.sort_unstable();
    let mut active_highlights = BTreeSet::new();
    let mut active_spans = BTreeSet::new();
    let mut active_selection = BTreeSet::new();
    let mut event_at = 0;
    let space = run(&options.font, options.font_size, " ", &Style::default())?.advance;
    let line_height = math::to_f64(options.line_height)?;
    let font_size = math::to_f64(options.font_size)?;
    let number_width = if options.line_numbers {
        let digits = "0".repeat(line_offsets.len().to_string().len());
        run(&options.font, options.font_size, &digits, &Style::default())?.advance
    } else {
        0.
    };
    let left = if options.line_numbers {
        number_width + 2. * space
    } else {
        0.
    };
    let mut tokens = imbl::Vector::new();
    let mut token_count = 0;
    let mut widest = left + space;
    let mut visible_row = 0usize;
    let mut hidden_until = 0usize;
    let mut collapses = BTreeMap::new();
    for scope in document
        .scopes
        .iter()
        .filter(|scope| folded.contains(&scope.id))
    {
        collapses
            .entry(scope.start_line)
            .and_modify(|end: &mut usize| *end = (*end).max(scope.end_line))
            .or_insert(scope.end_line);
    }
    let mut add = |word: &str,
                   line: usize,
                   column: usize,
                   position: [f64; 2],
                   style: &Style,
                   tag: &str|
     -> Result<f64, String> {
        let shaped = run(&options.font, options.font_size, word, style)?;
        let key: Arc<str> = format!(
            "{tag}:{}",
            digest(&serde_json::to_vec(&(word, style)).map_err(|error| error.to_string())?)
        )
        .into();
        let glyph = {
            let identity = format!("{id}/@live/{line}:{column}:{key}");
            let simple = style.background.is_none()
                && !style.underline.unwrap_or(false)
                && !style.strike.unwrap_or(false);
            if simple {
                Arc::new(element(
                    identity,
                    shaped.geometry.clone(),
                    local_position(&shaped.geometry, position)?,
                    style.fill.clone().unwrap_or(Color::parse("#c0bfbc")?),
                    style.opacity.unwrap_or(1.into()),
                ))
            } else {
                let mut children = imbl::Vector::new();
                if let Some(background) = &style.background {
                    let width = math::from_f64(shaped.advance.max(space / 4.))?;
                    let height = options.line_height;
                    let geometry = Arc::new(Geometry::Rectangle {
                        width,
                        height,
                        radius: Fraction::from(0).into(),
                        children: imbl::Vector::new(),
                    });
                    children.push_back(element(
                        format!("{identity}/background"),
                        geometry.clone(),
                        local_position(&geometry, [shaped.advance / 2., -font_size / 2.])?,
                        background.clone(),
                        1.into(),
                    ));
                }
                children.push_back(element(
                    format!("{identity}/glyph"),
                    shaped.geometry.clone(),
                    local_position(&shaped.geometry, [0., 0.])?,
                    style.fill.clone().unwrap_or(Color::parse("#c0bfbc")?),
                    1.into(),
                ));
                for (enabled, suffix, y) in [
                    (
                        style.underline.unwrap_or(false),
                        "underline",
                        font_size * 0.12,
                    ),
                    (style.strike.unwrap_or(false), "strike", -font_size * 0.3),
                ] {
                    if enabled {
                        let geometry = Arc::new(Geometry::Line {
                            end: Vec2 {
                                values: [shaped.advance, 0.],
                                unit: Unit::Length,
                            },
                            width: math::from_f64((font_size / 14.).max(0.5))?,
                        });
                        children.push_back(element(
                            format!("{identity}/{suffix}"),
                            geometry.clone(),
                            local_position(&geometry, [0., y])?,
                            style.fill.clone().unwrap_or(Color::parse("#c0bfbc")?),
                            1.into(),
                        ));
                    }
                }
                let geometry = Arc::new(Geometry::Box {
                    children,
                    radius: Default::default(),
                });
                let mut token = element(
                    identity,
                    geometry,
                    position,
                    Color::parse("transparent")?,
                    style.opacity.unwrap_or(1.into()),
                );
                token.layout.width = Some(sanim_core::Number {
                    value: math::from_f64(shaped.advance)?,
                    unit: Unit::Length,
                });
                token.layout.height = Some(sanim_core::Number {
                    value: options.line_height,
                    unit: Unit::Length,
                });
                Arc::new(token)
            }
        };
        let position = if matches!(glyph.geometry.as_ref(), Geometry::Box { .. }) {
            position
        } else {
            local_position(&glyph.geometry, position)?
        };
        let token = Token {
            revision,
            key,
            style: Arc::new(style.clone()),
            element: glyph,
            position: Vec2 {
                values: position,
                unit: Unit::Length,
            },
            line,
            column,
        };
        tokens.push_back(token);
        token_count += 1;
        if token_count > MAX_CODE_TOKENS {
            return Err(format!(
                "CodeView exceeds {MAX_CODE_TOKENS} displayed tokens; fold scopes or show a shorter file"
            ));
        }
        Ok(shaped.advance)
    };
    for (index, &start) in line_offsets.iter().enumerate() {
        let line = index + 1;
        let end = line_offsets.get(index + 1).copied().unwrap_or(text.len());
        if start == text.len() && text.ends_with('\n') {
            break;
        }
        if line <= hidden_until {
            continue;
        }
        let baseline = visible_row as f64 * line_height + font_size;
        visible_row += 1;
        if options.line_numbers {
            let style = Style {
                fill: Some(options.theme.line_number.clone()),
                ..Default::default()
            };
            let label = line.to_string();
            let advance = run(&options.font, options.font_size, &label, &style)?.advance;
            add(
                &label,
                line,
                1,
                [number_width - advance, baseline],
                &style,
                "line-number",
            )?;
        }
        let content = text[start..end].trim_end_matches(['\r', '\n']);
        let mut at = start;
        let mut x = left;
        let mut column = 1;
        let mut visual_column = 1;
        let content_end = start + content.len();
        while at < content_end {
            while let Some(&(offset, enter, kind, index)) =
                events.get(event_at).filter(|event| event.0 <= at)
            {
                match kind {
                    0 => {
                        let highlight = &document.highlights[index];
                        let item = (
                            highlight.end - highlight.start,
                            std::cmp::Reverse(index),
                            index,
                        );
                        if enter {
                            active_highlights.insert(item);
                        } else {
                            active_highlights.remove(&item);
                        }
                    }
                    1 => {
                        if enter {
                            active_spans.insert(index);
                        } else {
                            active_spans.remove(&index);
                        }
                    }
                    _ => {
                        if enter {
                            active_selection.insert(index);
                        } else {
                            active_selection.remove(&index);
                        }
                    }
                }
                event_at += 1;
                let _ = offset;
            }
            let next_event = events
                .get(event_at)
                .map_or(content_end, |event| event.0.min(content_end));
            let stop = next_event.max(
                at + text[at..]
                    .chars()
                    .next()
                    .expect("source character")
                    .len_utf8(),
            );
            let kind = active_highlights
                .first()
                .map(|(_, _, index)| document.highlights[*index].kind.as_ref())
                .unwrap_or("text");
            let mut style = options.theme.style(kind);
            for index in &active_spans {
                style.overlay(&options.spans[*index].style);
            }
            if !selection.is_empty() {
                if active_selection.is_empty() {
                    style.opacity = Some(math::visual_product(
                        style.opacity.unwrap_or(1.into()),
                        options.theme.inactive_opacity,
                    )?);
                } else {
                    let mut background = options.theme.selection.clone();
                    background.0[3] = Fraction::new(1u64, 3u64);
                    style.background = Some(background);
                }
            }
            let fragment = &text[at..stop];
            let first = fragment.chars().next().expect("fragment");
            let group = |ch: char| {
                if ch == '\t' {
                    0
                } else if ch.is_whitespace() {
                    1
                } else if !ch.is_ascii_punctuation() || ch == '_' {
                    2
                } else {
                    3
                }
            };
            let category = group(first);
            let count = if category == 0 {
                1
            } else {
                fragment
                    .chars()
                    .take_while(|ch| group(*ch) == category)
                    .map(char::len_utf8)
                    .sum()
            };
            let word = &fragment[..count];
            if category == 0 {
                let remaining = options.tab_size - (visual_column - 1) % options.tab_size;
                x += remaining as f64 * space;
                column += 1;
                visual_column += remaining;
            } else {
                if category == 1 && style.background.is_none() {
                    x += run(&options.font, options.font_size, word, &style)?.advance;
                } else {
                    x += add(word, line, column, [x, baseline], &style, "token")?;
                }
                column += word.chars().count();
                visual_column += word.chars().count();
            }
            at += count;
        }
        if let Some(&last) = collapses.get(&line) {
            hidden_until = last;
            let style = options.theme.style("comment");
            x += add("  …", line, column, [x, baseline], &style, "fold")?;
        }
        widest = widest.max(x);
    }
    Ok(Prepared {
        tokens,
        size: [
            widest.max(space),
            ((visible_row.max(1)) as f64 * line_height).max(font_size),
        ],
    })
}
