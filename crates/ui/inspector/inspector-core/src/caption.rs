use shrimply_project_document::project::{
    CaptionEdgeStyle, CaptionFont, CaptionItem, CaptionWritingDirection, Color, HorizontalAlign,
    ItemAddress, Time, VerticalAlign,
};

use crate::{
    ControlKind, InspectorCommit, InspectorControl, InspectorController, InspectorTarget,
    model::{EditKind, parsed_value, target_value},
};

#[derive(Clone, Copy)]
pub struct CaptionChoice<T> {
    pub value: T,
    pub key: &'static str,
    pub label: &'static str,
    pub icon: Option<CaptionChoiceIcon>,
}

#[derive(Clone, Copy)]
pub struct CaptionChoiceIcon {
    pub gtk: &'static str,
    pub qt: &'static str,
}

#[derive(Clone, Copy)]
pub struct CaptionNumberPresentation {
    pub label: &'static str,
    pub minimum: u16,
    pub maximum: u16,
    pub drag_step: f64,
    pub digits: usize,
    pub unit: &'static str,
}

#[derive(Clone)]
pub struct CaptionText {
    pub text: String,
    pub writing_direction: CaptionWritingDirection,
}

impl Default for CaptionText {
    fn default() -> Self {
        let caption = default_caption();
        Self {
            text: caption.text,
            writing_direction: caption.writing_direction,
        }
    }
}

impl From<&CaptionItem> for CaptionText {
    fn from(caption: &CaptionItem) -> Self {
        Self {
            text: caption.text.clone(),
            writing_direction: caption.writing_direction,
        }
    }
}

#[derive(Clone, Copy)]
pub struct CaptionLayout {
    pub enabled: bool,
    pub horizontal_align: HorizontalAlign,
    pub vertical_align: VerticalAlign,
    pub position_x: u8,
    pub position_y: u8,
}

impl Default for CaptionLayout {
    fn default() -> Self {
        Self::from(&default_caption())
    }
}

impl From<&CaptionItem> for CaptionLayout {
    fn from(caption: &CaptionItem) -> Self {
        Self {
            enabled: caption.layout_enabled,
            horizontal_align: caption.h_align,
            vertical_align: caption.v_align,
            position_x: caption.position_x,
            position_y: caption.position_y,
        }
    }
}

impl CaptionLayout {
    pub fn apply(self, caption: &mut CaptionItem) {
        caption.layout_enabled = self.enabled;
        caption.h_align = self.horizontal_align;
        caption.v_align = self.vertical_align;
        caption.position_x = self.position_x;
        caption.position_y = self.position_y;
    }
}

#[derive(Clone, Copy)]
pub struct CaptionAppearance {
    pub enabled: bool,
    pub font_scale: u16,
    pub font: CaptionFont,
    pub edge_style: CaptionEdgeStyle,
    pub text_color: Color<u8>,
    pub background_color: Color<u8>,
    pub edge_color: Color<u8>,
}

impl Default for CaptionAppearance {
    fn default() -> Self {
        Self::from(&default_caption())
    }
}

impl From<&CaptionItem> for CaptionAppearance {
    fn from(caption: &CaptionItem) -> Self {
        Self {
            enabled: caption.styling_enabled,
            font_scale: caption.font_scale,
            font: caption.font,
            edge_style: caption.edge_style,
            text_color: caption.text_color,
            background_color: caption.background_color,
            edge_color: caption.edge_color,
        }
    }
}

impl CaptionAppearance {
    pub fn apply(self, caption: &mut CaptionItem) {
        caption.styling_enabled = self.enabled;
        caption.font_scale = self.font_scale;
        caption.font = self.font;
        caption.edge_style = self.edge_style;
        caption.text_color = self.text_color;
        caption.background_color = self.background_color;
        caption.edge_color = self.edge_color;
    }
}

pub fn default_caption() -> CaptionItem {
    CaptionItem::new(Time::ZERO, Time::ZERO, String::new())
}

pub fn choice<T: Copy + Eq>(choices: &[CaptionChoice<T>], value: T) -> &CaptionChoice<T> {
    choices
        .iter()
        .find(|choice| choice.value == value)
        .expect("caption value must have a declared choice")
}

pub const POSITION_X: CaptionNumberPresentation = CaptionNumberPresentation {
    label: "Position X",
    minimum: 0,
    maximum: 100,
    drag_step: 1.0,
    digits: 0,
    unit: "%",
};

pub const POSITION_Y: CaptionNumberPresentation = CaptionNumberPresentation {
    label: "Position Y",
    ..POSITION_X
};

pub const FONT_SCALE: CaptionNumberPresentation = CaptionNumberPresentation {
    label: "Font size",
    minimum: 75,
    maximum: 300,
    drag_step: 1.0,
    digits: 0,
    unit: "%",
};

pub const HORIZONTAL_ALIGNMENTS: &[CaptionChoice<HorizontalAlign>] = &[
    CaptionChoice {
        value: HorizontalAlign::Left,
        key: "left",
        label: "Left",
        icon: Some(CaptionChoiceIcon {
            gtk: "text-justify-left-symbolic",
            qt: "format-justify-left",
        }),
    },
    CaptionChoice {
        value: HorizontalAlign::Center,
        key: "center",
        label: "Center",
        icon: Some(CaptionChoiceIcon {
            gtk: "text-justify-center-symbolic",
            qt: "format-justify-center",
        }),
    },
    CaptionChoice {
        value: HorizontalAlign::Right,
        key: "right",
        label: "Right",
        icon: Some(CaptionChoiceIcon {
            gtk: "text-justify-right-symbolic",
            qt: "format-justify-right",
        }),
    },
];

pub const VERTICAL_ALIGNMENTS: &[CaptionChoice<VerticalAlign>] = &[
    CaptionChoice {
        value: VerticalAlign::Top,
        key: "top",
        label: "Top",
        icon: Some(CaptionChoiceIcon {
            gtk: "valign-start-symbolic",
            qt: "align-vertical-top",
        }),
    },
    CaptionChoice {
        value: VerticalAlign::Middle,
        key: "middle",
        label: "Middle",
        icon: Some(CaptionChoiceIcon {
            gtk: "valign-center-symbolic",
            qt: "align-vertical-center",
        }),
    },
    CaptionChoice {
        value: VerticalAlign::Bottom,
        key: "bottom",
        label: "Bottom",
        icon: Some(CaptionChoiceIcon {
            gtk: "valign-end-symbolic",
            qt: "align-vertical-bottom",
        }),
    },
];

pub const FONTS: &[CaptionChoice<CaptionFont>] = &[
    CaptionChoice {
        value: CaptionFont::Roboto,
        key: "roboto",
        label: "Roboto",
        icon: None,
    },
    CaptionChoice {
        value: CaptionFont::MonospaceSerif,
        key: "monospace_serif",
        label: "Monospace Serif",
        icon: None,
    },
    CaptionChoice {
        value: CaptionFont::Serif,
        key: "serif",
        label: "Serif",
        icon: None,
    },
    CaptionChoice {
        value: CaptionFont::MonospaceSans,
        key: "monospace_sans",
        label: "Monospace Sans",
        icon: None,
    },
    CaptionChoice {
        value: CaptionFont::Casual,
        key: "casual",
        label: "Casual",
        icon: None,
    },
    CaptionChoice {
        value: CaptionFont::Cursive,
        key: "cursive",
        label: "Cursive",
        icon: None,
    },
    CaptionChoice {
        value: CaptionFont::SmallCapitals,
        key: "small_capitals",
        label: "Small Capitals",
        icon: None,
    },
];

pub const WRITING_DIRECTIONS: &[CaptionChoice<CaptionWritingDirection>] = &[
    CaptionChoice {
        value: CaptionWritingDirection::Horizontal,
        key: "horizontal",
        label: "Horizontal",
        icon: None,
    },
    CaptionChoice {
        value: CaptionWritingDirection::VerticalRightToLeft,
        key: "vertical_right_to_left",
        label: "Vertical RTL",
        icon: None,
    },
    CaptionChoice {
        value: CaptionWritingDirection::VerticalLeftToRight,
        key: "vertical_left_to_right",
        label: "Vertical LTR",
        icon: None,
    },
    CaptionChoice {
        value: CaptionWritingDirection::RotatedLeftToRight,
        key: "rotated_left_to_right",
        label: "Rotated LTR",
        icon: None,
    },
    CaptionChoice {
        value: CaptionWritingDirection::RotatedRightToLeft,
        key: "rotated_right_to_left",
        label: "Rotated RTL",
        icon: None,
    },
];

pub const EDGE_STYLES: &[CaptionChoice<CaptionEdgeStyle>] = &[
    CaptionChoice {
        value: CaptionEdgeStyle::None,
        key: "none",
        label: "None",
        icon: None,
    },
    CaptionChoice {
        value: CaptionEdgeStyle::HardShadow,
        key: "hard_shadow",
        label: "Hard shadow",
        icon: None,
    },
    CaptionChoice {
        value: CaptionEdgeStyle::Bevel,
        key: "bevel",
        label: "Bevel",
        icon: None,
    },
    CaptionChoice {
        value: CaptionEdgeStyle::Glow,
        key: "glow",
        label: "Glow / outline",
        icon: None,
    },
    CaptionChoice {
        value: CaptionEdgeStyle::SoftShadow,
        key: "soft_shadow",
        label: "Soft shadow",
        icon: None,
    },
];

pub const EDIT_COMMIT: &str = "caption";

impl InspectorController {
    pub(crate) fn set_caption_control_value(
        &self,
        target: &InspectorTarget,
        control: &InspectorControl,
        text: &str,
    ) -> Option<Result<(), String>> {
        if !matches!(target, InspectorTarget::Item(ItemAddress::Caption { .. }))
            || control.commit_name != EDIT_COMMIT
        {
            return None;
        }
        Some((|| {
            let replacement = {
                let project = self.project.borrow();
                let root = target_value(&project, target)
                    .ok_or("caption is no longer available")?
                    .1;
                let current = root
                    .pointer(&control.path)
                    .ok_or("caption field is no longer available")?;
                if control.kind == ControlKind::Number {
                    let value = text.parse::<f64>().map_err(|_| "invalid caption number")?;
                    if !value.is_finite() {
                        return Err("caption number must be finite".into());
                    }
                    serde_json::Value::from(control.store_number(value) as u16)
                } else {
                    parsed_value(current, text)?
                }
            };
            self.replace_value_with_commit(
                target,
                EditKind::Live,
                &control.path,
                replacement,
                None,
                if control.kind == ControlKind::MultilineText {
                    InspectorCommit::Deferred
                } else {
                    InspectorCommit::Coalesced(EDIT_COMMIT)
                },
            )
        })())
    }

    pub(crate) fn set_caption_control_components(
        &self,
        target: &InspectorTarget,
        control: &InspectorControl,
        values: &[f64],
    ) -> Option<Result<(), String>> {
        if !matches!(target, InspectorTarget::Item(ItemAddress::Caption { .. }))
            || control.kind != ControlKind::Color
            || control.commit_name != EDIT_COMMIT
        {
            return None;
        }
        Some((|| {
            let [red, green, blue, alpha] = values else {
                return Err("caption color needs four components".into());
            };
            if values
                .iter()
                .any(|value| !value.is_finite() || !(0.0..=f64::from(u8::MAX)).contains(value))
            {
                return Err("caption color components must be between 0 and 255".into());
            }
            self.replace_value_with_commit(
                target,
                EditKind::Live,
                &control.path,
                serde_json::to_value(Color::from_rgba(
                    *red as u8,
                    *green as u8,
                    *blue as u8,
                    *alpha as u8,
                ))
                .expect("caption color must serialize"),
                None,
                InspectorCommit::Coalesced(EDIT_COMMIT),
            )
        })())
    }

    pub(crate) fn commit_caption_control(
        &self,
        target: &InspectorTarget,
        control: &InspectorControl,
    ) -> Option<Result<(), String>> {
        if !matches!(target, InspectorTarget::Item(ItemAddress::Caption { .. }))
            || control.commit_name != EDIT_COMMIT
        {
            return None;
        }
        Some((|| {
            let project = self.project.borrow();
            target_value(&project, target)
                .and_then(|(_, root)| root.pointer(&control.path).cloned())
                .ok_or("caption field is no longer available")?;
            if control.kind == ControlKind::MultilineText {
                shrimply_project_document::project::commit_edit(&project, EDIT_COMMIT);
            }
            Ok(())
        })())
    }
}
