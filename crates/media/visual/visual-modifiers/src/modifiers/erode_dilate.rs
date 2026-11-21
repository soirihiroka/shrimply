use hashbrown::HashSet;

use serde::{Deserialize, Serialize};
#[cfg(feature = "preview")]
use shrimply_preview_provider_skia::{
    Color, Cursor, CursorUpdate, PointerEvent, PreviewBuilder, PreviewContext, PreviewEditOutcome,
    PreviewEditSink, PreviewProvider, PreviewRefresh, PreviewResponse, PreviewTarget,
};
use uuid::Uuid;

use super::{KeyframeSpan, ModifierModel, ensure_timeline_value_ids, timeline_value_span};
use shrimply_property_model::timeline_value::*;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErodeDilateOperation {
    Erode,
    #[default]
    Dilate,
}

#[cfg(feature = "preview")]
const RADIUS_COLOR: Color = Color::new(0.35, 0.82, 1.0, 1.0);
#[cfg(feature = "preview")]
struct ErodeDilatePreview {
    target: PreviewTarget,
    original: ErodeDilateModifier,
    map: glam::Mat3,
    size: glam::Vec2,
    radius: f32,
    active: bool,
    changed: bool,
}
#[cfg(feature = "preview")]
impl ErodeDilateModifier {
    #[cfg(feature = "preview")]
    pub(crate) fn preview_provider(
        &self,
        target: PreviewTarget,
        builder: &impl PreviewBuilder,
    ) -> Option<Box<dyn PreviewProvider>> {
        if !super::preview::is_target(target) || !super::preview::editable(&self.radius) {
            return None;
        }
        let (map, size) = super::preview::screen_map(target, builder)?;
        Some(Box::new(ErodeDilatePreview {
            target,
            original: self.clone(),
            map,
            size,
            radius: builder.resolve(&self.radius),
            active: false,
            changed: false,
        }))
    }
}
#[cfg(feature = "preview")]
impl ErodeDilatePreview {
    fn points(&self) -> (glam::Vec2, glam::Vec2) {
        let center = self.size * 0.5;
        (
            self.map.transform_point2(center),
            self.map
                .transform_point2(center + glam::Vec2::X * self.radius.max(0.0)),
        )
    }
}
#[cfg(feature = "preview")]
impl PreviewProvider for ErodeDilatePreview {
    fn on_draw(
        &mut self,
        painter: &shrimply_preview_provider_skia::PreviewCanvas,
        _context: &dyn PreviewContext,
    ) {
        let (center, handle) = self.points();
        super::preview::draw_line(painter, center, handle, RADIUS_COLOR);
        super::preview::draw_handle(painter, handle, RADIUS_COLOR);
    }
    fn on_pointer(
        &mut self,
        event: PointerEvent<'_>,
        _context: &dyn PreviewContext,
        edits: &mut dyn PreviewEditSink,
    ) -> PreviewResponse {
        let time = edits.keyframe_time();
        let handle = self.points().1;
        match event {
            PointerEvent::Hover(input) if super::preview::hit(input.sample.position, handle) => {
                PreviewResponse {
                    handled: true,
                    redraw: false,
                    cursor: CursorUpdate::Set(Cursor::ResizeHorizontal),
                    edit: PreviewEditOutcome::UNCHANGED,
                }
            }
            PointerEvent::Begin(input) if super::preview::hit(input.sample.position, handle) => {
                self.active = true;
                PreviewResponse::handled()
            }
            PointerEvent::Samples { input, .. } if self.active => {
                let Some(local) = super::preview::inverse_point(self.map, input.sample.position)
                else {
                    return PreviewResponse::IGNORED;
                };
                let radius = (local.x - self.size.x * 0.5).max(0.0);
                let changed = super::preview::set_scalar(
                    &mut edits
                        .target_mut(self.target)
                        .downcast_mut::<ErodeDilateModifier>()
                        .expect("erode preview target has wrong type")
                        .radius,
                    time,
                    radius,
                );
                if changed {
                    self.radius = radius;
                }
                self.changed |= changed;
                scalar_edit(changed, false)
            }
            PointerEvent::End(_) if self.active => {
                self.active = false;
                scalar_edit(std::mem::take(&mut self.changed), true)
            }
            PointerEvent::Cancel => {
                if self.changed {
                    *edits
                        .target_mut(self.target)
                        .downcast_mut::<ErodeDilateModifier>()
                        .expect("erode preview target has wrong type") = self.original.clone();
                }
                self.active = false;
                scalar_edit(std::mem::take(&mut self.changed), false)
            }
            _ => PreviewResponse::IGNORED,
        }
    }
}
#[cfg(feature = "preview")]
fn scalar_edit(changed: bool, commit: bool) -> PreviewResponse {
    if !changed {
        return PreviewResponse::handled();
    }
    let refresh = PreviewRefresh::PREVIEW | PreviewRefresh::INSPECTOR;
    PreviewResponse::edited(if commit {
        PreviewEditOutcome::committed(refresh)
    } else {
        PreviewEditOutcome::live(refresh)
    })
}

shrimply_property_model::timeline_value::timeline_step_type!(
    ErodeDilateOperation,
    ErodeDilateOperation::Dilate,
    &[
        TimelineStepVariant {
            value: ErodeDilateOperation::Erode,
            key: "erode",
            label: "Erode",
            icon: None
        },
        TimelineStepVariant {
            value: ErodeDilateOperation::Dilate,
            key: "dilate",
            label: "Dilate",
            icon: None
        },
    ]
);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ErodeDilateModifier {
    #[serde(deserialize_with = "deserialize_timeline_value")]
    pub operation: TimelineValue<ErodeDilateOperation>,
    pub radius: TimelineValue<f32>,
}

impl Default for ErodeDilateModifier {
    fn default() -> Self {
        Self {
            operation: TimelineValue::new_const(ErodeDilateOperation::Dilate),
            radius: TimelineValue::<f32>::new_const(5.0),
        }
    }
}

impl ModifierModel for ErodeDilateModifier {
    fn display_name(&self) -> &'static str {
        "Erode"
    }

    fn keywords(&self) -> &'static [&'static str] {
        &[
            "dilate",
            "morphology",
            "expand mask",
            "contract mask",
            "grow",
            "shrink",
        ]
    }

    fn ensure_ids(&mut self, seen: &mut HashSet<Uuid>) {
        ensure_timeline_value_ids(&mut self.operation, seen);
        ensure_timeline_value_ids(&mut self.radius, seen);
    }

    fn keyframe_span(&self) -> KeyframeSpan {
        super::combine([
            timeline_value_span(&self.operation),
            timeline_value_span(&self.radius),
        ])
    }

    fn number(&self, id: Uuid) -> Option<&TimelineValue<f32>> {
        (self.radius.id == id).then_some(&self.radius)
    }

    fn number_mut(&mut self, id: Uuid) -> Option<&mut TimelineValue<f32>> {
        (self.radius.id == id).then_some(&mut self.radius)
    }
}
