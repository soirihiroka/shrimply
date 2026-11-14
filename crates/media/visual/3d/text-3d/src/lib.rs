use glam::Vec3;
use hashbrown::HashSet;
use serde::{Deserialize, Deserializer, Serialize};
use shrimply_property_model::{
    DEFAULT_TEXT_FONT_FAMILY, FontFamily, FontVariation, TextDirection, TextFontStyle,
    TextHorizontalAlign, VerticalAlign,
    modifier_model::{
        KeyframeSpan, ModifierModel, combine, ensure_timeline_value_ids, timeline_value_span,
    },
    timeline_value::TimelineValue,
};
use shrimply_scene_3d::{PbrMaterial, Transform3d};
use uuid::Uuid;

#[cfg(feature = "mesh")]
mod mesh;
#[cfg(feature = "mesh")]
pub use mesh::{Geometry, Text3dError, generate_mesh};
#[cfg(feature = "mesh")]
pub use shrimply_fonts::native::register_application_font;

pub const MIN_SMOOTHNESS: f32 = 1.0;
pub const MAX_SMOOTHNESS: f32 = 12.0;
pub const DEFAULT_SMOOTHNESS: f32 = 4.0;
pub const DEFAULT_FONT_SIZE: f32 = 1.0;
pub const DEFAULT_DEPTH: f32 = 0.2;
pub const DEFAULT_ROUNDNESS: f32 = 0.03;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Text3dModifier {
    pub text: TimelineValue<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub font_families: Vec<FontFamily>,
    #[serde(default)]
    pub font_style: TextFontStyle,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub font_variations: Vec<FontVariation>,
    pub font_weight: TimelineValue<f32>,
    pub h_align: TextHorizontalAlign,
    pub v_align: VerticalAlign,
    pub direction: TextDirection,
    pub font_size: TimelineValue<f32>,
    pub transform: Transform3d,
    pub depth: TimelineValue<f32>,
    pub roundness: TimelineValue<f32>,
    #[serde(
        default = "default_smoothness",
        deserialize_with = "deserialize_smoothness"
    )]
    pub smoothness: TimelineValue<f32>,
    pub material: PbrMaterial,
}

impl Default for Text3dModifier {
    fn default() -> Self {
        Self {
            text: TimelineValue::new_const("Text".to_string()),
            font_families: vec![FontFamily::GoogleFonts {
                name: DEFAULT_TEXT_FONT_FAMILY.to_string(),
            }],
            font_style: TextFontStyle::Normal,
            font_variations: Vec::new(),
            font_weight: TimelineValue::new_const(400.0),
            h_align: TextHorizontalAlign::Center,
            v_align: VerticalAlign::Middle,
            direction: TextDirection::Horizontal,
            font_size: TimelineValue::new_const(DEFAULT_FONT_SIZE),
            transform: Transform3d::default(),
            depth: TimelineValue::new_const(DEFAULT_DEPTH),
            roundness: TimelineValue::new_const(DEFAULT_ROUNDNESS),
            smoothness: default_smoothness(),
            material: PbrMaterial::default(),
        }
    }
}

impl ModifierModel for Text3dModifier {
    fn display_name(&self) -> &'static str {
        "3D Text"
    }

    fn keywords(&self) -> &'static [&'static str] {
        &["extruded text", "type", "typography"]
    }

    fn ensure_ids(&mut self, seen: &mut HashSet<Uuid>) {
        ensure_timeline_value_ids(&mut self.text, seen);
        for value in [
            &mut self.font_weight,
            &mut self.font_size,
            &mut self.depth,
            &mut self.roundness,
            &mut self.smoothness,
        ] {
            ensure_timeline_value_ids(value, seen);
        }
        for value in [
            &mut self.transform.position,
            &mut self.transform.anchor,
            &mut self.transform.rotation_degrees,
            &mut self.transform.scale,
        ] {
            ensure_timeline_value_ids(value, seen);
        }
        ensure_timeline_value_ids(&mut self.transform.rotation_order, seen);
        for value in shrimply_scene_3d::material_numbers_mut(&mut self.material) {
            ensure_timeline_value_ids(value, seen);
        }
        for value in shrimply_scene_3d::material_colors_mut(&mut self.material) {
            ensure_timeline_value_ids(value, seen);
        }
    }

    fn keyframe_span(&self) -> KeyframeSpan {
        combine(
            [
                timeline_value_span(&self.text),
                timeline_value_span(&self.font_weight),
                timeline_value_span(&self.font_size),
                timeline_value_span(&self.depth),
                timeline_value_span(&self.roundness),
                timeline_value_span(&self.smoothness),
                timeline_value_span(&self.transform.position),
                timeline_value_span(&self.transform.anchor),
                timeline_value_span(&self.transform.rotation_degrees),
                timeline_value_span(&self.transform.rotation_order),
                timeline_value_span(&self.transform.scale),
            ]
            .into_iter()
            .chain(
                shrimply_scene_3d::material_numbers(&self.material)
                    .into_iter()
                    .map(timeline_value_span),
            )
            .chain(
                shrimply_scene_3d::material_colors(&self.material)
                    .into_iter()
                    .map(timeline_value_span),
            ),
        )
    }

    fn number(&self, id: Uuid) -> Option<&TimelineValue<f32>> {
        [
            &self.font_weight,
            &self.font_size,
            &self.depth,
            &self.roundness,
            &self.smoothness,
        ]
        .into_iter()
        .chain(shrimply_scene_3d::material_numbers(&self.material))
        .find(|value| value.id == id)
    }

    fn number_mut(&mut self, id: Uuid) -> Option<&mut TimelineValue<f32>> {
        [
            &mut self.font_weight,
            &mut self.font_size,
            &mut self.depth,
            &mut self.roundness,
            &mut self.smoothness,
        ]
        .into_iter()
        .chain(shrimply_scene_3d::material_numbers_mut(&mut self.material))
        .find(|value| value.id == id)
    }

    fn number3(&self, id: Uuid) -> Option<&TimelineValue<Vec3>> {
        [
            &self.transform.position,
            &self.transform.anchor,
            &self.transform.rotation_degrees,
            &self.transform.scale,
        ]
        .into_iter()
        .find(|value| value.id == id)
    }

    fn number3_mut(&mut self, id: Uuid) -> Option<&mut TimelineValue<Vec3>> {
        [
            &mut self.transform.position,
            &mut self.transform.anchor,
            &mut self.transform.rotation_degrees,
            &mut self.transform.scale,
        ]
        .into_iter()
        .find(|value| value.id == id)
    }

    fn color_mut(
        &mut self,
        id: Uuid,
    ) -> Option<&mut TimelineValue<shrimply_property_model::Color<u8>>> {
        shrimply_scene_3d::material_colors_mut(&mut self.material)
            .into_iter()
            .find(|value| value.id == id)
    }

    fn text(&self, id: Uuid) -> Option<&TimelineValue<String>> {
        (self.text.id == id).then_some(&self.text)
    }

    fn text_mut(&mut self, id: Uuid) -> Option<&mut TimelineValue<String>> {
        (self.text.id == id).then_some(&mut self.text)
    }
}

fn default_smoothness() -> TimelineValue<f32> {
    TimelineValue::new_const(DEFAULT_SMOOTHNESS)
}

fn deserialize_smoothness<'de, D>(deserializer: D) -> Result<TimelineValue<f32>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Value {
        Timeline(TimelineValue<f32>),
        Legacy(f32),
    }
    Ok(match Value::deserialize(deserializer)? {
        Value::Timeline(value) => value,
        Value::Legacy(value) => TimelineValue::new_const(value),
    })
}
