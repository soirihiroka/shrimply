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
pub struct WaveRippleModifier {
    pub amplitude: TimelineValue<f32>,
    pub wavelength: TimelineValue<f32>,
    pub angle_degrees: TimelineValue<f32>,
    pub phase: TimelineValue<f32>,
}

#[cfg(feature = "preview")]
const ANGLE_HANDLE_DISTANCE: f32 = 48.0;
#[derive(Clone, Copy)]
#[cfg(feature = "preview")]
enum WaveControl {
    Amplitude,
    Angle,
}
#[cfg(feature = "preview")]
struct WavePreview {
    target: PreviewTarget,
    original: WaveRippleModifier,
    map: glam::Mat3,
    size: glam::Vec2,
    amplitude: f32,
    angle: f32,
    editable: [bool; 2],
    active: Option<WaveControl>,
    changed: bool,
}
#[cfg(feature = "preview")]
impl WaveRippleModifier {
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
            super::preview::editable(&self.amplitude),
            super::preview::editable(&self.angle_degrees),
        ];
        if !editable.iter().any(|value| *value) {
            return None;
        }
        let (map, size) = super::preview::screen_map(target, builder)?;
        Some(Box::new(WavePreview {
            target,
            original: self.clone(),
            map,
            size,
            amplitude: builder.resolve(&self.amplitude),
            angle: builder.resolve(&self.angle_degrees),
            editable,
            active: None,
            changed: false,
        }))
    }
}
#[cfg(feature = "preview")]
impl WavePreview {
    fn points(&self) -> (glam::Vec2, glam::Vec2, glam::Vec2) {
        let center = self.size * 0.5;
        let direction = glam::Vec2::from_angle(self.angle.to_radians());
        (
            self.map.transform_point2(center),
            self.map
                .transform_point2(center + direction * self.amplitude),
            self.map
                .transform_point2(center + direction * ANGLE_HANDLE_DISTANCE),
        )
    }
    fn hit(&self, point: glam::Vec2) -> Option<WaveControl> {
        let (_, amplitude, angle) = self.points();
        if self.editable[0] && super::preview::hit(point, amplitude) {
            Some(WaveControl::Amplitude)
        } else if self.editable[1] && super::preview::hit(point, angle) {
            Some(WaveControl::Angle)
        } else {
            None
        }
    }
}
#[cfg(feature = "preview")]
impl PreviewProvider for WavePreview {
    fn on_draw(
        &mut self,
        painter: &shrimply_preview_provider_skia::PreviewCanvas,
        context: &dyn PreviewContext,
    ) {
        let (center, amplitude, angle) = self.points();
        let color = context.selection_color();
        super::preview::draw_line(painter, center, amplitude, color);
        super::preview::draw_handle(painter, amplitude, color);
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
                    .downcast_mut::<WaveRippleModifier>()
                    .expect("wave preview target has wrong type");
                let value = match control {
                    WaveControl::Amplitude => delta
                        .dot(glam::Vec2::from_angle(self.angle.to_radians()))
                        .max(0.0),
                    WaveControl::Angle => delta.y.atan2(delta.x).to_degrees(),
                };
                let changed = match control {
                    WaveControl::Amplitude => {
                        super::preview::set_scalar(&mut owner.amplitude, time, value)
                    }
                    WaveControl::Angle => {
                        super::preview::set_scalar(&mut owner.angle_degrees, time, value)
                    }
                };
                if changed {
                    match control {
                        WaveControl::Amplitude => self.amplitude = value,
                        WaveControl::Angle => self.angle = value,
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
                        .downcast_mut::<WaveRippleModifier>()
                        .expect("wave preview target has wrong type") = self.original.clone();
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

impl Default for WaveRippleModifier {
    fn default() -> Self {
        Self {
            amplitude: TimelineValue::<f32>::new_const(10.0),
            wavelength: TimelineValue::<f32>::new_const(50.0),
            angle_degrees: TimelineValue::<f32>::new_const(0.0),
            phase: TimelineValue::<f32>::new_const(0.0),
        }
    }
}

impl ModifierModel for WaveRippleModifier {
    fn display_name(&self) -> &'static str {
        "Wave"
    }

    fn keywords(&self) -> &'static [&'static str] {
        &["ripple", "distortion", "warp", "displacement", "water"]
    }

    fn ensure_ids(&mut self, seen: &mut HashSet<Uuid>) {
        ensure_timeline_value_ids(&mut self.amplitude, seen);
        ensure_timeline_value_ids(&mut self.wavelength, seen);
        ensure_timeline_value_ids(&mut self.angle_degrees, seen);
        ensure_timeline_value_ids(&mut self.phase, seen);
    }

    fn keyframe_span(&self) -> KeyframeSpan {
        combine([
            timeline_value_span(&self.amplitude),
            timeline_value_span(&self.wavelength),
            timeline_value_span(&self.angle_degrees),
            timeline_value_span(&self.phase),
        ])
    }

    fn number(&self, id: Uuid) -> Option<&TimelineValue<f32>> {
        [
            &self.amplitude,
            &self.wavelength,
            &self.angle_degrees,
            &self.phase,
        ]
        .into_iter()
        .find(|value| value.id == id)
    }

    fn number_mut(&mut self, id: Uuid) -> Option<&mut TimelineValue<f32>> {
        [
            &mut self.amplitude,
            &mut self.wavelength,
            &mut self.angle_degrees,
            &mut self.phase,
        ]
        .into_iter()
        .find(|value| value.id == id)
    }
}
