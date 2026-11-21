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
pub struct DisplacementMapModifier {
    pub amount: TimelineValue<f32>,
    pub scale: TimelineValue<f32>,
    pub phase: TimelineValue<f32>,
}

#[cfg(feature = "preview")]
const AMOUNT_COLOR: Color = Color::new(0.35, 0.82, 1.0, 1.0);
#[cfg(feature = "preview")]
const SCALE_COLOR: Color = Color::new(1.0, 0.82, 0.24, 1.0);
#[cfg(feature = "preview")]
const PHASE_COLOR: Color = Color::new(0.75, 0.45, 1.0, 1.0);
#[cfg(feature = "preview")]
const PHASE_RADIUS_RATIO: f32 = 0.18;
#[cfg(feature = "preview")]
const MIN_PHASE_RADIUS: f32 = 48.0;
#[derive(Clone, Copy)]
#[cfg(feature = "preview")]
enum DisplacementControl {
    Amount,
    Scale,
    Phase,
}
#[cfg(feature = "preview")]
struct DisplacementPreview {
    target: PreviewTarget,
    original: DisplacementMapModifier,
    map: glam::Mat3,
    size: glam::Vec2,
    values: [f32; 3],
    editable: [bool; 3],
    active: Option<DisplacementControl>,
    changed: bool,
}
#[cfg(feature = "preview")]
impl DisplacementMapModifier {
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
            super::preview::editable(&self.amount),
            super::preview::editable(&self.scale),
            super::preview::editable(&self.phase),
        ];
        if !editable.iter().any(|value| *value) {
            return None;
        }
        let (map, size) = super::preview::screen_map(target, builder)?;
        Some(Box::new(DisplacementPreview {
            target,
            original: self.clone(),
            map,
            size,
            values: [
                builder.resolve(&self.amount),
                builder.resolve(&self.scale),
                builder.resolve(&self.phase),
            ],
            editable,
            active: None,
            changed: false,
        }))
    }
}
#[cfg(feature = "preview")]
impl DisplacementPreview {
    fn points(&self) -> (glam::Vec2, [glam::Vec2; 3]) {
        let center = self.size * 0.5;
        let phase = glam::Vec2::from_angle(self.values[2].to_radians())
            * (self.size.min_element() * PHASE_RADIUS_RATIO).max(MIN_PHASE_RADIUS);
        (
            self.map.transform_point2(center),
            [
                self.map
                    .transform_point2(center + glam::Vec2::X * self.values[0]),
                self.map
                    .transform_point2(center + glam::Vec2::Y * self.values[1].max(1.0)),
                self.map.transform_point2(center + phase),
            ],
        )
    }
    fn hit(&self, point: glam::Vec2) -> Option<DisplacementControl> {
        let (_, handles) = self.points();
        [
            (DisplacementControl::Amount, 0),
            (DisplacementControl::Scale, 1),
            (DisplacementControl::Phase, 2),
        ]
        .into_iter()
        .find_map(|(control, index)| {
            (self.editable[index] && super::preview::hit(point, handles[index])).then_some(control)
        })
    }
}
#[cfg(feature = "preview")]
impl PreviewProvider for DisplacementPreview {
    fn on_draw(
        &mut self,
        painter: &shrimply_preview_provider_skia::PreviewCanvas,
        _context: &dyn PreviewContext,
    ) {
        let (center, handles) = self.points();
        for (handle, color) in [
            (handles[0], AMOUNT_COLOR),
            (handles[1], SCALE_COLOR),
            (handles[2], PHASE_COLOR),
        ] {
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
                let cursor = match control {
                    DisplacementControl::Amount => Cursor::ResizeHorizontal,
                    DisplacementControl::Scale => Cursor::ResizeVertical,
                    DisplacementControl::Phase => Cursor::Grab,
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
                let delta = local - self.size * 0.5;
                let owner = edits
                    .target_mut(self.target)
                    .downcast_mut::<DisplacementMapModifier>()
                    .expect("displacement preview target has wrong type");
                let (index, value) = match control {
                    DisplacementControl::Amount => (0, delta.x),
                    DisplacementControl::Scale => (1, delta.y.max(1.0)),
                    DisplacementControl::Phase => (2, delta.y.atan2(delta.x).to_degrees()),
                };
                let changed = match control {
                    DisplacementControl::Amount => {
                        super::preview::set_scalar(&mut owner.amount, time, value)
                    }
                    DisplacementControl::Scale => {
                        super::preview::set_scalar(&mut owner.scale, time, value)
                    }
                    DisplacementControl::Phase => {
                        super::preview::set_scalar(&mut owner.phase, time, value)
                    }
                };
                if changed {
                    self.values[index] = value;
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
                        .downcast_mut::<DisplacementMapModifier>()
                        .expect("displacement preview target has wrong type") =
                        self.original.clone();
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

impl Default for DisplacementMapModifier {
    fn default() -> Self {
        Self {
            amount: TimelineValue::<f32>::new_const(10.0),
            scale: TimelineValue::<f32>::new_const(50.0),
            phase: TimelineValue::<f32>::new_const(0.0),
        }
    }
}

impl ModifierModel for DisplacementMapModifier {
    fn display_name(&self) -> &'static str {
        "Displacement map"
    }

    fn keywords(&self) -> &'static [&'static str] {
        &["warp", "distortion", "height map", "texture displacement"]
    }

    fn ensure_ids(&mut self, seen: &mut HashSet<Uuid>) {
        ensure_timeline_value_ids(&mut self.amount, seen);
        ensure_timeline_value_ids(&mut self.scale, seen);
        ensure_timeline_value_ids(&mut self.phase, seen);
    }

    fn keyframe_span(&self) -> KeyframeSpan {
        combine([
            timeline_value_span(&self.amount),
            timeline_value_span(&self.scale),
            timeline_value_span(&self.phase),
        ])
    }

    fn number(&self, id: Uuid) -> Option<&TimelineValue<f32>> {
        [&self.amount, &self.scale, &self.phase]
            .into_iter()
            .find(|value| value.id == id)
    }

    fn number_mut(&mut self, id: Uuid) -> Option<&mut TimelineValue<f32>> {
        [&mut self.amount, &mut self.scale, &mut self.phase]
            .into_iter()
            .find(|value| value.id == id)
    }
}
