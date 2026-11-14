use super::{KeyframeSpan, ModifierModel, combine, ensure_timeline_value_ids, timeline_value_span};
use hashbrown::HashSet;
use serde::{Deserialize, Serialize};
#[cfg(feature = "preview")]
use shrimply_preview_provider_skia::{
    Cursor, CursorUpdate, PointerEvent, PreviewBuilder, PreviewContext, PreviewEditOutcome,
    PreviewEditSink, PreviewProvider, PreviewRefresh, PreviewResponse, PreviewTarget,
};
use shrimply_property_model::timeline_value::*;
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EmbossModifier {
    pub direction_degrees: TimelineValue<f32>,
    pub depth: TimelineValue<f32>,
    pub amount: TimelineValue<f32>,
}

#[cfg(feature = "preview")]
const ANGLE_HANDLE_DISTANCE: f32 = 48.0;
#[cfg(feature = "preview")]
const DEPTH_SCALE: f32 = 8.0;
#[derive(Clone, Copy)]
#[cfg(feature = "preview")]
enum EmbossControl {
    Depth,
    Angle,
}
#[cfg(feature = "preview")]
struct EmbossPreview {
    target: PreviewTarget,
    original: EmbossModifier,
    map: glam::Mat3,
    size: glam::Vec2,
    depth: f32,
    angle: f32,
    editable: [bool; 2],
    active: Option<EmbossControl>,
    changed: bool,
}
#[cfg(feature = "preview")]
impl EmbossModifier {
    #[cfg(feature = "preview")]
    pub(crate) fn preview_provider(
        &self,
        target: PreviewTarget,
        builder: &impl PreviewBuilder,
    ) -> Option<Box<dyn PreviewProvider>> {
        if !super::preview::is_target(target) {
            return None;
        }
        let editable = [
            super::preview::editable(&self.depth),
            super::preview::editable(&self.direction_degrees),
        ];
        if !editable.iter().any(|value| *value) {
            return None;
        }
        let (map, size) = super::preview::screen_map(target, builder)?;
        Some(Box::new(EmbossPreview {
            target,
            original: self.clone(),
            map,
            size,
            depth: builder.resolve(&self.depth),
            angle: builder.resolve(&self.direction_degrees),
            editable,
            active: None,
            changed: false,
        }))
    }
}
#[cfg(feature = "preview")]
impl EmbossPreview {
    fn points(&self) -> (glam::Vec2, glam::Vec2, glam::Vec2) {
        let center = self.size * 0.5;
        let direction = glam::Vec2::from_angle(self.angle.to_radians());
        (
            self.map.transform_point2(center),
            self.map
                .transform_point2(center + direction * self.depth * DEPTH_SCALE),
            self.map
                .transform_point2(center + direction * ANGLE_HANDLE_DISTANCE),
        )
    }
    fn hit(&self, point: glam::Vec2) -> Option<EmbossControl> {
        let (_, depth, angle) = self.points();
        if self.editable[0] && super::preview::hit(point, depth) {
            Some(EmbossControl::Depth)
        } else if self.editable[1] && super::preview::hit(point, angle) {
            Some(EmbossControl::Angle)
        } else {
            None
        }
    }
}
#[cfg(feature = "preview")]
impl PreviewProvider for EmbossPreview {
    fn on_draw(
        &mut self,
        painter: &shrimply_preview_provider_skia::PreviewCanvas,
        context: &dyn PreviewContext,
    ) {
        let (center, depth, angle) = self.points();
        let color = context.selection_color();
        super::preview::draw_line(painter, center, depth, color);
        super::preview::draw_handle(painter, depth, color);
        super::preview::draw_line(painter, center, angle, color);
        super::preview::draw_handle(painter, angle, color);
    }
    fn on_pointer(
        &mut self,
        event: PointerEvent<'_>,
        _context: &dyn PreviewContext,
        edits: &mut dyn PreviewEditSink,
    ) -> PreviewResponse {
        let time = edits.keyframe_time();
        match event {
            PointerEvent::Hover(input) if self.hit(input.sample.position).is_some() => {
                PreviewResponse {
                    handled: true,
                    redraw: false,
                    cursor: CursorUpdate::Set(Cursor::Grab),
                    edit: PreviewEditOutcome::UNCHANGED,
                }
            }
            PointerEvent::Begin(input) => {
                self.active = self.hit(input.sample.position);
                if self.active.is_some() {
                    PreviewResponse::handled()
                } else {
                    PreviewResponse::IGNORED
                }
            }
            PointerEvent::Samples { input, .. } => {
                let Some(control) = self.active else {
                    return PreviewResponse::IGNORED;
                };
                let Some(local) = super::preview::inverse_point(self.map, input.sample.position)
                else {
                    return PreviewResponse::IGNORED;
                };
                let delta = local - self.size * 0.5;
                let owner = edits
                    .target_mut(self.target)
                    .downcast_mut::<EmbossModifier>()
                    .expect("emboss preview target has wrong type");
                let value = match control {
                    EmbossControl::Depth => {
                        delta
                            .dot(glam::Vec2::from_angle(self.angle.to_radians()))
                            .max(0.0)
                            / DEPTH_SCALE
                    }
                    EmbossControl::Angle => delta.y.atan2(delta.x).to_degrees(),
                };
                let changed = match control {
                    EmbossControl::Depth => {
                        super::preview::set_scalar(&mut owner.depth, time, value)
                    }
                    EmbossControl::Angle => {
                        super::preview::set_scalar(&mut owner.direction_degrees, time, value)
                    }
                };
                if changed {
                    match control {
                        EmbossControl::Depth => self.depth = value,
                        EmbossControl::Angle => self.angle = value,
                    }
                }
                self.changed |= changed;
                preview_edit(changed, false)
            }
            PointerEvent::End(_) if self.active.is_some() => {
                self.active = None;
                preview_edit(std::mem::take(&mut self.changed), true)
            }
            PointerEvent::Cancel => {
                if self.changed {
                    *edits
                        .target_mut(self.target)
                        .downcast_mut::<EmbossModifier>()
                        .expect("emboss preview target has wrong type") = self.original.clone();
                }
                self.active = None;
                preview_edit(std::mem::take(&mut self.changed), false)
            }
            _ => PreviewResponse::IGNORED,
        }
    }
}
#[cfg(feature = "preview")]
fn preview_edit(changed: bool, commit: bool) -> PreviewResponse {
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

impl Default for EmbossModifier {
    fn default() -> Self {
        Self {
            direction_degrees: TimelineValue::<f32>::new_const(135.0),
            depth: TimelineValue::<f32>::new_const(1.0),
            amount: TimelineValue::<f32>::new_const(1.0),
        }
    }
}

impl ModifierModel for EmbossModifier {
    fn display_name(&self) -> &'static str {
        "Emboss"
    }

    fn ensure_ids(&mut self, seen: &mut HashSet<Uuid>) {
        ensure_timeline_value_ids(&mut self.direction_degrees, seen);
        ensure_timeline_value_ids(&mut self.depth, seen);
        ensure_timeline_value_ids(&mut self.amount, seen);
    }

    fn keyframe_span(&self) -> KeyframeSpan {
        combine([
            timeline_value_span(&self.direction_degrees),
            timeline_value_span(&self.depth),
            timeline_value_span(&self.amount),
        ])
    }

    fn number(&self, id: Uuid) -> Option<&TimelineValue<f32>> {
        [&self.direction_degrees, &self.depth, &self.amount]
            .into_iter()
            .find(|value| value.id == id)
    }

    fn number_mut(&mut self, id: Uuid) -> Option<&mut TimelineValue<f32>> {
        [
            &mut self.direction_degrees,
            &mut self.depth,
            &mut self.amount,
        ]
        .into_iter()
        .find(|value| value.id == id)
    }
}
