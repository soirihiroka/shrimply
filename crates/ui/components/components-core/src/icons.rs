use quick_xml::{
    Reader, XmlVersion,
    events::{BytesStart, Event},
};
use serde::Serialize;

include!(concat!(env!("OUT_DIR"), "/icons.rs"));

/// Look up a bundled SVG icon by name (with or without `.svg` suffix).
pub fn svg(name: &str) -> Option<&'static str> {
    let name = name.strip_suffix(".svg").unwrap_or(name);
    ALL.binary_search_by_key(&name, |(key, _)| *key)
        .ok()
        .map(|index| ALL[index].1)
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct VectorIcon {
    pub width: f32,
    pub height: f32,
    pub paths: Vec<VectorPath>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct VectorPath {
    #[serde(rename = "d")]
    pub data: String,
    #[serde(rename = "tx")]
    pub translate_x: f32,
    #[serde(rename = "ty")]
    pub translate_y: f32,
    #[serde(rename = "sx")]
    pub scale_x: f32,
    #[serde(rename = "sy")]
    pub scale_y: f32,
    pub fill_alpha: f32,
    #[serde(rename = "even_odd")]
    pub fill_even_odd: bool,
    pub stroke_alpha: f32,
    pub stroke_width: f32,
    pub stroke_cap: &'static str,
    pub stroke_join: &'static str,
}

#[derive(Clone, Copy)]
struct GroupStyle {
    translate_x: f32,
    translate_y: f32,
    scale_x: f32,
    scale_y: f32,
    opacity: f32,
    filled: bool,
    fill_opacity: f32,
    fill_even_odd: bool,
    stroked: bool,
    stroke_opacity: f32,
    stroke_width: f32,
    stroke_cap: &'static str,
    stroke_join: &'static str,
    hidden: bool,
}

impl Default for GroupStyle {
    fn default() -> Self {
        Self {
            translate_x: 0.0,
            translate_y: 0.0,
            scale_x: 1.0,
            scale_y: 1.0,
            opacity: 1.0,
            filled: true,
            fill_opacity: 1.0,
            fill_even_odd: false,
            stroked: false,
            stroke_opacity: 1.0,
            stroke_width: 1.0,
            stroke_cap: "butt",
            stroke_join: "miter",
            hidden: false,
        }
    }
}

/// Parse a bundled symbolic SVG into structured vector paths for native vector toolkits.
pub fn vector(name: &str) -> Option<VectorIcon> {
    let source = svg(name)?;
    let mut reader = Reader::from_str(source);
    let mut width = 16.0f32;
    let mut height = 16.0f32;
    let mut paths = Vec::new();
    let mut stack = vec![GroupStyle::default()];
    let mut skipped_depth = 0usize;

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref tag)) => {
                let tag_name = tag.local_name();
                let tag_name = tag_name.as_ref();
                if skipped_depth > 0
                    || matches!(
                        tag_name,
                        "defs" | "mask" | "clipPath" | "metadata" | "title" | "desc"
                    )
                {
                    skipped_depth += 1;
                    continue;
                }
                if tag_name == "svg" {
                    (width, height) = parse_viewport(tag, width, height);
                }
                let is_path = tag_name == "path";
                let (style, data) = parse_style(*stack.last().expect("style stack"), tag, is_path);
                if let Some(data) = data
                    && !style.hidden
                {
                    push_path(data, style, &mut paths);
                }
                stack.push(style);
            }
            Ok(Event::Empty(ref tag))
                if skipped_depth == 0 && tag.local_name().as_ref() == "path" =>
            {
                let (style, data) = parse_style(*stack.last().expect("style stack"), tag, true);
                if let Some(data) = data
                    && !style.hidden
                {
                    push_path(data, style, &mut paths);
                }
            }
            Ok(Event::End(_)) => {
                if skipped_depth > 0 {
                    skipped_depth -= 1;
                } else if stack.len() > 1 {
                    stack.pop();
                }
            }
            Ok(Event::Eof) => break,
            Err(error) => panic!("bundled SVG {name} failed to parse: {error}"),
            _ => {}
        }
    }

    Some(VectorIcon {
        width,
        height,
        paths,
    })
}

/// Serialize a parsed bundled icon as JSON for cross-language FFI (e.g. Android JNI).
pub fn vector_json(name: &str) -> Option<String> {
    Some(serde_json::to_string(&vector(name)?).expect("serialize vector icon"))
}

fn parse_viewport(tag: &BytesStart<'_>, default_w: f32, default_h: f32) -> (f32, f32) {
    let mut width = None;
    let mut height = None;
    let mut view_box = None;
    for attr in tag.attributes().flatten() {
        let Ok(value) = attr.normalized_value(XmlVersion::Implicit1_0) else {
            continue;
        };
        match attr.key.local_name().as_ref() {
            "viewBox" => {
                let mut nums = parse_numbers(&value);
                if let (Some(_), Some(_), Some(w), Some(h), None) = (
                    nums.next(),
                    nums.next(),
                    nums.next(),
                    nums.next(),
                    nums.next(),
                ) {
                    view_box = Some((w, h));
                }
            }
            "width" => width = parse_px(&value),
            "height" => height = parse_px(&value),
            _ => {}
        }
    }
    view_box.unwrap_or((width.unwrap_or(default_w), height.unwrap_or(default_h)))
}

fn parse_style(
    mut style: GroupStyle,
    tag: &BytesStart<'_>,
    capture_path: bool,
) -> (GroupStyle, Option<String>) {
    let mut path_data = None;
    for attr in tag.attributes().flatten() {
        let Ok(value) = attr.normalized_value(XmlVersion::Implicit1_0) else {
            continue;
        };
        let key = attr.key.local_name();
        match key.as_ref() {
            "d" if capture_path => path_data = Some(value.into_owned()),
            "transform" => {
                let value = value.trim();
                if let Some((tx, ty)) = parse_transform_pair(value, "translate(") {
                    style.translate_x += tx * style.scale_x;
                    style.translate_y += ty * style.scale_y;
                }
                if let Some((sx, sy)) = parse_transform_pair(value, "scale(") {
                    style.scale_x *= sx;
                    style.scale_y *= sy;
                }
            }
            "style" => {
                for decl in value.split(';') {
                    if let Some((prop, val)) = decl.split_once(':') {
                        apply_style_prop(&mut style, prop.trim(), val.trim());
                    }
                }
            }
            key => apply_style_prop(&mut style, key, value.trim()),
        }
    }
    (style, path_data)
}

fn apply_style_prop(style: &mut GroupStyle, prop: &str, val: &str) {
    match prop {
        "visibility" if val == "hidden" => style.hidden = true,
        "display" if val == "none" => style.hidden = true,
        "opacity" => {
            if let Ok(opacity) = val.parse::<f32>() {
                style.opacity *= opacity.clamp(0.0, 1.0);
            }
        }
        "fill" => style.filled = val != "none",
        "fill-opacity" => {
            if let Ok(opacity) = val.parse::<f32>() {
                style.fill_opacity = opacity.clamp(0.0, 1.0);
            }
        }
        "fill-rule" => style.fill_even_odd = val == "evenodd",
        "stroke" => style.stroked = val != "none",
        "stroke-opacity" => {
            if let Ok(opacity) = val.parse::<f32>() {
                style.stroke_opacity = opacity.clamp(0.0, 1.0);
            }
        }
        "stroke-width" => {
            if let Some(width) = parse_px(val) {
                style.stroke_width = width;
            }
        }
        "stroke-linecap" => {
            style.stroke_cap = match val {
                "round" => "round",
                "square" => "square",
                _ => "butt",
            };
        }
        "stroke-linejoin" => {
            style.stroke_join = match val {
                "round" => "round",
                "bevel" => "bevel",
                _ => "miter",
            };
        }
        _ => {}
    }
}

fn push_path(data: String, style: GroupStyle, out: &mut Vec<VectorPath>) {
    let fill_alpha = if style.filled {
        style.opacity * style.fill_opacity
    } else {
        0.0
    };
    let stroke_alpha = if style.stroked {
        style.opacity * style.stroke_opacity
    } else {
        0.0
    };
    if fill_alpha <= 0.0 && stroke_alpha <= 0.0 {
        return;
    }
    out.push(VectorPath {
        data,
        translate_x: style.translate_x,
        translate_y: style.translate_y,
        scale_x: style.scale_x,
        scale_y: style.scale_y,
        fill_alpha,
        fill_even_odd: style.fill_even_odd,
        stroke_alpha,
        stroke_width: style.stroke_width,
        stroke_cap: style.stroke_cap,
        stroke_join: style.stroke_join,
    });
}

fn parse_px(value: &str) -> Option<f32> {
    value.trim().trim_end_matches("px").parse::<f32>().ok()
}

fn parse_numbers(value: &str) -> impl Iterator<Item = f32> + '_ {
    value
        .split(|c: char| c.is_ascii_whitespace() || c == ',')
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse::<f32>().ok())
}

fn parse_transform_pair(value: &str, prefix: &str) -> Option<(f32, f32)> {
    let start = value.find(prefix)? + prefix.len();
    let end = value[start..].find(')')? + start;
    let mut nums = parse_numbers(&value[start..end]);
    match (nums.next(), nums.next(), nums.next()) {
        (Some(first), None, None) if prefix == "scale(" => Some((first, first)),
        (Some(first), None, None) => Some((first, 0.0)),
        (Some(first), Some(second), None) => Some((first, second)),
        _ => None,
    }
}
