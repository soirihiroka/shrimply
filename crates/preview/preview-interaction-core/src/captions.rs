use shrimply_project::caption::markup::{self, Span};
use shrimply_project::project::{
    CaptionFont, CaptionItem, HorizontalAlign, ItemAddress, Project, Time, VerticalAlign,
};
use shrimply_skia_adw_core::canvas::{Color, Rect, TimelinePainter, Vec2, vec2};
use skia_safe::{
    FontMgr, FontStyle, Point,
    textlayout::{
        FontCollection, Paragraph, ParagraphBuilder, ParagraphStyle, RectHeightStyle,
        RectWidthStyle, TextAlign, TextDecoration, TextStyle,
    },
};

const PREVIEW_CAPTION_SAFE_MARGIN: f32 = 24.0;
const PREVIEW_CAPTION_BOTTOM_PADDING: f32 = 24.0;
const PREVIEW_CAPTION_PADDING_X: f32 = 8.0;
const PREVIEW_CAPTION_PADDING_Y: f32 = 3.0;
const PARAGRAPH_LAYOUT_EPSILON: f32 = 1.0;
const CAPTION_SPLIT_CARET_WIDTH: f32 = 2.0;
const CAPTION_SPLIT_CARET_PADDING: f32 = 2.0;

thread_local! {
    static CAPTION_FONTS: FontCollection = {
        let mut fonts = FontCollection::new();
        fonts.set_default_font_manager(FontMgr::new(), None);
        fonts
    };
}

#[derive(Clone)]
struct CaptionFontId {
    size: f32,
    font: CaptionFont,
}

struct CaptionLayout {
    paragraph: Paragraph,
    spans: Vec<Span>,
    rendered: String,
    text_pos: Vec2,
    text_size: Vec2,
    background_rect: Rect,
    font: CaptionFontId,
    wrap_width: f32,
    elapsed_millis: u32,
    vertical: bool,
}

struct CaptionSplit {
    text_byte: usize,
    caret: Rect,
}

#[derive(Clone, Copy)]
pub struct CaptionAppearance {
    pub preview_rect: Rect,
    pub font_size: f32,
    pub background_color: Color<u8>,
    pub bottom_inset: f32,
}

fn caption_paragraph(
    spans: &[Span],
    font: CaptionFontId,
    color: Color,
    width: f32,
    align: HorizontalAlign,
) -> Paragraph {
    let mut paragraph_style = ParagraphStyle::new();
    paragraph_style.set_text_align(match align {
        HorizontalAlign::Left => TextAlign::Left,
        HorizontalAlign::Center => TextAlign::Center,
        HorizontalAlign::Right => TextAlign::Right,
    });
    let mut builder =
        CAPTION_FONTS.with(|fonts| ParagraphBuilder::new(&paragraph_style, fonts.clone()));
    for span in spans {
        let mut style = TextStyle::new();
        let family = match font.font {
            CaptionFont::Roboto => "Roboto",
            CaptionFont::MonospaceSerif => "Courier New",
            CaptionFont::Serif => "Times New Roman",
            CaptionFont::MonospaceSans => "Lucida Console",
            CaptionFont::Casual => "Comic Sans MS",
            CaptionFont::Cursive => "Monotype Corsiva",
            CaptionFont::SmallCapitals => "Arial",
        };
        style
            .set_color(color)
            .set_font_size(font.size.max(1.0))
            .set_font_families(&[family]);
        style.set_font_style(match (span.bold, span.italic) {
            (true, true) => FontStyle::bold_italic(),
            (true, false) => FontStyle::bold(),
            (false, true) => FontStyle::italic(),
            _ => FontStyle::normal(),
        });
        if span.underline {
            style.set_decoration_type(TextDecoration::UNDERLINE);
        }
        builder.push_style(&style).add_text(
            span.ruby
                .as_ref()
                .map_or(span.text.as_str(), |ruby| ruby.base.as_str()),
        );
    }
    let mut paragraph = builder.build();
    paragraph.layout(f32::MAX);
    let unwrapped_width = paragraph.max_intrinsic_width() + PARAGRAPH_LAYOUT_EPSILON;
    paragraph.layout(if unwrapped_width <= width {
        unwrapped_width
    } else {
        width.max(1.0)
    });
    paragraph
}

pub fn draw_captions(
    painter: &TimelinePainter,
    project: &Project,
    position: Time,
    appearance: CaptionAppearance,
    split: Option<(&ItemAddress, Vec2, Color)>,
) {
    let CaptionAppearance {
        preview_rect,
        font_size,
        background_color,
        bottom_inset,
    } = appearance;
    if preview_rect.width() <= 1.0 || preview_rect.height() <= 1.0 {
        return;
    }

    let active = active_captions(project, position, background_color);
    let mut bottom_stack = bottom_inset;
    for item in active {
        let automatic_bottom = item.h_align == HorizontalAlign::Center
            && item.v_align == VerticalAlign::Bottom
            && item.position_x == 50
            && item.position_y == 90;
        let layout = caption_layout(
            &item,
            position,
            preview_rect,
            font_size,
            automatic_bottom,
            if automatic_bottom { bottom_stack } else { 0.0 },
        );
        let height = layout.as_ref().map_or(0.0, |layout| layout.text_size.y);
        if let Some(layout) = layout {
            let split = split
                .filter(|(address, _, _)| {
                    address.item_id() == item.id && item.start < position && position < item.end
                })
                .and_then(|(_, point, color)| {
                    split_for_layout(&item, &layout, point).map(|split| (split, color))
                });
            draw_caption(painter, &item, layout, split);
        }
        if automatic_bottom {
            bottom_stack += height + PREVIEW_CAPTION_PADDING_Y * 2.0;
        }
    }
}

pub fn split_at_position(
    project: &Project,
    address: &ItemAddress,
    position: Time,
    appearance: CaptionAppearance,
    point: Vec2,
) -> Option<usize> {
    let CaptionAppearance {
        preview_rect,
        font_size,
        background_color,
        bottom_inset,
    } = appearance;
    let selected = project.caption_item(address)?;
    if !(selected.start < position && position < selected.end) {
        return None;
    }
    let mut bottom_stack = bottom_inset;
    for item in active_captions(project, position, background_color) {
        let automatic_bottom = item.h_align == HorizontalAlign::Center
            && item.v_align == VerticalAlign::Bottom
            && item.position_x == 50
            && item.position_y == 90;
        let layout = caption_layout(
            &item,
            position,
            preview_rect,
            font_size,
            automatic_bottom,
            if automatic_bottom { bottom_stack } else { 0.0 },
        );
        if automatic_bottom {
            bottom_stack += layout.as_ref().map_or(0.0, |layout| layout.text_size.y)
                + PREVIEW_CAPTION_PADDING_Y * 2.0;
        }
        if item.id == address.item_id() {
            return split_for_layout(&item, &layout?, point).map(|split| split.text_byte);
        }
    }
    None
}

fn active_captions(
    project: &Project,
    position: Time,
    caption_background_color: Color<u8>,
) -> Vec<CaptionItem> {
    let mut active = project
        .caption_tracks
        .iter()
        .rev()
        .filter(|track| track.enabled)
        .flat_map(|track| &track.items)
        .filter(|item| position >= item.start && position < item.end)
        .cloned()
        .collect::<Vec<_>>();
    active.sort_by_key(|item| {
        (
            if item.layout_enabled {
                item.position_y
            } else {
                90
            },
            item.start,
        )
    });
    active
        .into_iter()
        .map(|mut item| {
            let defaults = (!item.styling_enabled || !item.layout_enabled)
                .then(|| CaptionItem::new(item.start, item.end, item.text.clone()));
            if let Some(defaults) = defaults {
                if !item.styling_enabled {
                    item.text_color = defaults.text_color;
                    item.background_color = caption_background_color;
                    item.edge_color = defaults.edge_color;
                    item.edge_style = defaults.edge_style;
                    item.font = defaults.font;
                    item.font_scale = defaults.font_scale;
                }
                if !item.layout_enabled {
                    item.h_align = defaults.h_align;
                    item.v_align = defaults.v_align;
                    item.position_x = defaults.position_x;
                    item.position_y = defaults.position_y;
                }
            }
            item
        })
        .collect()
}

fn caption_layout(
    item: &CaptionItem,
    playback_position: Time,
    preview_rect: Rect,
    caption_font_size: f32,
    automatic_bottom: bool,
    caption_bottom_inset: f32,
) -> Option<CaptionLayout> {
    if item.text.trim().is_empty() || item.text_color.a == 0 {
        return None;
    }

    let margin = PREVIEW_CAPTION_SAFE_MARGIN;
    let wrap_width = (preview_rect.width() - margin * 2.0).max(1.0);
    // Font size and decoration are deliberately constant in preview pixels. Only the caption's
    // percentage position follows the preview canvas; neither canvas dimensions nor video bounds
    // may scale these metrics.
    let font_size = caption_font_size * f32::from(item.font_scale) / 100.0;
    let font = CaptionFontId {
        size: font_size,
        font: item.font,
    };
    let text_color = item.text_color.into();
    let elapsed_millis =
        (playback_position.as_nanos_i128() - item.start.as_nanos_i128()).max(0) / 1_000_000;
    let elapsed_millis = elapsed_millis.min(i128::from(u32::MAX)) as u32;
    let mut spans = markup::visible_spans(&item.text, elapsed_millis);
    for span in &mut spans {
        if let Some(ruby) = span.ruby.take() {
            span.text = format!("{}({})", ruby.base, ruby.annotation);
        }
    }
    let vertical =
        item.writing_direction != shrimply_project::project::CaptionWritingDirection::Horizontal;
    let mut rendered = markup::plain_text_from_spans(&spans);
    if vertical {
        rendered = rendered
            .chars()
            .flat_map(|character| [character, '\n'])
            .collect();
        spans = vec![Span {
            text: rendered.clone(),
            bold: false,
            italic: false,
            underline: false,
            start_millis: 0,
            ruby: None,
        }];
    }
    let paragraph = caption_paragraph(&spans, font.clone(), text_color, wrap_width, item.h_align);
    let text_size = Vec2::new(paragraph.max_width(), paragraph.height());
    if text_size.x <= 0.0 || text_size.y <= 0.0 {
        return None;
    }
    let text_pos = caption_position(
        item,
        preview_rect,
        margin,
        text_size,
        automatic_bottom,
        caption_bottom_inset,
    );
    let background_rect = Rect::from_min_max(
        vec2(
            text_pos.x - PREVIEW_CAPTION_PADDING_X,
            text_pos.y - PREVIEW_CAPTION_PADDING_Y,
        ),
        vec2(
            text_pos.x + text_size.x + PREVIEW_CAPTION_PADDING_X,
            text_pos.y + text_size.y + PREVIEW_CAPTION_PADDING_Y,
        ),
    );
    Some(CaptionLayout {
        paragraph,
        spans,
        rendered,
        text_pos,
        text_size,
        background_rect,
        font,
        wrap_width,
        elapsed_millis,
        vertical,
    })
}

fn draw_caption(
    painter: &TimelinePainter,
    item: &CaptionItem,
    layout: CaptionLayout,
    split: Option<(CaptionSplit, Color)>,
) {
    if item.background_color.a > 0 {
        painter.rect_filled(layout.background_rect, 0, item.background_color.into());
    }

    draw_caption_edge(
        painter,
        item,
        layout.text_pos,
        &layout.spans,
        layout.font,
        layout.wrap_width,
    );
    layout.paragraph.paint(
        painter.canvas(),
        Point::new(layout.text_pos.x, layout.text_pos.y),
    );
    if let Some((split, color)) = split {
        painter.rect_filled(split.caret, 0, color);
    }
}

fn split_for_layout(
    item: &CaptionItem,
    layout: &CaptionLayout,
    point: Vec2,
) -> Option<CaptionSplit> {
    let hit = layout
        .paragraph
        .get_glyph_position_at_coordinate(Point::new(
            point.x - layout.text_pos.x,
            point.y - layout.text_pos.y,
        ));
    let rendered = &layout.rendered;
    let mut rendered_byte = usize::try_from(hit.position).ok()?.min(rendered.len());
    while !rendered.is_char_boundary(rendered_byte) {
        rendered_byte -= 1;
    }
    let visible_byte = if layout.vertical {
        rendered[..rendered_byte]
            .chars()
            .filter(|character| *character != '\n')
            .map(char::len_utf8)
            .sum()
    } else {
        rendered_byte
    };
    let text_byte = shrimply_project::caption::markup::plain_text_byte_at_visible_byte(
        &item.text,
        layout.elapsed_millis,
        visible_byte,
    )?;
    shrimply_project::caption::markup::split_at_plain_text_byte(&item.text, text_byte)?;

    let (range, trailing) = if rendered_byte < rendered.len() {
        let next = rendered[rendered_byte..].chars().next()?.len_utf8();
        (rendered_byte..rendered_byte + next, false)
    } else {
        let previous = rendered[..rendered_byte].char_indices().next_back()?.0;
        (previous..rendered_byte, true)
    };
    let text_box = layout
        .paragraph
        .get_rects_for_range(range, RectHeightStyle::Tight, RectWidthStyle::Tight)
        .into_iter()
        .next()?;
    let x = layout.text_pos.x
        + if trailing {
            text_box.rect.right()
        } else {
            text_box.rect.left()
        };
    let top = layout.text_pos.y + text_box.rect.top() - CAPTION_SPLIT_CARET_PADDING;
    let bottom = layout.text_pos.y + text_box.rect.bottom() + CAPTION_SPLIT_CARET_PADDING;
    Some(CaptionSplit {
        text_byte,
        caret: Rect::from_xywh(
            x - CAPTION_SPLIT_CARET_WIDTH / 2.0,
            top,
            CAPTION_SPLIT_CARET_WIDTH,
            bottom - top,
        ),
    })
}

fn draw_caption_edge(
    painter: &TimelinePainter,
    item: &CaptionItem,
    position: Vec2,
    spans: &[Span],
    font: CaptionFontId,
    width: f32,
) {
    use shrimply_project::project::CaptionEdgeStyle;
    let color = item.edge_color.into();
    let offsets: &[Vec2] = match item.edge_style {
        CaptionEdgeStyle::None => &[],
        CaptionEdgeStyle::HardShadow => &[Vec2::splat(2.0)],
        CaptionEdgeStyle::Bevel => &[Vec2::splat(-1.0), Vec2::ONE],
        CaptionEdgeStyle::Glow => &[Vec2::NEG_X, Vec2::X, Vec2::NEG_Y, Vec2::Y],
        CaptionEdgeStyle::SoftShadow => &[Vec2::ONE, Vec2::splat(2.0)],
    };
    for &offset in offsets {
        caption_paragraph(spans, font.clone(), color, width, item.h_align).paint(
            painter.canvas(),
            Point::new(position.x + offset.x, position.y + offset.y),
        );
    }
}

fn caption_position(
    item: &CaptionItem,
    preview_rect: Rect,
    margin: f32,
    size: shrimply_skia_adw_core::canvas::Vec2,
    automatic_bottom: bool,
    bottom_inset: f32,
) -> Vec2 {
    let anchor_x = preview_rect.left() + preview_rect.width() * f32::from(item.position_x) / 100.0;
    let x = match item.h_align {
        HorizontalAlign::Left => anchor_x,
        HorizontalAlign::Center => anchor_x - size.x / 2.0,
        HorizontalAlign::Right => anchor_x - size.x,
    };
    // Default bottom captions use fixed preview-space padding. Percentage positioning belongs to
    // explicit layouts only, so resizing the preview does not change the default bottom gap.
    let y = if automatic_bottom {
        preview_rect.bottom()
            - PREVIEW_CAPTION_BOTTOM_PADDING
            - PREVIEW_CAPTION_PADDING_Y
            - size.y
            - bottom_inset.max(0.0)
    } else {
        let anchor_y =
            preview_rect.top() + preview_rect.height() * f32::from(item.position_y) / 100.0;
        match item.v_align {
            VerticalAlign::Top => anchor_y,
            VerticalAlign::Middle => anchor_y - size.y / 2.0,
            VerticalAlign::Bottom => anchor_y - size.y,
        }
    };

    let min_x = preview_rect.left() + margin;
    let max_x = (preview_rect.right() - margin - size.x).max(min_x);
    let min_y = preview_rect.top() + margin;
    let max_y = (preview_rect.bottom() - margin - bottom_inset.max(0.0) - size.y).max(min_y);
    vec2(x.clamp(min_x, max_x), y.clamp(min_y, max_y))
}
