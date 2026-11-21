//! Shared paint-tool presentation and state transitions for native preview rails.
use crate::{PaintPreviewMode, PaintPreviewState};
use shrimply_paint_model::PaintItem;
use shrimply_timeline_value::Time;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Pen,
    Fill,
    StrokeTransform,
    Eraser,
    Adjust,
    Size,
    Palette(Uuid),
    Previous,
    Next,
}

impl Action {
    pub fn id(self) -> String {
        match self {
            Self::Pen => "pen".into(),
            Self::Fill => "fill".into(),
            Self::StrokeTransform => "stroke_transform".into(),
            Self::Eraser => "eraser".into(),
            Self::Adjust => "adjust".into(),
            Self::Size => "size".into(),
            Self::Palette(id) => format!("palette:{id}"),
            Self::Previous => "previous".into(),
            Self::Next => "next".into(),
        }
    }

    pub fn parse(id: &str) -> Result<Self, String> {
        Ok(match id {
            "pen" => Self::Pen,
            "fill" => Self::Fill,
            "stroke_transform" => Self::StrokeTransform,
            "eraser" => Self::Eraser,
            "adjust" => Self::Adjust,
            "size" => Self::Size,
            "previous" => Self::Previous,
            "next" => Self::Next,
            _ => Self::Palette(
                id.strip_prefix("palette:")
                    .ok_or("Unknown paint tool")?
                    .parse()
                    .map_err(|_| "Invalid paint palette entry")?,
            ),
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Button {
    pub action: Action,
    pub label: String,
    pub icon: Option<&'static str>,
    pub text: Option<String>,
    pub color: Option<[f32; 4]>,
    pub active: bool,
    pub separator: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Size {
    pub label: String,
    pub value: f32,
    pub minimum: f32,
    pub maximum: f32,
    pub step: f32,
    pub page: f32,
    pub digits: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Presentation {
    pub buttons: Vec<Button>,
    pub size: Size,
}

impl Presentation {
    pub fn new(state: &PaintPreviewState, paint: &PaintItem, time: Time) -> Self {
        let mut buttons = [
            (
                Action::Pen,
                "Pen (B)",
                "document-edit-symbolic",
                state.mode == PaintPreviewMode::Pen,
            ),
            (
                Action::Fill,
                "Fill (F)",
                "fill-tool-symbolic",
                state.mode == PaintPreviewMode::Fill,
            ),
            (
                Action::Eraser,
                "Eraser (E)",
                "eraser-symbolic",
                state.eraser,
            ),
            (
                Action::Adjust,
                "Adjust points (hold Ctrl for temporary adjustment)",
                "function-exponential-symbolic",
                state.adjusting,
            ),
            (
                Action::StrokeTransform,
                "Stroke Transform (T)",
                "move-tool-symbolic",
                state.mode == PaintPreviewMode::StrokeTransform,
            ),
        ]
        .into_iter()
        .map(|(action, label, icon, active)| Button {
            action,
            label: shrimply_i18n::text(label).into_owned(),
            icon: Some(icon),
            text: None,
            color: None,
            active,
            separator: action == Action::Pen,
        })
        .collect::<Vec<_>>();
        let fill = state.mode == PaintPreviewMode::Fill;
        let size = Size {
            label: shrimply_i18n::text(if fill {
                "Fill gap tolerance"
            } else if state.eraser {
                "Eraser width scale"
            } else {
                "Pen width scale"
            })
            .into_owned(),
            value: if fill {
                state.fill_tolerance
            } else {
                state.brush_scale(state.eraser)
            },
            minimum: if fill { 1.0 } else { 0.1 },
            maximum: 99.0,
            step: if fill { 1.0 } else { 0.1 },
            page: if fill { 10.0 } else { 1.0 },
            digits: u32::from(!fill),
        };
        buttons.push(Button {
            action: Action::Size,
            label: size.label.clone(),
            icon: None,
            text: Some(if fill {
                format!("{:.0}", size.value)
            } else if size.value < 1.0 {
                format!(".{}x", (size.value * 10.0).round() as u8)
            } else {
                format!("{:.0}x", size.value)
            }),
            color: None,
            active: false,
            separator: false,
        });
        for (index, entry) in paint.palette.iter().enumerate() {
            buttons.push(Button {
                action: Action::Palette(entry.color.id),
                label: shrimply_i18n::text_args(
                    "Paint texture %{number}",
                    &[("number", (index + 1).to_string())],
                ),
                icon: None,
                text: None,
                color: Some(entry.color.value_at(time).to_srgba()),
                active: index
                    == state
                        .palette_index
                        .min(paint.palette.len().saturating_sub(1)),
                separator: index == 0,
            });
        }
        for (action, label, text, active) in [
            (
                Action::Previous,
                "Previous drawing onion skin",
                "−1",
                state.onion_previous,
            ),
            (
                Action::Next,
                "Next drawing onion skin",
                "+1",
                state.onion_next,
            ),
        ] {
            buttons.push(Button {
                action,
                label: shrimply_i18n::text(label).into_owned(),
                icon: None,
                text: Some(text.into()),
                color: None,
                active,
                separator: action == Action::Previous,
            });
        }
        Self { buttons, size }
    }
}

pub fn apply(
    state: &mut PaintPreviewState,
    paint: &PaintItem,
    action: Action,
    value: Option<f32>,
) -> Result<(), String> {
    match action {
        Action::Pen => state.set_mode(PaintPreviewMode::Pen),
        Action::Fill => state.set_mode(PaintPreviewMode::Fill),
        Action::StrokeTransform => state.set_mode(PaintPreviewMode::StrokeTransform),
        Action::Eraser => state.set_eraser(!state.eraser),
        Action::Adjust => {
            let adjusting = !state.adjusting;
            state.set_mode(PaintPreviewMode::Pen);
            state.set_eraser(false);
            state.adjusting = adjusting;
        }
        Action::Size => {
            let value = value
                .filter(|value| value.is_finite())
                .ok_or("Paint size requires a finite number")?;
            if state.mode == PaintPreviewMode::Fill {
                state.set_fill_tolerance(value.round())
            } else {
                state.set_brush_scale(state.eraser, crate::math::rounded_tool_size(value))
            }
        }
        Action::Palette(id) => {
            let index = paint
                .palette
                .iter()
                .position(|entry| entry.color.id == id)
                .ok_or("Paint palette changed; select a color again")?;
            state.select_palette(index, paint.palette.len());
        }
        Action::Previous => state.set_onion_skin(true, !state.onion_previous),
        Action::Next => state.set_onion_skin(false, !state.onion_next),
    }
    Ok(())
}

pub fn stepped_size(state: &PaintPreviewState, larger: bool) -> f32 {
    if state.mode == PaintPreviewMode::Fill {
        crate::step_fill_tolerance(state.fill_tolerance, larger)
    } else {
        crate::step_tool_size(state.brush_scale(state.eraser), larger)
    }
}
