use super::{KeyframeSpan, ModifierModel, combine, ensure_timeline_value_ids, timeline_value_span};
use hashbrown::HashSet;
use serde::{Deserialize, Serialize};
#[cfg(feature = "preview")]
use shrimply_preview_provider_skia::{
    Color, Cursor, CursorUpdate, PointerEvent, PreviewBuilder, PreviewContext, PreviewEditOutcome,
    PreviewEditSink, PreviewProvider, PreviewRefresh, PreviewResponse, PreviewTarget,
};
use shrimply_property_model::timeline_value::*;
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VignetteModifier {
    pub amount: TimelineValue<f32>,
    pub midpoint: TimelineValue<f32>,
    pub softness: TimelineValue<f32>,
}

#[cfg(feature = "preview")]
const MIDPOINT_COLOR: Color = Color::new(0.35, 0.82, 1.0, 1.0);
#[cfg(feature = "preview")]
const SOFTNESS_COLOR: Color = Color::new(1.0, 0.82, 0.24, 1.0);
#[derive(Clone, Copy)]
#[cfg(feature = "preview")]
enum VignetteControl {
    Midpoint,
    Softness,
}
#[cfg(feature = "preview")]
struct VignettePreview {
    target: PreviewTarget,
    original: VignetteModifier,
    map: glam::Mat3,
    size: glam::Vec2,
    values: glam::Vec2,
    editable: [bool; 2],
    active: Option<VignetteControl>,
    changed: bool,
}
#[cfg(feature = "preview")]
impl VignetteModifier {
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
            super::preview::editable(&self.midpoint),
            super::preview::editable(&self.softness),
        ];
        if !editable.iter().any(|value| *value) {
            return None;
        }
        let (map, size) = super::preview::screen_map(target, builder)?;
        Some(Box::new(VignettePreview {
            target,
            original: self.clone(),
            map,
            size,
            values: glam::Vec2::new(
                builder.resolve(&self.midpoint),
                builder.resolve(&self.softness),
            ),
            editable,
            active: None,
            changed: false,
        }))
    }
}
#[cfg(feature = "preview")]
impl VignettePreview {
    fn points(&self) -> (glam::Vec2, [glam::Vec2; 2]) {
        let center = self.size * 0.5;
        (
            self.map.transform_point2(center),
            [
                self.map.transform_point2(
                    center
                        + glam::Vec2::X
                            * self.values.x.clamp(0.0, 1.0)
                            * self.size.x
                            * 0.5
                            * core::f32::consts::SQRT_2,
                ),
                self.map.transform_point2(
                    center + glam::Vec2::Y * self.values.y.clamp(0.0, 1.0) * self.size.y * 0.5,
                ),
            ],
        )
    }
    fn hit(&self, point: glam::Vec2) -> Option<VignetteControl> {
        let (_, handles) = self.points();
        if self.editable[0] && super::preview::hit(point, handles[0]) {
            Some(VignetteControl::Midpoint)
        } else if self.editable[1] && super::preview::hit(point, handles[1]) {
            Some(VignetteControl::Softness)
        } else {
            None
        }
    }
}
#[cfg(feature = "preview")]
impl PreviewProvider for VignettePreview {
    fn on_draw(
        &mut self,
        painter: &shrimply_preview_provider_skia::PreviewCanvas,
        _context: &dyn PreviewContext,
    ) {
        let (center, handles) = self.points();
        for (handle, color) in [(handles[0], MIDPOINT_COLOR), (handles[1], SOFTNESS_COLOR)] {
            super::preview::draw_line(painter, center, handle, color);
            super::preview::draw_handle(painter, handle, color);
        }
    }
    fn on_pointer(
        &mut self,
        event: PointerEvent<'_>,
        _context: &dyn PreviewContext,
        edits: &mut dyn PreviewEditSink,
    ) -> PreviewResponse {
        let time = edits.keyframe_time();
        match event {
            PointerEvent::Hover(input) => {
                let Some(control) = self.hit(input.sample.position) else {
                    return PreviewResponse::IGNORED;
                };
                let cursor = if matches!(control, VignetteControl::Midpoint) {
                    Cursor::ResizeHorizontal
                } else {
                    Cursor::ResizeVertical
                };
                PreviewResponse {
                    handled: true,
                    redraw: false,
                    cursor: CursorUpdate::Set(cursor),
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
                let owner = edits
                    .target_mut(self.target)
                    .downcast_mut::<VignetteModifier>()
                    .expect("vignette preview target has wrong type");
                let value = match control {
                    VignetteControl::Midpoint => ((local.x - self.size.x * 0.5)
                        / (self.size.x * 0.5).max(1.0)
                        / core::f32::consts::SQRT_2)
                        .clamp(0.0, 1.0),
                    VignetteControl::Softness => ((local.y - self.size.y * 0.5)
                        / (self.size.y * 0.5).max(1.0))
                    .clamp(0.0, 1.0),
                };
                let changed = match control {
                    VignetteControl::Midpoint => {
                        super::preview::set_scalar(&mut owner.midpoint, time, value)
                    }
                    VignetteControl::Softness => {
                        super::preview::set_scalar(&mut owner.softness, time, value)
                    }
                };
                if changed {
                    match control {
                        VignetteControl::Midpoint => self.values.x = value,
                        VignetteControl::Softness => self.values.y = value,
                    }
                }
                self.changed |= changed;
                scalar_edit(changed, false)
            }
            PointerEvent::End(_) if self.active.is_some() => {
                self.active = None;
                scalar_edit(std::mem::take(&mut self.changed), true)
            }
            PointerEvent::Cancel => {
                if self.changed {
                    *edits
                        .target_mut(self.target)
                        .downcast_mut::<VignetteModifier>()
                        .expect("vignette preview target has wrong type") = self.original.clone();
                }
                self.active = None;
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
impl Default for VignetteModifier {
    fn default() -> Self {
        Self {
            amount: TimelineValue::<f32>::new_const(0.5),
            midpoint: TimelineValue::<f32>::new_const(0.5),
            softness: TimelineValue::<f32>::new_const(0.5),
        }
    }
}
impl ModifierModel for VignetteModifier {
    fn display_name(&self) -> &'static str {
        "Vignette"
    }

    fn keywords(&self) -> &'static [&'static str] {
        &["edge darkening", "dark corners", "spotlight"]
    }

    fn ensure_ids(&mut self, seen: &mut HashSet<Uuid>) {
        for value in [&mut self.amount, &mut self.midpoint, &mut self.softness] {
            ensure_timeline_value_ids(value, seen);
        }
    }
    fn keyframe_span(&self) -> KeyframeSpan {
        combine([&self.amount, &self.midpoint, &self.softness].map(timeline_value_span))
    }
    fn number(&self, id: Uuid) -> Option<&TimelineValue<f32>> {
        [&self.amount, &self.midpoint, &self.softness]
            .into_iter()
            .find(|value| value.id == id)
    }
    fn number_mut(&mut self, id: Uuid) -> Option<&mut TimelineValue<f32>> {
        [&mut self.amount, &mut self.midpoint, &mut self.softness]
            .into_iter()
            .find(|value| value.id == id)
    }
}
