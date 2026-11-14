use hashbrown::HashSet;

use serde::{Deserialize, Serialize};
use shrimply_property_model::timeline_value::*;
use uuid::Uuid;

#[cfg(feature = "preview")]
use shrimply_preview_provider_skia::{
    Cursor, CursorUpdate, PointerEvent, PreviewBuilder, PreviewContext, PreviewEditOutcome,
    PreviewEditSink, PreviewProvider, PreviewRefresh, PreviewResponse, PreviewTarget,
};

#[cfg(feature = "preview")]
use super::preview;

use super::{KeyframeSpan, ModifierModel, combine, ensure_timeline_value_ids, timeline_value_span};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CornerPinModifier {
    pub top_left: TimelineValue<glam::Vec2>,
    pub top_right: TimelineValue<glam::Vec2>,
    pub bottom_right: TimelineValue<glam::Vec2>,
    pub bottom_left: TimelineValue<glam::Vec2>,
    pub perspective: TimelineValue<f32>,
}

impl Default for CornerPinModifier {
    fn default() -> Self {
        Self {
            top_left: TimelineValue::new_const(glam::Vec2::ZERO),
            top_right: TimelineValue::new_const(glam::Vec2::X),
            bottom_right: TimelineValue::new_const(glam::Vec2::ONE),
            bottom_left: TimelineValue::new_const(glam::Vec2::Y),
            perspective: TimelineValue::new_const(0.0),
        }
    }
}

impl ModifierModel for CornerPinModifier {
    fn display_name(&self) -> &'static str {
        "Corner Pin"
    }

    fn keywords(&self) -> &'static [&'static str] {
        &["perspective", "keystone", "four point", "4 point"]
    }

    fn ensure_ids(&mut self, seen: &mut HashSet<Uuid>) {
        ensure_timeline_value_ids(&mut self.top_left, seen);
        ensure_timeline_value_ids(&mut self.top_right, seen);
        ensure_timeline_value_ids(&mut self.bottom_right, seen);
        ensure_timeline_value_ids(&mut self.bottom_left, seen);
        ensure_timeline_value_ids(&mut self.perspective, seen);
    }

    fn keyframe_span(&self) -> KeyframeSpan {
        combine([
            timeline_value_span(&self.top_left),
            timeline_value_span(&self.top_right),
            timeline_value_span(&self.bottom_right),
            timeline_value_span(&self.bottom_left),
            timeline_value_span(&self.perspective),
        ])
    }

    fn number(&self, id: Uuid) -> Option<&TimelineValue<f32>> {
        (self.perspective.id == id).then_some(&self.perspective)
    }

    fn number_mut(&mut self, id: Uuid) -> Option<&mut TimelineValue<f32>> {
        (self.perspective.id == id).then_some(&mut self.perspective)
    }

    fn number2(&self, id: Uuid) -> Option<&TimelineValue<glam::Vec2>> {
        [
            &self.top_left,
            &self.top_right,
            &self.bottom_right,
            &self.bottom_left,
        ]
        .into_iter()
        .find(|value| value.id == id)
    }

    fn number2_mut(&mut self, id: Uuid) -> Option<&mut TimelineValue<glam::Vec2>> {
        [
            &mut self.top_left,
            &mut self.top_right,
            &mut self.bottom_right,
            &mut self.bottom_left,
        ]
        .into_iter()
        .find(|value| value.id == id)
    }
}

#[derive(Clone)]
#[cfg(feature = "preview")]
struct CornerPinPreview {
    target: PreviewTarget,
    snapshot: CornerPinModifier,
    map: glam::Mat3,
    size: glam::Vec2,
    corners: [glam::Vec2; 4],
    editable: [bool; 5],
    perspective: f32,
    active: Option<CornerPinDrag>,
    changed: bool,
}

#[derive(Clone, Copy)]
#[cfg(feature = "preview")]
enum CornerPinDrag {
    Corner {
        map: glam::Mat3,
        size: glam::Vec2,
        index: usize,
    },
    Perspective {
        map: glam::Mat3,
        start: glam::Vec2,
        value: f32,
        radius: f32,
    },
}

#[derive(Clone, Copy)]
#[cfg(feature = "preview")]
enum CornerPinValue {
    Corner { index: usize, value: glam::Vec2 },
    Perspective(f32),
}

impl CornerPinModifier {
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
        Some(Box::new(CornerPinPreview {
            target,
            snapshot: self.clone(),
            map,
            size,
            corners: [
                builder.resolve(&self.top_left),
                builder.resolve(&self.top_right),
                builder.resolve(&self.bottom_right),
                builder.resolve(&self.bottom_left),
            ],
            editable: [
                preview::editable(&self.top_left),
                preview::editable(&self.top_right),
                preview::editable(&self.bottom_right),
                preview::editable(&self.bottom_left),
                preview::editable(&self.perspective),
            ],
            perspective: builder.resolve(&self.perspective),
            active: None,
            changed: false,
        }))
    }
}

#[cfg(feature = "preview")]
impl CornerPinPreview {
    fn local_corners(&self) -> [glam::Vec2; 4] {
        self.corners.map(|corner| corner * self.size)
    }
    fn center(&self) -> glam::Vec2 {
        self.local_corners().into_iter().sum::<glam::Vec2>() * 0.25
    }
}

#[cfg(feature = "preview")]
impl PreviewProvider for CornerPinPreview {
    fn on_draw(
        &mut self,
        painter: &shrimply_preview_provider_skia::PreviewCanvas,
        context: &dyn PreviewContext,
    ) {
        let color = context.selection_color();
        let points = self
            .local_corners()
            .map(|point| self.map.transform_point2(point));
        shrimply_preview_provider_skia::drawing::polyline(
            painter,
            &points,
            true,
            shrimply_preview_provider_skia::Paint::stroke(
                shrimply_preview_provider_skia::Stroke::new(color, preview::LINE_WIDTH),
            ),
        );
        for point in points {
            preview::draw_handle(painter, point, color);
        }
        preview::draw_handle(painter, self.map.transform_point2(self.center()), color);
    }

    fn on_pointer(
        &mut self,
        event: PointerEvent<'_>,
        _context: &dyn PreviewContext,
        edits: &mut dyn PreviewEditSink,
    ) -> PreviewResponse {
        let time = edits.keyframe_time();
        let hit = |point| {
            for (index, corner) in self.local_corners().into_iter().enumerate() {
                if self.editable[index] && preview::hit(point, self.map.transform_point2(corner)) {
                    return Some(CornerPinDrag::Corner {
                        map: self.map,
                        size: self.size,
                        index,
                    });
                }
            }
            let center = self.map.transform_point2(self.center());
            (self.editable[4] && preview::hit(point, center)).then(|| CornerPinDrag::Perspective {
                map: self.map,
                start: preview::inverse_point(self.map, point).expect("corner pin map is singular"),
                value: self.perspective,
                radius: self.size.min_element().max(1.0) * 0.45,
            })
        };
        match event {
            PointerEvent::Hover(input) => {
                let Some(drag) = hit(input.sample.position) else {
                    return PreviewResponse::IGNORED;
                };
                let cursor = if matches!(drag, CornerPinDrag::Perspective { .. }) {
                    Cursor::ResizeHorizontal
                } else {
                    Cursor::Move
                };
                PreviewResponse {
                    handled: true,
                    redraw: false,
                    cursor: CursorUpdate::Set(cursor),
                    edit: PreviewEditOutcome::UNCHANGED,
                }
            }
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
                let Some((changed, value)) = update_corner_pin(
                    edits
                        .target_mut(self.target)
                        .downcast_mut::<CornerPinModifier>()
                        .expect("corner pin preview target has wrong type"),
                    &drag,
                    input.sample.position,
                    time,
                ) else {
                    return PreviewResponse::IGNORED;
                };
                if changed {
                    match value {
                        CornerPinValue::Corner { index, value } => self.corners[index] = value,
                        CornerPinValue::Perspective(value) => self.perspective = value,
                    }
                }
                self.changed |= changed;
                corner_edit(changed, false)
            }
            PointerEvent::End(_) if self.active.is_some() => {
                self.active = None;
                corner_edit(std::mem::take(&mut self.changed), true)
            }
            PointerEvent::Cancel => {
                if self.changed {
                    *edits
                        .target_mut(self.target)
                        .downcast_mut::<CornerPinModifier>()
                        .expect("corner pin preview target has wrong type") = self.snapshot.clone();
                }
                self.active = None;
                corner_edit(std::mem::take(&mut self.changed), false)
            }
            _ => PreviewResponse::IGNORED,
        }
    }
}

#[cfg(feature = "preview")]
fn corner_edit(changed: bool, commit: bool) -> PreviewResponse {
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
fn update_corner_pin(
    modifier: &mut CornerPinModifier,
    drag: &CornerPinDrag,
    point: glam::Vec2,
    time: shrimply_property_model::Time,
) -> Option<(bool, CornerPinValue)> {
    let (map, local) = match *drag {
        CornerPinDrag::Corner { map, .. } | CornerPinDrag::Perspective { map, .. } => {
            let local = preview::inverse_point(map, point)?;
            (map, local)
        }
    };
    let _ = map;
    let (changed, value) = match *drag {
        CornerPinDrag::Corner { size, index, .. } => {
            let value =
                (local / size.max(glam::Vec2::ONE)).clamp(glam::Vec2::ZERO, glam::Vec2::ONE);
            (
                preview::set_vec2(
                    match index {
                        0 => &mut modifier.top_left,
                        1 => &mut modifier.top_right,
                        2 => &mut modifier.bottom_right,
                        3 => &mut modifier.bottom_left,
                        _ => unreachable!(),
                    },
                    time,
                    value,
                ),
                CornerPinValue::Corner { index, value },
            )
        }
        CornerPinDrag::Perspective {
            start,
            value,
            radius,
            ..
        } => {
            let value = (value + (local.x - start.x) / radius).clamp(0.0, 1.0);
            (
                preview::set_scalar(&mut modifier.perspective, time, value),
                CornerPinValue::Perspective(value),
            )
        }
    };
    Some((changed, value))
}
