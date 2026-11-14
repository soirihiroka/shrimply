use hashbrown::HashSet;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{KeyframeSpan, ModifierModel, combine, ensure_timeline_value_ids, timeline_value_span};
#[cfg(feature = "preview")]
use shrimply_preview_provider_skia::{
    Cursor, CursorUpdate, PointerEvent, PreviewBuilder, PreviewContext, PreviewEditOutcome,
    PreviewEditSink, PreviewProvider, PreviewRefresh, PreviewResponse, PreviewTarget,
};
use shrimply_property_model::timeline_value::*;

#[cfg(feature = "preview")]
use super::preview;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepeatOffsetAxis {
    #[default]
    X,
    Y,
}

shrimply_property_model::timeline_value::timeline_step_type!(
    RepeatOffsetAxis,
    RepeatOffsetAxis::X,
    &[
        TimelineStepVariant {
            value: RepeatOffsetAxis::X,
            key: "x",
            label: "X",
            icon: None
        },
        TimelineStepVariant {
            value: RepeatOffsetAxis::Y,
            key: "y",
            label: "Y",
            icon: None
        },
    ]
);

fn default_copies_x() -> TimelineValue<f32> {
    TimelineValue::<f32>::new_const(2.0)
}
fn default_copies_y() -> TimelineValue<f32> {
    TimelineValue::<f32>::new_const(1.0)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RepeatModifier {
    #[serde(default = "default_copies_x")]
    pub copies_x: TimelineValue<f32>,
    #[serde(default = "default_copies_y")]
    pub copies_y: TimelineValue<f32>,
    pub step: TimelineValue<glam::Vec2>,
    pub row_offset: TimelineValue<f32>,
    #[serde(default, deserialize_with = "deserialize_timeline_value")]
    pub row_offset_axis: TimelineValue<RepeatOffsetAxis>,
}

impl Default for RepeatModifier {
    fn default() -> Self {
        Self {
            copies_x: default_copies_x(),
            copies_y: default_copies_y(),
            step: TimelineValue::<glam::Vec2>::new_const(glam::Vec2::new(100.0, 100.0)),
            row_offset: TimelineValue::<f32>::new_const(0.0),
            row_offset_axis: TimelineValue::new_const(RepeatOffsetAxis::X),
        }
    }
}

impl ModifierModel for RepeatModifier {
    fn display_name(&self) -> &'static str {
        "Repeat"
    }

    fn keywords(&self) -> &'static [&'static str] {
        &["copies", "duplicate", "clone", "tile", "array"]
    }

    fn ensure_ids(&mut self, seen: &mut HashSet<Uuid>) {
        ensure_timeline_value_ids(&mut self.copies_x, seen);
        ensure_timeline_value_ids(&mut self.copies_y, seen);
        ensure_timeline_value_ids(&mut self.step, seen);
        ensure_timeline_value_ids(&mut self.row_offset, seen);
        ensure_timeline_value_ids(&mut self.row_offset_axis, seen);
    }

    fn keyframe_span(&self) -> KeyframeSpan {
        combine([
            timeline_value_span(&self.copies_x),
            timeline_value_span(&self.copies_y),
            timeline_value_span(&self.step),
            timeline_value_span(&self.row_offset),
            timeline_value_span(&self.row_offset_axis),
        ])
    }

    fn number(&self, id: Uuid) -> Option<&TimelineValue<f32>> {
        [&self.copies_x, &self.copies_y, &self.row_offset]
            .into_iter()
            .find(|value| value.id == id)
    }

    fn number_mut(&mut self, id: Uuid) -> Option<&mut TimelineValue<f32>> {
        [&mut self.copies_x, &mut self.copies_y, &mut self.row_offset]
            .into_iter()
            .find(|value| value.id == id)
            .map(|value| &mut *value)
    }

    fn number2(&self, id: Uuid) -> Option<&TimelineValue<glam::Vec2>> {
        (self.step.id == id).then_some(&self.step)
    }

    fn number2_mut(&mut self, id: Uuid) -> Option<&mut TimelineValue<glam::Vec2>> {
        (self.step.id == id).then_some(&mut self.step)
    }
}

#[derive(Clone)]
#[cfg(feature = "preview")]
struct RepeatPreview {
    target: PreviewTarget,
    snapshot: RepeatModifier,
    map: glam::Mat3,
    size: glam::Vec2,
    step: glam::Vec2,
    row_offset: f32,
    axis: RepeatOffsetAxis,
    editable: [bool; 2],
    active: Option<RepeatDrag>,
    changed: bool,
}

#[derive(Clone, Copy)]
#[cfg(feature = "preview")]
enum RepeatDrag {
    Step {
        map: glam::Mat3,
        center: glam::Vec2,
    },
    Row {
        map: glam::Mat3,
        center: glam::Vec2,
        axis: RepeatOffsetAxis,
    },
}

#[derive(Clone, Copy)]
#[cfg(feature = "preview")]
enum RepeatValue {
    Step(glam::Vec2),
    Row(f32),
}

impl RepeatModifier {
    #[cfg(feature = "preview")]
    pub(crate) fn preview_provider(
        &self,
        target: PreviewTarget,
        builder: &impl PreviewBuilder,
    ) -> Option<Box<dyn PreviewProvider>> {
        if !preview::is_target(target) {
            return None;
        }
        let (map, size) = preview::screen_map(target, builder)?;
        Some(Box::new(RepeatPreview {
            target,
            snapshot: self.clone(),
            map,
            size,
            step: builder.resolve(&self.step),
            row_offset: builder.resolve(&self.row_offset),
            axis: builder.resolve(&self.row_offset_axis),
            editable: [
                preview::editable(&self.step),
                preview::editable(&self.row_offset),
            ],
            active: None,
            changed: false,
        }))
    }
}

#[cfg(feature = "preview")]
impl RepeatPreview {
    fn center(&self) -> glam::Vec2 {
        self.size * 0.5
    }
    fn step_point(&self) -> glam::Vec2 {
        self.center() + self.step
    }
    fn row_point(&self) -> glam::Vec2 {
        self.center()
            + match self.axis {
                RepeatOffsetAxis::X => glam::Vec2::X * self.row_offset,
                RepeatOffsetAxis::Y => glam::Vec2::Y * self.row_offset,
            }
    }
}

#[cfg(feature = "preview")]
impl PreviewProvider for RepeatPreview {
    fn on_draw(
        &mut self,
        painter: &shrimply_preview_provider_skia::PreviewCanvas,
        context: &dyn PreviewContext,
    ) {
        let color = context.selection_color();
        let center = self.map.transform_point2(self.center());
        let step = self.map.transform_point2(self.step_point());
        let row = self.map.transform_point2(self.row_point());
        preview::draw_line(painter, center, step, color);
        preview::draw_handle(painter, step, color);
        preview::draw_line(
            painter,
            center,
            row,
            shrimply_preview_provider_skia::Color::new(1.0, 0.82, 0.24, 1.0),
        );
        preview::draw_handle(
            painter,
            row,
            shrimply_preview_provider_skia::Color::new(1.0, 0.82, 0.24, 1.0),
        );
    }

    fn on_pointer(
        &mut self,
        event: PointerEvent<'_>,
        _context: &dyn PreviewContext,
        edits: &mut dyn PreviewEditSink,
    ) -> PreviewResponse {
        let time = edits.keyframe_time();
        let hit = |point| {
            if self.editable[0] && preview::hit(point, self.map.transform_point2(self.step_point()))
            {
                return Some(RepeatDrag::Step {
                    map: self.map,
                    center: self.center(),
                });
            }
            (self.editable[1] && preview::hit(point, self.map.transform_point2(self.row_point())))
                .then(|| RepeatDrag::Row {
                    map: self.map,
                    center: self.center(),
                    axis: self.axis,
                })
        };
        match event {
            PointerEvent::Hover(input) if hit(input.sample.position).is_some() => PreviewResponse {
                handled: true,
                redraw: false,
                cursor: CursorUpdate::Set(Cursor::Move),
                edit: PreviewEditOutcome::UNCHANGED,
            },
            PointerEvent::Begin(input) => {
                self.active = hit(input.sample.position);
                if self.active.is_some() {
                    PreviewResponse::handled()
                } else {
                    PreviewResponse::IGNORED
                }
            }
            PointerEvent::Samples { input, .. } => {
                let Some(drag) = self.active else {
                    return PreviewResponse::IGNORED;
                };
                let Some((changed, value)) = update_repeat(
                    edits
                        .target_mut(self.target)
                        .downcast_mut::<RepeatModifier>()
                        .expect("repeat preview target has wrong type"),
                    &drag,
                    input.sample.position,
                    time,
                ) else {
                    return PreviewResponse::IGNORED;
                };
                if changed {
                    match value {
                        RepeatValue::Step(step) => self.step = step,
                        RepeatValue::Row(offset) => self.row_offset = offset,
                    }
                }
                self.changed |= changed;
                repeat_edit(changed, false)
            }
            PointerEvent::End(_) if self.active.is_some() => {
                self.active = None;
                repeat_edit(std::mem::take(&mut self.changed), true)
            }
            PointerEvent::Cancel => {
                if self.changed {
                    *edits
                        .target_mut(self.target)
                        .downcast_mut::<RepeatModifier>()
                        .expect("repeat preview target has wrong type") = self.snapshot.clone();
                }
                self.active = None;
                repeat_edit(std::mem::take(&mut self.changed), false)
            }
            _ => PreviewResponse::IGNORED,
        }
    }
}

#[cfg(feature = "preview")]
fn repeat_edit(changed: bool, commit: bool) -> PreviewResponse {
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

#[cfg(feature = "preview")]
fn update_repeat(
    modifier: &mut RepeatModifier,
    drag: &RepeatDrag,
    point: glam::Vec2,
    time: shrimply_property_model::Time,
) -> Option<(bool, RepeatValue)> {
    let (map, center) = match *drag {
        RepeatDrag::Step { map, center } | RepeatDrag::Row { map, center, .. } => (map, center),
    };
    let local = preview::inverse_point(map, point)?;
    let delta = local - center;
    let (changed, value) = match *drag {
        RepeatDrag::Step { .. } => (
            preview::set_vec2(&mut modifier.step, time, delta),
            RepeatValue::Step(delta),
        ),
        RepeatDrag::Row { axis, .. } => {
            let value = if axis == RepeatOffsetAxis::X {
                delta.x
            } else {
                delta.y
            };
            (
                preview::set_scalar(&mut modifier.row_offset, time, value),
                RepeatValue::Row(value),
            )
        }
    };
    Some((changed, value))
}
