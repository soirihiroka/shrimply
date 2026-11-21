//! Native color controls render this draft; only confirmation commits inspector components.
use serde_json::{Value, json};
use shrimply_i18n::text;
use shrimply_math_color::{Color, parse_css_color};

use super::{Hsva, PALETTE, color_hex, remember_color};
use crate::math::{ColorSelection, color_selection, color_selection_key};

pub const SELECTION_SIZE: i32 = 300;
pub const BAR_SIZE: i32 = 24;
pub const TRACK_SIZE: f64 = 16.0;
pub const THUMB_RADIUS: f64 = 10.0;
pub const CHECKER_SIZE: f64 = 8.0;
pub const CHECKER_DARK: f64 = 0.72;
pub const CHECKER_LIGHT: f64 = 0.9;

pub fn parse_color(value: &Value, with_alpha: bool) -> Option<Color<u8>> {
    let mut color = if let Some(value) = value.as_str() {
        parse_css_color(value).or_else(|| {
            serde_json::from_str::<Value>(value)
                .ok()
                .and_then(|value| parse_color(&value, with_alpha))
        })?
    } else if let Some(values) = value.as_array().filter(|values| values.len() == 4) {
        let mut channels = [0; 4];
        for (channel, value) in channels.iter_mut().zip(values) {
            *channel = value
                .as_u64()
                .and_then(|value| value.try_into().ok())
                .or_else(|| value.as_str()?.parse().ok())?;
        }
        Color::from(channels)
    } else {
        serde_json::from_value::<Color<u8>>(value.clone()).ok()?
    };
    if !with_alpha {
        color.a = u8::MAX;
    }
    Some(color)
}

pub fn hue_stops() -> [(f64, Color<u8>); 7] {
    std::array::from_fn(|index| {
        let position = index as f64 / 6.0;
        (position, Color::from_hsv(position as f32 * 360.0, 1.0, 1.0))
    })
}

pub fn plane_stops(hue: f32) -> [[Color<u8>; 2]; 2] {
    [
        [Color::from_hsv(hue, 1.0, 1.0), Color::WHITE],
        [Color::BLACK, Color::TRANSPARENT],
    ]
}

pub fn alpha_stops(color: Color<u8>) -> [Color<u8>; 2] {
    [
        Color::new(color.r, color.g, color.b, 0),
        Color::from_rgb(color.r, color.g, color.b),
    ]
}

fn argb(color: Color<u8>) -> u32 {
    u32::from_be_bytes([color.a, color.r, color.g, color.b])
}

pub fn request(request: &Value) -> Result<Value, String> {
    if request["action"] == "sample" {
        let dimension = |key| {
            request[key]
                .as_u64()
                .and_then(|value| u32::try_from(value).ok())
                .filter(|value| *value > 0)
                .ok_or("Invalid image dimensions")
        };
        let coordinate = |key| {
            request[key]
                .as_f64()
                .filter(|value| value.is_finite())
                .ok_or("Invalid sample position")
        };
        let size = [dimension("width")?, dimension("height")?];
        let delta = match request["key"].as_str() {
            None => [0, 0],
            Some("left") => [-1, 0],
            Some("right") => [1, 0],
            Some("up") => [0, -1],
            Some("down") => [0, 1],
            _ => return Err("Unknown sample key".into()),
        };
        let pixel =
            crate::math::image_sample_point(size, [coordinate("x")?, coordinate("y")?], delta);
        return Ok(
            json!({"pixel":pixel,"position":[(f64::from(pixel[0])+0.5)/f64::from(size[0]),(f64::from(pixel[1])+0.5)/f64::from(size[1])]}),
        );
    }
    let state = request.get("state").unwrap_or(request);
    let with_alpha = state["with_alpha"].as_bool().unwrap_or(true);
    let title = state["title"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| text("Select color").into_owned());
    let mut draft =
        if let Some(values) = state["hsva"].as_array().filter(|values| values.len() == 4) {
            let mut channels = [0.0; 4];
            for (channel, value) in channels.iter_mut().zip(values) {
                *channel = value
                    .as_f64()
                    .filter(|value| value.is_finite())
                    .ok_or("Invalid color state")? as f32;
            }
            if !(0.0..=360.0).contains(&channels[0])
                || channels[1..]
                    .iter()
                    .any(|value| !(0.0..=1.0).contains(value))
            {
                return Err("Invalid color state".into());
            }
            Hsva {
                hue: channels[0],
                saturation: channels[1],
                value: channels[2],
                alpha: channels[3],
            }
        } else {
            Hsva::from_color(parse_color(&state["color"], with_alpha).ok_or("Invalid color")?)
        };
    let mut recent = state["recent"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|value| parse_color(value, true))
        .take(super::RECENT_LIMIT)
        .collect::<Vec<_>>();
    let position = |field: &str| {
        request[field]
            .as_f64()
            .filter(|value| value.is_finite())
            .map(|value| value.clamp(0.0, 1.0) as f32)
            .ok_or_else(|| format!("Color selection requires {field}"))
    };
    let mut error = None;
    let confirmed = match request["action"].as_str().unwrap_or("open") {
        "open" => false,
        "plane" => {
            draft = color_selection(
                draft,
                ColorSelection::Plane,
                [position("x")?, position("y")?],
            );
            false
        }
        "hue" => {
            draft = color_selection(draft, ColorSelection::Hue, [position("position")?, 0.0]);
            false
        }
        "alpha" => {
            draft = color_selection(draft, ColorSelection::Alpha, [position("position")?, 0.0]);
            false
        }
        "key" => {
            let control = match request["target"].as_str() {
                Some("plane") => ColorSelection::Plane,
                Some("hue") => ColorSelection::Hue,
                Some("alpha") => ColorSelection::Alpha,
                _ => return Err("Invalid color control".into()),
            };
            draft = color_selection_key(
                draft,
                control,
                request["key"].as_str().ok_or("Color key is required")?,
            )
            .ok_or("Invalid color key")?;
            false
        }
        "argb" => {
            let value = u32::try_from(
                request["value"]
                    .as_u64()
                    .ok_or("Sampled color is required")?,
            )
            .map_err(|_| "Invalid sampled color")?;
            let [alpha, red, green, blue] = value.to_be_bytes();
            draft = draft.update_color(Color::new(red, green, blue, alpha));
            false
        }
        "hex" | "color" => {
            let value = if request["action"] == "hex" {
                &request["text"]
            } else {
                &request["color"]
            };
            if let Some(color) = parse_color(value, with_alpha) {
                draft = draft.update_color(color);
            } else {
                error = Some(text("Invalid color").into_owned());
            }
            false
        }
        "confirm" => true,
        _ => return Err("Unknown color selection action".into()),
    };
    if !with_alpha {
        draft.alpha = 1.0;
    }
    let color = draft.color();
    if confirmed {
        remember_color(&mut recent, color);
    }
    let swatch = |label: String, sample: Color<u8>| json!({"label":label,"hex":color_hex(sample,true),"argb":argb(sample),"selected":sample == color});
    let palette = PALETTE
        .iter()
        .enumerate()
        .map(|(index, (label, sample))| {
            let mut value = swatch(text(label).into_owned(), *sample);
            value["row"] = json!(index % 5);
            value["column"] = json!(index / 5);
            value
        })
        .collect::<Vec<_>>();
    let [vertical, horizontal] = plane_stops(draft.hue).map(|colors| colors.map(argb));
    let mut response = json!({
        "state":{"hsva":[draft.hue,draft.saturation,draft.value,draft.alpha],"with_alpha":with_alpha,"title":title,
            "recent":recent.iter().map(|color|color_hex(*color,true)).collect::<Vec<_>>()},
        "hex":color_hex(color,with_alpha),"rgba":color.to_array(),"argb":argb(color),
        "position":[draft.value,1.0-draft.saturation],"hue_position":draft.hue/360.0,"alpha_position":draft.alpha,
        "hue_stops":hue_stops().map(|(position,color)|json!([position,argb(color)])),
        "plane_stops":{"vertical":vertical,"horizontal":horizontal},
        "alpha_stops":alpha_stops(color).map(argb),
        "palette":palette,"recent":recent.iter().map(|color|swatch(color_hex(*color,true),*color)).collect::<Vec<_>>(),
        "labels":{"title":title,"hex":text("Hexadecimal color"),"plane":text("Saturation and value"),"hue":text("Hue"),
            "alpha":text("Alpha"),"recent":text("Recent"),"confirm":text("Confirm color"),"cancel":text("Cancel"),"pick":text("Pick a color from the screen"),"notification_title":text("Select color"),
            "notification_text":text("Pick a color from the screen"),"stop":text("Cancel"),
            "sample_error":text("Could not capture the screen."),"sampling":text("Pick a color from the screen"),
            "left":text("Left"),"right":text("Right"),"up":text("Move up"),"down":text("Move down")},
        "metrics":{"plane":SELECTION_SIZE,"bar":BAR_SIZE,"track":TRACK_SIZE,"thumb":THUMB_RADIUS,"checker":CHECKER_SIZE,"checker_dark":CHECKER_DARK,"checker_light":CHECKER_LIGHT},
        "confirmed":confirmed,"value":serde_json::to_string(&color.to_array()).expect("serialize color"),
        "values":color.to_array().map(|value|value.to_string())
    });
    if let Some(error) = error {
        response["error"] = json!(error);
    }
    Ok(response)
}
