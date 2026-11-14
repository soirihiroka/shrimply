use hashbrown::HashSet;

use serde::{Deserialize, Serialize};
use shrimply_property_model::timeline_value::{
    TimelineStepVariant, TimelineValue, deserialize_timeline_value,
};
use uuid::Uuid;

use super::{KeyframeSpan, ModifierModel};
#[cfg(feature = "preview")]
use shrimply_preview_provider_skia::{
    PointerEvent, PreviewBuilder, PreviewContext, PreviewEditSink, PreviewProvider,
    PreviewResponse, PreviewTarget, Rect,
};

#[cfg(feature = "preview")]
use super::preview;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaskMode {
    #[default]
    Alpha,
    Luminance,
}

shrimply_property_model::timeline_value::timeline_step_type!(
    MaskMode,
    MaskMode::Alpha,
    &[
        TimelineStepVariant {
            value: MaskMode::Alpha,
            key: "alpha",
            label: "Alpha",
            icon: None
        },
        TimelineStepVariant {
            value: MaskMode::Luminance,
            key: "luminance",
            label: "Luminance",
            icon: None
        },
    ]
);

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MaskModifier {
    #[serde(default)]
    pub item_id: Option<Uuid>,
    #[serde(default, deserialize_with = "deserialize_timeline_value")]
    pub mode: TimelineValue<MaskMode>,
    #[serde(default)]
    pub invert: bool,
}

impl ModifierModel for MaskModifier {
    fn display_name(&self) -> &'static str {
        "Mask"
    }

    fn keywords(&self) -> &'static [&'static str] {
        &["matte", "cutout", "alpha", "clip"]
    }

    fn ensure_ids(&mut self, seen: &mut HashSet<Uuid>) {
        super::ensure_timeline_value_ids(&mut self.mode, seen);
    }

    fn keyframe_span(&self) -> KeyframeSpan {
        super::timeline_value_span(&self.mode)
    }
}

#[derive(Clone, Copy)]
#[cfg(feature = "preview")]
struct MaskPreview {
    map: glam::Mat3,
    size: glam::Vec2,
}

impl MaskModifier {
    #[cfg(feature = "preview")]
    pub(crate) fn preview_provider(
        &self,
        target: PreviewTarget,
        builder: &impl PreviewBuilder,
    ) -> Option<Box<dyn PreviewProvider>> {
        if !preview::is_target(target) {
            return None;
        }
        let geometry = builder.item_geometry(self.item_id?)?;
        Some(Box::new(MaskPreview {
            map: builder.viewport().canvas_to_screen * geometry.local_to_canvas,
            size: geometry.source_size,
        }))
    }
}

#[cfg(feature = "preview")]
impl PreviewProvider for MaskPreview {
    fn on_draw(
        &mut self,
        painter: &shrimply_preview_provider_skia::PreviewCanvas,
        context: &dyn PreviewContext,
    ) {
        preview::draw_rect(
            painter,
            self.map,
            Rect::from_min_size(glam::Vec2::ZERO, self.size),
            context.selection_color(),
        );
    }

    fn on_pointer(
        &mut self,
        _event: PointerEvent<'_>,
        _context: &dyn PreviewContext,
        _edits: &mut dyn PreviewEditSink,
    ) -> PreviewResponse {
        PreviewResponse::IGNORED
    }
}
