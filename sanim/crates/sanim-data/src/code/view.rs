mod gutter;
mod resolver;
mod transition;
pub(crate) use resolver::normalized_text;
pub use resolver::{range_bounds, resolve_pending};
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
    rows: Vec<SourceRow>,
}
struct SourceRow {
    start: (usize, usize),
    end: (usize, usize),
    top: f64,
    bottom: f64,
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
            text: sanim_core::text::Layout {
                white_space: Some(sanim_core::text::WhiteSpace::Pre),
                resolved: Some(sanim_core::text::Recipe {
                    white_space: sanim_core::text::WhiteSpace::Pre,
                    overflow_wrap: sanim_core::text::OverflowWrap::Normal,
                    width: None,
                }),
                ..Default::default()
            },
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
        Color([Fraction::from(0); 4]),
        1.into(),
    );
    let bounds = verify::bounds(&dummy)?;
    Ok([
        position[0] + bounds.min.values[0],
        position[1] + bounds.min.values[1],
    ])
}

struct Command {
    word: String,
    line: usize,
    column: usize,
    style: Style,
    tag: &'static str,
}

pub fn prepare(
    id: &str,
    document: &Document,
    options: &Options,
    folded: &imbl::OrdSet<Arc<str>>,
    selection: &[Range],
) -> Result<Prepared, String> {
    prepare_layout(
        id,
        document,
        options,
        folded,
        selection,
        sanim_core::text::Recipe {
            white_space: sanim_core::text::WhiteSpace::Pre,
            overflow_wrap: sanim_core::text::OverflowWrap::Normal,
            width: None,
        },
    )
}
pub(super) fn prepare_layout(
    id: &str,
    document: &Document,
    options: &Options,
    folded: &imbl::OrdSet<Arc<str>>,
    selection: &[Range],
    recipe: sanim_core::text::Recipe,
) -> Result<Prepared, String> {
    if options.spans.len() > MAX_CODE_SPANS || selection.len() > MAX_CODE_SPANS {
        return Err(format!(
            "CodeView supports at most {MAX_CODE_SPANS} style/selection ranges"
        ));
    }
    let text = document.source()?;
    let flow_key = if recipe.white_space == sanim_core::text::WhiteSpace::Pre {
        None
    } else {
        Some(source_flow_key(
            id,
            document,
            options,
            folded,
            selection,
            recipe.white_space,
        )?)
    };
    let cached_flow = flow_key
        .as_deref()
        .map(|key| resolver::flow(key, None))
        .transpose()?
        .flatten();
    let revision = cached_flow
        .as_ref()
        .map_or_else(|| digest_bytes(text.as_bytes()), |flow| flow.revision);
    let line_offsets: Vec<_> = if cached_flow.is_some() {
        vec![]
    } else {
        std::iter::once(0)
            .chain(text.match_indices('\n').map(|(at, _)| at + 1))
            .collect()
    };
    let logical_lines = cached_flow
        .as_ref()
        .map_or(line_offsets.len(), |flow| flow.lines);
    let mut events = Vec::<(usize, bool, usize, usize)>::new();
    if cached_flow.is_none() {
        let ranges: Vec<_> = options
            .spans
            .iter()
            .map(|span| span.range.clone())
            .chain(selection.iter().cloned())
            .collect();
        let resolved = sanim_core::code::view::resolve_ranges(id, document, &ranges)?;
        for (index, highlight) in document.highlights.iter().enumerate() {
            if highlight.start < highlight.end && highlight.end <= text.len() {
                events.push((highlight.start, true, 0, index));
                events.push((highlight.end, false, 0, index));
            }
        }
        for (index, bounds) in resolved.iter().take(options.spans.len()).enumerate() {
            events.push((bounds.start, true, 1, index));
            events.push((bounds.end, false, 1, index));
        }
        for (index, bounds) in resolved.iter().skip(options.spans.len()).enumerate() {
            events.push((bounds.start, true, 2, index));
            events.push((bounds.end, false, 2, index));
        }
    }
    events.sort_unstable();
    let mut active_highlights = BTreeSet::new();
    let mut active_spans = BTreeSet::new();
    let mut active_selection = BTreeSet::new();
    let mut event_at = 0;
    let space = run(&options.font, options.font_size, " ", &Style::default())?.advance;
    let line_height = math::to_f64(options.line_height)?;
    let font_size = math::to_f64(options.font_size)?;
    let gutter = gutter::Gutter::new(options, logical_lines, space)?;
    let number_width = gutter.number_width;
    let left = gutter.left;
    let mut indicators = if cached_flow.is_none() {
        gutter.indicators(id, document, options, folded, revision)?
    } else {
        BTreeMap::new()
    };
    let mut tokens = imbl::Vector::new();
    let mut token_count = 0;
    let mut widest = left + space;
    let mut visible_row = 0usize;
    let mut source_rows = Vec::new();
    let mut hidden_until = 0usize;
    let mut collapses = BTreeMap::new();
    if cached_flow.is_none() {
        for scope in document
            .scopes
            .iter()
            .filter(|scope| folded.contains(&scope.id))
        {
            let end = scope.hidden_end_line(text)?;
            collapses
                .entry(scope.start_line)
                .and_modify(|last: &mut usize| *last = (*last).max(end))
                .or_insert(end);
        }
    }
    let default_fill = Color::parse("#c0bfbc")?;
    let mut glyph = |word: &str,
                     line: usize,
                     column: usize,
                     position: [f64; 2],
                     style: &Style,
                     tag: &str|
     -> Result<(Token, f64), String> {
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
                    style.fill.clone().unwrap_or_else(|| default_fill.clone()),
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
                    style.fill.clone().unwrap_or_else(|| default_fill.clone()),
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
                            style.fill.clone().unwrap_or_else(|| default_fill.clone()),
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
            glyph.position.values
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
        Ok((token, shaped.advance))
    };
    if let Some(flow) = cached_flow {
        return draw_flow(&flow, options, recipe, left, space, &mut glyph);
    }
    let mut commands = Vec::new();
    let mut add = |word: &str,
                   line: usize,
                   column: usize,
                   position: [f64; 2],
                   style: &Style,
                   tag: &'static str|
     -> Result<f64, String> {
        if recipe.white_space == sanim_core::text::WhiteSpace::Pre {
            return match tag {
                "newline" => Ok(0.),
                "space" => Ok(run(&options.font, options.font_size, word, style)?.advance),
                _ => {
                    let (token, advance) = glyph(word, line, column, position, style, tag)?;
                    tokens.push_back(token);
                    token_count += 1;
                    if token_count > MAX_CODE_TOKENS {
                        return Err(format!(
                            "CodeView exceeds {MAX_CODE_TOKENS} displayed tokens; fold scopes or show a shorter file"
                        ));
                    }
                    Ok(advance)
                }
            };
        }
        let advance = if tag == "newline" {
            0.
        } else {
            run(&options.font, options.font_size, word, style)?.advance
        };
        if commands.len() >= MAX_CODE_TOKENS {
            return Err(format!(
                "CodeView exceeds {MAX_CODE_TOKENS} source layout runs; fold scopes or show a shorter file"
            ));
        }
        commands.push(Command {
            word: word.into(),
            line,
            column,
            style: style.clone(),
            tag,
        });
        Ok(advance)
    };
    let mut indicator_rows = Vec::new();
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
        if recipe.white_space == sanim_core::text::WhiteSpace::Pre {
            if source_rows.len() >= MAX_CODE_TOKENS {
                return Err(format!(
                    "CodeView exceeds {MAX_CODE_TOKENS} visual rows; fold scopes or show a shorter file"
                ));
            }
            source_rows.push(SourceRow {
                start: (line, 1),
                end: (line + 1, 1),
                top: visible_row as f64 * line_height,
                bottom: (visible_row + 1) as f64 * line_height,
            });
        }
        if recipe.white_space == sanim_core::text::WhiteSpace::Pre && indicators.contains_key(&line)
        {
            indicator_rows.push((line, baseline));
        }
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
                if recipe.white_space != sanim_core::text::WhiteSpace::Pre {
                    add("\t", line, column, [x, baseline], &style, "space")?;
                }
                let remaining = options.tab_size - (visual_column - 1) % options.tab_size;
                x += remaining as f64 * space;
                column += 1;
                visual_column += remaining;
            } else {
                if category == 1 && style.background.is_none() {
                    x += add(word, line, column, [x, baseline], &style, "space")?;
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
        add(
            "\n",
            line,
            column,
            [x, baseline],
            &Style::default(),
            "newline",
        )?;
        widest = widest.max(x);
    }
    if recipe.white_space != sanim_core::text::WhiteSpace::Pre {
        let flow = prepare_flow(
            &commands,
            document,
            recipe.white_space,
            logical_lines,
            revision,
            options,
            &mut glyph,
        )?;
        let mut flow = flow;
        flow.indicators = std::mem::take(&mut indicators);
        let visible: BTreeSet<_> = commands
            .iter()
            .filter(|command| command.tag == "newline")
            .map(|command| command.line)
            .collect();
        flow.indicators.retain(|line, _| visible.contains(line));
        let flow = resolver::flow(
            flow_key.as_deref().expect("wrapped source flow key"),
            Some(flow),
        )?
        .expect("stored source flow");
        return draw_flow(&flow, options, recipe, left, space, &mut glyph);
    }
    for (line, baseline) in indicator_rows {
        let mut marker = indicators.remove(&line).expect("visible scope control");
        marker.position.values[1] += baseline;
        tokens.push_back(marker);
    }
    if tokens.len() > MAX_CODE_TOKENS {
        return Err(format!(
            "CodeView exceeds {MAX_CODE_TOKENS} displayed tokens; fold scopes or hide gutter controls"
        ));
    }
    Ok(Prepared {
        tokens,
        rows: source_rows,
        size: [
            widest.max(space),
            ((visible_row.max(1)) as f64 * line_height).max(font_size),
        ],
    })
}

/// Shape the visible source as one paragraph, then split only at Pango's visual
/// row boundaries. Source coordinates remain logical coordinates throughout.
fn source_flow_key(
    id: &str,
    document: &Document,
    options: &Options,
    folded: &imbl::OrdSet<Arc<str>>,
    selection: &[Range],
    white_space: sanim_core::text::WhiteSpace,
) -> Result<String, String> {
    serde_json::to_vec(&(
        "flow",
        id,
        resolver::document_identity(document),
        options.visual_recipe(resolver::font_identity(&options.font)),
        matches!(&options.font, Font::Family(_))
            .then(crate::font::system_font_epoch)
            .flatten(),
        folded,
        selection,
        white_space,
    ))
    .map(|bytes| digest(&bytes))
    .map_err(|error| error.to_string())
}
struct Piece {
    start: usize,
    end: usize,
    line: usize,
    column: usize,
    style: Arc<Style>,
    tag: &'static str,
    template: Option<Token>,
}
pub(super) struct Flow {
    document: Arc<Document>,
    source: shrimply_fonts::native::shaping::Source,
    pieces: Vec<Piece>,
    numbers: BTreeMap<usize, Token>,
    indicators: BTreeMap<usize, Token>,
    font: Font,
    lines: usize,
    last_line: usize,
    source_end: (usize, usize),
    revision: [u8; 32],
}
impl Flow {
    fn retained_bytes(&self) -> Result<usize, String> {
        let mut bytes = resolver::MAX_LAYOUT_BYTES;
        let mut items = resolver::MAX_LAYOUT_ITEMS;
        let mut shared = value_budget::Budget::default();
        value_budget::charge_with_budget(
            &Value::Code(self.document.clone()),
            &mut bytes,
            &mut items,
            resolver::MAX_LAYOUT_DEPTH,
            &mut shared,
        )?;
        let templates = self
            .pieces
            .iter()
            .filter_map(|piece| piece.template.as_ref())
            .chain(self.numbers.values())
            .chain(self.indicators.values())
            .map(|token| Value::Element(token.element.clone()))
            .collect();
        value_budget::charge_with_budget(
            &Value::List(List::typed(Type::Element, templates)),
            &mut bytes,
            &mut items,
            resolver::MAX_LAYOUT_DEPTH,
            &mut shared,
        )?;
        value_budget::charge_with_budget(
            &Value::Font(self.font.clone()),
            &mut bytes,
            &mut items,
            resolver::MAX_LAYOUT_DEPTH,
            &mut shared,
        )?;
        let token_bytes = self
            .pieces
            .iter()
            .filter_map(|piece| piece.template.as_ref())
            .chain(self.numbers.values())
            .chain(self.indicators.values())
            .map(|token| std::mem::size_of::<Token>() + token.key.len())
            .sum::<usize>();
        Ok(resolver::MAX_LAYOUT_BYTES - bytes
            + token_bytes
            + self.numbers.len() * std::mem::size_of::<(usize, Token)>()
            + self.indicators.len() * std::mem::size_of::<(usize, Token)>()
            + std::mem::size_of::<Self>()
            + self.source.retained_bytes()
            + self.pieces.capacity() * std::mem::size_of::<Piece>()
            + self.pieces.len() * std::mem::size_of::<Style>())
    }
}
fn prepare_flow(
    commands: &[Command],
    document: &Document,
    mode: sanim_core::text::WhiteSpace,
    lines: usize,
    revision: [u8; 32],
    options: &Options,
    glyph: &mut impl FnMut(&str, usize, usize, [f64; 2], &Style, &str) -> Result<(Token, f64), String>,
) -> Result<Flow, String> {
    let mut raw = String::new();
    let mut sources = Vec::new();
    for command in commands
        .iter()
        .filter(|command| command.tag != "line-number")
    {
        sources.push((raw.len(), command));
        raw.push_str(&command.word);
    }
    if raw.ends_with('\n') {
        raw.pop();
    }
    let (text, mapping) = sanim_core::text::normalize(&raw, mode);
    let mut pieces: Vec<Piece> = Vec::new();
    let mut source_index = 0;
    let mut previous = None;
    for ((offset, ch), source) in text.char_indices().zip(mapping.iter().copied()) {
        while sources
            .get(source_index + 1)
            .is_some_and(|(start, _)| *start <= source)
        {
            source_index += 1;
        }
        let Some(&(start, command)) = sources.get(source_index) else {
            continue;
        };
        if previous == Some(source_index) {
            pieces.last_mut().expect("existing source piece").end = offset + ch.len_utf8();
        } else {
            pieces.push(Piece {
                start: offset,
                end: offset + ch.len_utf8(),
                line: command.line,
                column: command.column + command.word[..source - start].chars().count(),
                style: Arc::new(command.style.clone()),
                tag: command.tag,
                template: None,
            });
        }
        previous = Some(source_index);
    }
    let mut spans: Vec<shrimply_fonts::native::shaping::StyleSpan> = Vec::new();
    for piece in &pieces {
        let weight = if piece.style.bold.unwrap_or(false) {
            700
        } else {
            400
        };
        let style = if piece.style.italic.unwrap_or(false) {
            pango::Style::Italic
        } else {
            pango::Style::Normal
        };
        // Default font attributes need no Pango range at all. Merge adjacent
        // nondefault ranges so token colors do not force extra font itemization.
        if weight == 400 && style == pango::Style::Normal {
            continue;
        }
        if let Some(last) = spans.last_mut().filter(|last| {
            last.end as usize == piece.start && last.weight == weight && last.style == style
        }) {
            last.end = piece.end as u32;
        } else {
            spans.push(shrimply_fonts::native::shaping::StyleSpan {
                start: piece.start as u32,
                end: piece.end as u32,
                weight,
                style,
            });
        }
    }
    for piece in &mut pieces {
        let word = &text[piece.start..piece.end];
        if !word.trim().is_empty() || piece.style.background.is_some() {
            piece.template = Some(
                glyph(
                    word,
                    piece.line,
                    piece.column,
                    [0.; 2],
                    &piece.style,
                    piece.tag,
                )?
                .0,
            );
        }
    }
    let mut numbers = BTreeMap::new();
    if options.line_numbers {
        let digits = "0".repeat(lines.to_string().len());
        let width = run(&options.font, options.font_size, &digits, &Style::default())?.advance;
        let style = Style {
            fill: Some(options.theme.line_number.clone()),
            ..Default::default()
        };
        let visible: BTreeSet<_> = commands
            .iter()
            .filter(|command| command.tag == "newline")
            .map(|command| command.line)
            .collect();
        for line in visible {
            let (mut token, advance) =
                glyph(&line.to_string(), line, 1, [0.; 2], &style, "line-number")?;
            token.position.values[0] += width - advance;
            numbers.insert(line, token);
        }
    }
    Ok(Flow {
        document: Arc::new(document.clone()),
        source: shrimply_fonts::native::shaping::Source::styled(text.into(), spans.into())?,
        pieces,
        numbers,
        indicators: BTreeMap::new(),
        font: options.font.clone(),
        lines,
        last_line: commands
            .iter()
            .rev()
            .find(|command| command.tag == "newline")
            .map_or(1, |command| command.line),
        source_end: sanim_core::code::view::SourceIndex::new(&document.text)
            .position(document.text.len())?,
        revision,
    })
}
fn draw_flow(
    flow: &Flow,
    options: &Options,
    recipe: sanim_core::text::Recipe,
    left: f64,
    space: f64,
    glyph: &mut impl FnMut(&str, usize, usize, [f64; 2], &Style, &str) -> Result<(Token, f64), String>,
) -> Result<Prepared, String> {
    let mut tokens = imbl::Vector::new();
    let text = flow.source.text();
    let pieces = &flow.pieces;
    let mut native = crate::font::layout_options(options.font_size, recipe)?;
    native.width = native.width.map(|width| (width - left).max(0.));
    native.tab_width = Some(space * options.tab_size as f64);
    let native =
        shrimply_fonts::native::shaping::layout_shared(&options.font, &flow.source, &native)?;
    if native.metrics.missing_glyphs != 0 || native.metrics.color_glyphs {
        return Err("CodeView contains unsupported glyphs; choose an outline font/fallback covering the requested text".into());
    }
    // Pango's index_to_x/index_to_pos rescan UTF-8 from the whole source start.
    // Its incremental character iterator avoids that quadratic prefix work and
    // gives visual rectangles directly, including RTL and ligature clusters.
    let mut characters = Vec::new();
    let mut cursor = native.layout().iter();
    loop {
        let index = cursor.index() as usize;
        if index < text.len() {
            let rect = cursor.char_extents();
            characters.push((index, rect.x().min(rect.x() + rect.width())));
        }
        if !cursor.next_char() {
            break;
        }
    }
    characters.sort_unstable_by_key(|(index, _)| *index);
    // Pango's character iterator visits Unicode scalars, including spaces and
    // combining characters. Reuse that sorted index for every source column;
    // rescanning a long token prefix for each wrapped row is quadratic.
    let source_column = |piece: &Piece, at: usize| {
        piece.column + characters.partition_point(|(index, _)| *index < at)
            - characters.partition_point(|(index, _)| *index < piece.start)
    };
    let native_rows = native.layout().lines_readonly();
    let rows: Vec<usize> = native_rows
        .iter()
        .map(|line| line.start_index() as usize)
        .collect();
    if rows.len() > MAX_CODE_TOKENS {
        return Err(format!(
            "CodeView exceeds {MAX_CODE_TOKENS} visual rows; fold scopes or show a shorter file"
        ));
    }
    let line_height = math::to_f64(options.line_height)?;
    let size = math::to_f64(options.font_size)?;
    let mut numbered = BTreeSet::new();
    let mut piece_index = 0;
    let mut source_rows = Vec::with_capacity(rows.len());
    for (row, start) in rows.iter().copied().enumerate() {
        let end = rows.get(row + 1).copied().unwrap_or(text.len());
        while pieces
            .get(piece_index)
            .is_some_and(|piece| piece.end <= start)
        {
            piece_index += 1;
        }
        let line = pieces.get(piece_index).map_or(
            if recipe.white_space.preserves_newlines() {
                flow.last_line
            } else {
                1
            },
            |piece| piece.line,
        );
        let row_pieces = &pieces[piece_index..pieces.partition_point(|piece| piece.start < end)];
        let start_position = row_pieces.first().map_or((line, 1), |piece| {
            (piece.line, source_column(piece, start.max(piece.start)))
        });
        let end_position = row_pieces.last().map_or(start_position, |piece| {
            let until = end.min(piece.end);
            if text[piece.start..until].ends_with('\n') {
                (piece.line + 1, 1)
            } else {
                (piece.line, source_column(piece, until))
            }
        });
        source_rows.push(SourceRow {
            start: start_position,
            end: end_position,
            top: row as f64 * line_height,
            bottom: (row + 1) as f64 * line_height,
        });
        if numbered.insert(line) {
            if let Some(template) = flow.numbers.get(&line) {
                tokens.push_back(positioned(template, [0., row as f64 * line_height + size]));
            }
            if let Some(template) = flow.indicators.get(&line) {
                tokens.push_back(positioned(template, [0., row as f64 * line_height + size]));
            }
        }
        for piece in pieces[piece_index..]
            .iter()
            .take_while(|piece| piece.start < end)
        {
            let at = piece.start.max(start);
            let until = piece.end.min(end);
            let word = text[at..until].trim_end_matches(['\n', '\r']);
            if word.is_empty()
                || (word.chars().all(char::is_whitespace) && piece.style.background.is_none())
            {
                continue;
            }
            let column = source_column(piece, at);
            let first_char = characters.partition_point(|(index, _)| *index < at);
            let last_char = characters.partition_point(|(index, _)| *index < at + word.len());
            let x = characters[first_char..last_char]
                .iter()
                .map(|(_, x)| *x)
                .min()
                .ok_or("CodeView native layout omitted a source character")?;
            let position = [
                left + f64::from(x) / f64::from(pango::SCALE),
                row as f64 * line_height + size,
            ];
            let token = if at == piece.start
                && at + word.len() == piece.end
                && let Some(template) = &piece.template
            {
                positioned(template, position)
            } else {
                glyph(word, piece.line, column, position, &piece.style, piece.tag)?.0
            };
            tokens.push_back(token);
            if tokens.len() > MAX_CODE_TOKENS {
                return Err(format!(
                    "CodeView exceeds {MAX_CODE_TOKENS} displayed tokens; fold scopes or show a shorter file"
                ));
            }
        }
    }
    // Collapsed whitespace between native rows belongs to the preceding row;
    // retain that source mapping even when it contributes no painted glyph.
    if let Some(first) = source_rows.first_mut() {
        first.start = (1, 1);
    }
    for index in 1..source_rows.len() {
        let next = source_rows[index].start;
        if source_rows[index - 1].end < next {
            source_rows[index - 1].end = next;
        }
    }
    if let Some(last) = source_rows.last_mut() {
        last.end = last.end.max(flow.source_end);
    }
    Ok(Prepared {
        tokens,
        rows: source_rows,
        size: [
            (left + native.metrics.advance).max(space),
            (rows.len().max(1) as f64 * line_height).max(size),
        ],
    })
}
fn positioned(template: &Token, position: [f64; 2]) -> Token {
    let mut token = template.clone();
    for (value, offset) in token.position.values.iter_mut().zip(position) {
        *value += offset;
    }
    token
}

#[cfg(test)]
mod wrapping_tests {
    use super::*;
    use sanim_core::text::{OverflowWrap, WhiteSpace};
    fn options() -> Options {
        Options {
            font: Font::family("monospace", "monospace").unwrap(),
            font_size: 12.into(),
            line_height: 18.into(),
            auto_line_height: false,
            tab_size: 4,
            line_numbers: true,
            scope_indicators: false,
            theme: Default::default(),
            spans: vec![],
        }
    }
    #[test]
    fn resizing_reuses_the_source_map_styles_and_normalized_text() {
        let document = crate::code::parse(
            Document::source_shared(
                "",
                "def hello():\n    return 'a long example with many wrapped words'".into(),
                sanim_core::code::Language::Python,
            )
            .unwrap(),
        )
        .unwrap();
        let options = options();
        let key = source_flow_key(
            "code",
            &document,
            &options,
            &Default::default(),
            &[],
            WhiteSpace::PreWrap,
        )
        .unwrap();
        let recipe = |width| sanim_core::text::Recipe {
            white_space: WhiteSpace::PreWrap,
            overflow_wrap: OverflowWrap::Anywhere,
            width: Some(Fraction::from(width)),
        };
        let initial = prepare_layout(
            "code",
            &document,
            &options,
            &Default::default(),
            &[],
            recipe(120),
        )
        .unwrap();
        let retained = initial.tokens.iter().find(|token| matches!(token.element.geometry.as_ref(), Geometry::Text { text, .. } if text.as_ref() == "def")).unwrap().element.clone();
        let original = resolver::flow(&key, None).unwrap().unwrap();
        for width in [200, 180, 160, 140, 220] {
            let changed = prepare_layout(
                "code",
                &document,
                &options,
                &Default::default(),
                &[],
                recipe(width),
            )
            .unwrap();
            let unchanged = changed.tokens.iter().find(|token| matches!(token.element.geometry.as_ref(), Geometry::Text { text, .. } if text.as_ref() == "def")).unwrap();
            assert!(
                Arc::ptr_eq(&retained, &unchanged.element),
                "width changes must reuse whole glyph elements"
            );
            let current = resolver::flow(&key, None).unwrap().unwrap();
            assert!(Arc::ptr_eq(&original, &current));
            assert!(Arc::ptr_eq(original.source.text(), current.source.text()));
            assert!(Arc::ptr_eq(original.source.spans(), current.source.spans()));
            assert!(
                original
                    .document
                    .syntax
                    .shares_tree(&current.document.syntax)
            );
        }
    }
    #[test]
    fn long_wrapped_reflow_rejects_quadratic_prefix_shaping_before_native_work() {
        let document = Document::source_shared(
            "",
            "A".repeat(8192).into(),
            sanim_core::code::Language::Plain,
        )
        .unwrap();
        let options = options();
        let from = prepare(
            "bounded-fragments",
            &document,
            &options,
            &Default::default(),
            &[],
        )
        .unwrap();
        let to = prepare_layout(
            "bounded-fragments",
            &document,
            &options,
            &Default::default(),
            &[],
            sanim_core::text::Recipe {
                white_space: WhiteSpace::PreWrap,
                overflow_wrap: OverflowWrap::Anywhere,
                width: Some(90.into()),
            },
        )
        .unwrap();
        let half = Fraction::new(1u64, 2u64);
        let error = transition_tokens(
            &from.tokens,
            &to.tokens,
            half,
            half,
            half,
            sanim_core::code::view::PatchAnimation::Morph,
        )
        .unwrap_err();
        assert!(error.contains("shaping work budget"), "{error}");
    }
    #[test]
    fn long_wrapped_unicode_tokens_keep_exact_columns_for_every_visual_row() {
        let text = "αβ".repeat(512);
        let document =
            Document::source_shared("", text.clone().into(), sanim_core::code::Language::Plain)
                .unwrap();
        let prepared = prepare_layout(
            "unicode-columns",
            &document,
            &options(),
            &Default::default(),
            &[],
            sanim_core::text::Recipe {
                white_space: WhiteSpace::PreWrap,
                overflow_wrap: OverflowWrap::Anywhere,
                width: Some(90.into()),
            },
        )
        .unwrap();
        let source: Vec<_> = text.chars().collect();
        assert!(prepared.rows.len() > 100);
        for token in prepared.tokens.iter().filter(|token| !token.is_gutter()) {
            let Geometry::Text { text, .. } = token.element.geometry.as_ref() else {
                panic!("plain token text");
            };
            let expected: String = source[token.column - 1..]
                .iter()
                .take(text.chars().count())
                .collect();
            assert_eq!(text.as_ref(), expected);
        }
    }
    #[test]
    #[ignore = "relative CPU benchmark; run make wrapped-token-bench"]
    fn long_wrapped_token_scaling_benchmark() {
        let mut timings = Vec::new();
        for length in [8192, 32768] {
            let document = Document::source_shared(
                "",
                "A".repeat(length).into(),
                sanim_core::code::Language::Plain,
            )
            .unwrap();
            let mut samples = Vec::new();
            for sample in 0..9 {
                let started = std::time::Instant::now();
                let prepared = prepare_layout(
                    "token-scaling",
                    &document,
                    &options(),
                    &Default::default(),
                    &[],
                    sanim_core::text::Recipe {
                        white_space: WhiteSpace::PreWrap,
                        overflow_wrap: OverflowWrap::Anywhere,
                        width: Some(90.into()),
                    },
                )
                .unwrap();
                assert!(prepared.rows.len() > length / 20);
                std::hint::black_box(prepared);
                if sample > 0 {
                    samples.push(started.elapsed());
                }
            }
            samples.sort();
            timings.push(samples[4]);
        }
        let ratio = timings[1].as_secs_f64() / timings[0].as_secs_f64();
        eprintln!(
            "native wrapped token scaling: 8192={:?}, 32768={:?}, ratio={ratio:.2}",
            timings[0], timings[1]
        );
        assert!(
            ratio < 8.,
            "4x source length must stay below quadratic 16x scaling"
        );
    }
    #[test]
    #[ignore = "relative CPU benchmark; run make text-wrapping-bench"]
    fn width_reflow_benchmark() {
        let text = (0..100).map(|index| format!("def function_{index}():\n    return 'A long line of source text with several word boundaries'\n")).collect::<String>();
        let document = crate::code::parse(
            Document::source_shared("", text.clone().into(), sanim_core::code::Language::Python)
                .unwrap(),
        )
        .unwrap();
        let options = options();
        let mut warm = Vec::new();
        let mut cold = Vec::new();
        for sample in 0..15 {
            for cold_source in if sample % 2 == 0 {
                [false, true]
            } else {
                [true, false]
            } {
                let mut source = document.clone();
                if cold_source {
                    source.text = Arc::from(text.as_str());
                }
                let started = std::time::Instant::now();
                for width in [180, 220, 260, 300] {
                    prepare_layout(
                        "code",
                        &source,
                        &options,
                        &Default::default(),
                        &[],
                        sanim_core::text::Recipe {
                            white_space: WhiteSpace::PreWrap,
                            overflow_wrap: OverflowWrap::Anywhere,
                            width: Some(width.into()),
                        },
                    )
                    .unwrap();
                }
                if cold_source {
                    cold.push(started.elapsed());
                } else {
                    warm.push(started.elapsed());
                }
            }
        }
        warm.sort();
        cold.sort();
        eprintln!(
            "100-function width reflow, four widths: shared flow median {:?}; rebuilt flow median {:?}; speedup {:.2}x",
            warm[7],
            cold[7],
            cold[7].as_secs_f64() / warm[7].as_secs_f64()
        );
        assert!(
            warm[7] < cold[7],
            "retaining the source flow must beat rebuilding it"
        );
    }
    #[test]
    fn excessive_layout_runs_fail_before_building_source_maps_or_native_paragraphs() {
        let document = Document::source_shared(
            "",
            "a,".repeat(MAX_CODE_TOKENS).into(),
            sanim_core::code::Language::Plain,
        )
        .unwrap();
        let error = prepare_layout(
            "large",
            &document,
            &options(),
            &Default::default(),
            &[],
            sanim_core::text::Recipe {
                white_space: WhiteSpace::PreWrap,
                overflow_wrap: OverflowWrap::Anywhere,
                width: Some(120.into()),
            },
        )
        .err()
        .expect("bounded source run error");
        assert!(error.contains("source layout runs"), "{error}");
    }
    #[test]
    fn family_font_epoch_invalidates_glyph_templates() {
        use pango::prelude::FontMapExt;
        let document =
            Document::source_shared("", "a sample".into(), sanim_core::code::Language::Plain)
                .unwrap();
        let options = options();
        let before = source_flow_key(
            "code",
            &document,
            &options,
            &Default::default(),
            &[],
            WhiteSpace::PreWrap,
        )
        .unwrap();
        shrimply_fonts::native::system_map().unwrap().changed();
        let after = source_flow_key(
            "code",
            &document,
            &options,
            &Default::default(),
            &[],
            WhiteSpace::PreWrap,
        )
        .unwrap();
        assert_ne!(
            before, after,
            "changed family font metrics must rebuild cached glyph templates"
        );
    }
}
