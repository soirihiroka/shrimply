use super::{
    CaptionEdgeStyle, CaptionItem, CaptionWritingDirection, HorizontalAlign, VerticalAlign, markup,
};
use crate::project::{Color, Project};
use std::fmt::Write;
use std::fs;
use std::path::{Path, PathBuf};

const YOUTUBE_MAX_OPACITY: u8 = 254;
const YOUTUBE_VIEWPORT_INSET_PERCENT: f32 = 2.0;
const YOUTUBE_VIEWPORT_SCALE: f32 = 0.96;
const DEFAULT_FONT_SCALE: i32 = 100;
const FONT_SCALE_MULTIPLIER: i32 = 4;
const ZERO_WIDTH_SPACE: &str = "&#8203;";

#[repr(u8)]
#[derive(Clone, Copy)]
enum RubyStyle {
    Base = 1,
    Parenthesis = 2,
    Above = 4,
}

#[repr(u8)]
enum Anchor {
    TopLeft,
    TopCenter,
    TopRight,
    MiddleLeft,
    Center,
    MiddleRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}
#[repr(u8)]
enum Justification {
    Left,
    Right,
    Center,
}
#[repr(u8)]
enum PrintDirection {
    LeftToRight,
    Vertical = 2,
    Rotated,
}
#[repr(u8)]
enum ScrollDirection {
    Default,
    Reverse,
}
#[repr(u8)]
enum YttFont {
    Roboto,
    MonospaceSerif,
    Serif,
    MonospaceSans,
    Casual = 5,
    Cursive,
    SmallCapitals,
}

#[derive(Clone, Copy)]
pub enum ExportMode {
    Merge,
    Separate,
}

pub fn export(project: &Project, path: &Path, mode: ExportMode) -> Result<Vec<PathBuf>, String> {
    let tracks = project.caption_tracks.iter().filter(|track| track.enabled);
    let outputs: Vec<(PathBuf, Vec<&CaptionItem>)> = match mode {
        ExportMode::Merge => vec![(
            path.to_path_buf(),
            tracks.flat_map(|track| &track.items).collect(),
        )],
        ExportMode::Separate => tracks
            .enumerate()
            .map(|(index, track)| (track_path(path, index + 1), track.items.iter().collect()))
            .collect(),
    };
    for (path, cues) in &outputs {
        fs::write(path, document(cues))
            .map_err(|error| format!("Could not save {}: {error}", path.display()))?;
    }
    Ok(outputs.into_iter().map(|(path, _)| path).collect())
}

fn track_path(path: &Path, number: usize) -> PathBuf {
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("captions");
    path.with_file_name(format!("{stem}-track-{number}.ytt"))
}

struct Cue<'a> {
    item: &'a CaptionItem,
    wp: usize,
    ws: usize,
    parts: Vec<Part>,
}
struct Part {
    pen: usize,
    text: String,
    start_millis: u32,
}

pub fn document(items: &[&CaptionItem]) -> String {
    let mut items = items
        .iter()
        .copied()
        .filter(|item| !markup::plain_text(&item.text).trim().is_empty())
        .collect::<Vec<_>>();
    items.sort_by_key(|item| (item.start, item.end));
    let mut pens = vec![String::from("<pen id=\"0\"/>")];
    let mut cues = Vec::new();
    for (index, item) in items.into_iter().enumerate() {
        let mut parts = Vec::new();
        for span in markup::parse(&item.text) {
            if let Some(ruby) = span.ruby {
                for (text, ruby_style) in [
                    (ruby.base, RubyStyle::Base),
                    ("(".into(), RubyStyle::Parenthesis),
                    (ruby.annotation, RubyStyle::Above),
                    (")".into(), RubyStyle::Parenthesis),
                ] {
                    let pen = pens.len();
                    pens.push(pen_xml(
                        pen,
                        item,
                        span.bold,
                        span.italic,
                        span.underline,
                        Some(ruby_style as u8),
                    ));
                    parts.push(Part {
                        pen,
                        text,
                        start_millis: span.start_millis,
                    });
                }
            } else {
                let pen = pens.len();
                pens.push(pen_xml(
                    pen,
                    item,
                    span.bold,
                    span.italic,
                    span.underline,
                    None,
                ));
                parts.push(Part {
                    pen,
                    text: span.text,
                    start_millis: span.start_millis,
                });
            }
        }
        cues.push(Cue {
            item,
            wp: if item.layout_enabled { index + 1 } else { 0 },
            ws: index + 1,
            parts,
        });
    }

    let mut output = String::from(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<timedtext format=\"3\">\n<head>\n<wp id=\"0\" ap=\"7\" ah=\"50\" av=\"90\"/>\n",
    );
    for cue in cues.iter().filter(|cue| cue.item.layout_enabled) {
        writeln!(output, "{}", position_xml(cue.wp, cue.item)).unwrap();
    }
    output.push_str("<ws id=\"0\" ju=\"2\" pd=\"0\" sd=\"0\" wfo=\"0\"/>\n");
    for cue in &cues {
        writeln!(output, "{}", window_style_xml(cue.ws, cue.item)).unwrap();
    }
    for pen in pens {
        writeln!(output, "{pen}").unwrap();
    }
    output.push_str("</head>\n<body>\n");
    for cue in cues {
        write_cue(&mut output, cue);
    }
    output.push_str("</body>\n</timedtext>\n");
    output
}

fn write_cue(output: &mut String, cue: Cue<'_>) {
    let start = cue.item.start.as_nanos_i128().max(1_000_000) / 1_000_000;
    let end = cue
        .item
        .end
        .as_nanos_i128()
        .max(start * 1_000_000 + 1_000_000)
        / 1_000_000;
    let duration = (end - start).max(1);
    write!(
        output,
        "<p t=\"{start}\" d=\"{duration}\" wp=\"{}\" ws=\"{}\">",
        cue.wp, cue.ws
    )
    .unwrap();
    for (index, part) in cue.parts.into_iter().enumerate() {
        write!(output, "<s p=\"{}\"", part.pen).unwrap();
        if part.start_millis > 0 {
            write!(output, " t=\"{}\"", part.start_millis).unwrap();
        }
        write!(output, ">{}</s>", escape(&part.text)).unwrap();
        if index == 0 {
            output.push_str(ZERO_WIDTH_SPACE);
        }
    }
    output.push_str("</p>\n");
}

fn position_xml(id: usize, item: &CaptionItem) -> String {
    format!(
        "<wp id=\"{id}\" ap=\"{}\" ah=\"{}\" av=\"{}\"/>",
        anchor(item),
        youtube_coordinate(item.position_x),
        youtube_coordinate(item.position_y)
    )
}

fn window_style_xml(id: usize, item: &CaptionItem) -> String {
    let justify = match if item.layout_enabled {
        item.h_align
    } else {
        HorizontalAlign::Center
    } {
        HorizontalAlign::Left => Justification::Left,
        HorizontalAlign::Right => Justification::Right,
        HorizontalAlign::Center => Justification::Center,
    };
    let (pd, sd) = match item.writing_direction {
        CaptionWritingDirection::Horizontal => {
            (PrintDirection::LeftToRight, ScrollDirection::Default)
        }
        CaptionWritingDirection::VerticalRightToLeft => {
            (PrintDirection::Vertical, ScrollDirection::Default)
        }
        CaptionWritingDirection::VerticalLeftToRight => {
            (PrintDirection::Vertical, ScrollDirection::Reverse)
        }
        CaptionWritingDirection::RotatedLeftToRight => {
            (PrintDirection::Rotated, ScrollDirection::Default)
        }
        CaptionWritingDirection::RotatedRightToLeft => {
            (PrintDirection::Rotated, ScrollDirection::Reverse)
        }
    };
    format!(
        "<ws id=\"{id}\" ju=\"{}\" pd=\"{}\" sd=\"{}\" wfo=\"0\"/>",
        justify as u8, pd as u8, sd as u8
    )
}

fn pen_xml(
    id: usize,
    item: &CaptionItem,
    bold: bool,
    italic: bool,
    underline: bool,
    ruby: Option<u8>,
) -> String {
    let mut value = format!("<pen id=\"{id}\"");
    if item.styling_enabled {
        write!(
            value,
            " sz=\"{}\" fs=\"{}\" fc=\"{}\" fo=\"{}\" bc=\"{}\" bo=\"{}\"",
            youtube_font_scale(item.font_scale),
            font_id(item.font),
            color(item.text_color),
            opacity(item.text_color),
            color(item.background_color),
            opacity(item.background_color)
        )
        .unwrap();
    }
    if bold {
        value.push_str(" b=\"1\"");
    }
    if italic {
        value.push_str(" i=\"1\"");
    }
    if underline {
        value.push_str(" u=\"1\"");
    }
    if item.styling_enabled && item.edge_style != CaptionEdgeStyle::None {
        write!(
            value,
            " ec=\"{}\" et=\"{}\"",
            color(item.edge_color),
            item.edge_style as u8
        )
        .unwrap();
    }
    if let Some(ruby) = ruby {
        write!(value, " rb=\"{ruby}\"").unwrap();
    }
    value.push_str("/>");
    value
}

fn anchor(item: &CaptionItem) -> u8 {
    (match (item.v_align, item.h_align) {
        (VerticalAlign::Top, HorizontalAlign::Left) => Anchor::TopLeft,
        (VerticalAlign::Top, HorizontalAlign::Center) => Anchor::TopCenter,
        (VerticalAlign::Top, HorizontalAlign::Right) => Anchor::TopRight,
        (VerticalAlign::Middle, HorizontalAlign::Left) => Anchor::MiddleLeft,
        (VerticalAlign::Middle, HorizontalAlign::Center) => Anchor::Center,
        (VerticalAlign::Middle, HorizontalAlign::Right) => Anchor::MiddleRight,
        (VerticalAlign::Bottom, HorizontalAlign::Left) => Anchor::BottomLeft,
        (VerticalAlign::Bottom, HorizontalAlign::Center) => Anchor::BottomCenter,
        (VerticalAlign::Bottom, HorizontalAlign::Right) => Anchor::BottomRight,
    }) as u8
}
fn youtube_coordinate(value: u8) -> u8 {
    (((f32::from(value) - YOUTUBE_VIEWPORT_INSET_PERCENT) / YOUTUBE_VIEWPORT_SCALE).round() as i32)
        .clamp(0, 100) as u8
}
fn youtube_font_scale(value: u16) -> u16 {
    (DEFAULT_FONT_SCALE + (i32::from(value) - DEFAULT_FONT_SCALE) * FONT_SCALE_MULTIPLIER)
        .clamp(0, u16::MAX as i32) as u16
}
fn opacity(color: Color<u8>) -> u8 {
    color.a.min(YOUTUBE_MAX_OPACITY)
}
fn font_id(font: super::CaptionFont) -> u8 {
    (match font {
        super::CaptionFont::Roboto => YttFont::Roboto,
        super::CaptionFont::MonospaceSerif => YttFont::MonospaceSerif,
        super::CaptionFont::Serif => YttFont::Serif,
        super::CaptionFont::MonospaceSans => YttFont::MonospaceSans,
        super::CaptionFont::Casual => YttFont::Casual,
        super::CaptionFont::Cursive => YttFont::Cursive,
        super::CaptionFont::SmallCapitals => YttFont::SmallCapitals,
    }) as u8
}
fn color(color: Color<u8>) -> String {
    let (r, g, b) = if color.r == 255 && color.g == 255 && color.b == 255 {
        (254, 254, 254)
    } else {
        (color.r, color.g, color.b)
    };
    format!("#{r:02X}{g:02X}{b:02X}")
}
fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{CaptionEdgeStyle, Color, Time};

    #[test]
    fn writes_youtube_styles_and_spans() {
        let mut item = CaptionItem::new(
            Time::from_nanos(0),
            Time::from_nanos(4_000_000_000),
            "**Bold** *italic* __under__ {1200}[漢/かん]字".into(),
        );
        item.styling_enabled = true;
        item.text_color = Color::<u8>::from_rgb(255, 120, 40);
        item.edge_style = CaptionEdgeStyle::Glow;
        let xml = document(&[&item]);
        assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"utf-8\"?>"));
        assert!(xml.contains("fc=\"#FF7828\" fo=\"254\""));
        assert!(xml.contains("et=\"3\""));
        assert!(xml.contains(" b=\"1\""));
        assert!(xml.contains(" i=\"1\""));
        assert!(xml.contains(" u=\"1\""));
        assert!(xml.contains(" t=\"1200\""));
        assert!(xml.contains(" rb=\"1\""));
        assert!(xml.contains(" rb=\"4\""));
        assert!(xml.ends_with("</timedtext>\n"));
    }
}
