use hashbrown::HashSet;
use serde::{Deserialize, Serialize};
#[cfg(feature = "preview")]
use shrimply_preview_provider_skia::{
    CursorUpdate, PointerEvent, PreviewBuilder, PreviewContext, PreviewEditOutcome,
    PreviewEditSink, PreviewProvider, PreviewRefresh, PreviewResponse, PreviewTarget, Rect,
};
use shrimply_property_model::timeline_value::*;
use uuid::Uuid;

#[cfg(feature = "preview")]
use super::preview::{self, HANDLES, Handle};
use super::{KeyframeSpan, ModifierModel, combine, ensure_timeline_value_ids, timeline_value_span};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CropEdges {
    pub top: TimelineValue<f32>,
    pub right: TimelineValue<f32>,
    pub bottom: TimelineValue<f32>,
    pub left: TimelineValue<f32>,
}
impl Default for CropEdges {
    fn default() -> Self {
        Self {
            top: TimelineValue::new_const(0.0),
            right: TimelineValue::new_const(0.0),
            bottom: TimelineValue::new_const(0.0),
            left: TimelineValue::new_const(0.0),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "mode", content = "values", rename_all = "snake_case")]
pub enum CropModifier {
    Percentage(CropEdges),
    Pixels(CropEdges),
}
impl Default for CropModifier {
    fn default() -> Self {
        Self::Percentage(CropEdges::default())
    }
}
impl CropModifier {
    fn edges(&self) -> &CropEdges {
        match self {
            Self::Percentage(edges) | Self::Pixels(edges) => edges,
        }
    }

    fn edges_mut(&mut self) -> &mut CropEdges {
        match self {
            Self::Percentage(edges) | Self::Pixels(edges) => edges,
        }
    }
}
impl ModifierModel for CropModifier {
    fn display_name(&self) -> &'static str {
        "Crop"
    }
    fn ensure_ids(&mut self, seen: &mut HashSet<Uuid>) {
        let edges = self.edges_mut();
        for value in [
            &mut edges.top,
            &mut edges.right,
            &mut edges.bottom,
            &mut edges.left,
        ] {
            ensure_timeline_value_ids(value, seen);
        }
    }
    fn keyframe_span(&self) -> KeyframeSpan {
        let edges = self.edges();
        combine([&edges.top, &edges.right, &edges.bottom, &edges.left].map(timeline_value_span))
    }
    fn number(&self, id: Uuid) -> Option<&TimelineValue<f32>> {
        let edges = self.edges();
        [&edges.top, &edges.right, &edges.bottom, &edges.left]
            .into_iter()
            .find(|value| value.id == id)
    }
    fn number_mut(&mut self, id: Uuid) -> Option<&mut TimelineValue<f32>> {
        let edges = self.edges_mut();
        [
            &mut edges.top,
            &mut edges.right,
            &mut edges.bottom,
            &mut edges.left,
        ]
        .into_iter()
        .find(|value| value.id == id)
    }
}

#[derive(Clone, Copy)]
#[cfg(feature = "preview")]
struct CropDrag {
    map: glam::Mat3,
    size: glam::Vec2,
    values: [f32; 4],
    pixels: bool,
    handle: Handle,
}
#[cfg(feature = "preview")]
struct CropPreview {
    target: PreviewTarget,
    original: CropModifier,
    map: glam::Mat3,
    size: glam::Vec2,
    values: [f32; 4],
    pixels: bool,
    editable: [bool; 4],
    active: Option<CropDrag>,
    changed: bool,
}

impl CropModifier {
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
        let (pixels, edges) = match self {
            Self::Percentage(edges) => (false, edges),
            Self::Pixels(edges) => (true, edges),
        };
        Some(Box::new(CropPreview {
            target,
            original: self.clone(),
            map,
            size,
            values: [
                builder.resolve(&edges.top),
                builder.resolve(&edges.right),
                builder.resolve(&edges.bottom),
                builder.resolve(&edges.left),
            ],
            pixels,
            editable: [&edges.top, &edges.right, &edges.bottom, &edges.left].map(preview::editable),
            active: None,
            changed: false,
        }))
    }
}

#[cfg(feature = "preview")]
impl CropPreview {
    fn rect(&self) -> Rect {
        let [mut top, mut right, mut bottom, mut left] = if self.pixels {
            self.values
        } else {
            [
                self.size.y * self.values[0] / 100.0,
                self.size.x * self.values[1] / 100.0,
                self.size.y * self.values[2] / 100.0,
                self.size.x * self.values[3] / 100.0,
            ]
        };
        let maximum = self.size * 0.999_99;
        top = top.clamp(0.0, maximum.y);
        bottom = bottom.clamp(0.0, (maximum.y - top).max(0.0));
        right = right.clamp(0.0, maximum.x);
        left = left.clamp(0.0, (maximum.x - right).max(0.0));
        Rect {
            min: glam::Vec2::new(left, top),
            max: glam::Vec2::new(self.size.x - right, self.size.y - bottom),
        }
    }
    fn hit(&self, point: glam::Vec2) -> Option<CropDrag> {
        let rect = self.rect();
        HANDLES.into_iter().find_map(|handle| {
            let (x, y) = preview::handle_axes(handle);
            let editable = (x == 0 || self.editable[if x < 0 { 3 } else { 1 }])
                && (y == 0 || self.editable[if y < 0 { 0 } else { 2 }]);
            (editable
                && preview::hit(
                    point,
                    self.map
                        .transform_point2(preview::handle_point(handle, rect)),
                ))
            .then_some(CropDrag {
                map: self.map,
                size: self.size,
                values: self.values,
                pixels: self.pixels,
                handle,
            })
        })
    }
}

#[cfg(feature = "preview")]
impl PreviewProvider for CropPreview {
    fn on_draw(
        &mut self,
        painter: &shrimply_preview_provider_skia::PreviewCanvas,
        context: &dyn PreviewContext,
    ) {
        let color = context.selection_color();
        let rect = self.rect();
        preview::draw_rect(painter, self.map, rect, color);
        for handle in HANDLES {
            preview::draw_handle(
                painter,
                self.map
                    .transform_point2(preview::handle_point(handle, rect)),
                color,
            );
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
            PointerEvent::Hover(input) => self
                .hit(input.sample.position)
                .map_or(PreviewResponse::IGNORED, |drag| {
                    cursor(preview::resize_cursor(drag.handle))
                }),
            PointerEvent::Begin(input) => {
                self.active = self.hit(input.sample.position);
                self.active.map_or(PreviewResponse::IGNORED, |drag| {
                    cursor(preview::resize_cursor(drag.handle))
                })
            }
            PointerEvent::Samples { input, .. } => {
                let Some(drag) = self.active else {
                    return PreviewResponse::IGNORED;
                };
                let Some((changed, values)) = update(
                    edits
                        .target_mut(self.target)
                        .downcast_mut::<CropModifier>()
                        .expect("crop preview target has the wrong type"),
                    drag,
                    input.sample.position,
                    time,
                ) else {
                    return PreviewResponse::IGNORED;
                };
                if changed {
                    self.values = values;
                }
                self.changed |= changed;
                edited(changed, false)
            }
            PointerEvent::End(_) => {
                self.active = None;
                edited(std::mem::take(&mut self.changed), true)
            }
            PointerEvent::Cancel => {
                if self.changed {
                    *edits
                        .target_mut(self.target)
                        .downcast_mut::<CropModifier>()
                        .expect("crop preview target has the wrong type") = self.original.clone();
                }
                self.active = None;
                let changed = std::mem::take(&mut self.changed);
                edited(changed, false)
            }
            _ => PreviewResponse::IGNORED,
        }
    }
}

#[cfg(feature = "preview")]
fn update(
    crop: &mut CropModifier,
    drag: CropDrag,
    point: glam::Vec2,
    time: shrimply_property_model::Time,
) -> Option<(bool, [f32; 4])> {
    let local = preview::inverse_point(drag.map, point)?;
    let (x, y) = preview::handle_axes(drag.handle);
    let mut value = drag.values;
    if drag.pixels {
        if y < 0 {
            value[0] = local
                .y
                .clamp(0.0, (drag.size.y - value[2].max(0.0)).max(0.0));
        } else if y > 0 {
            value[2] =
                (drag.size.y - local.y).clamp(0.0, (drag.size.y - value[0].max(0.0)).max(0.0));
        }
        if x < 0 {
            value[3] = local
                .x
                .clamp(0.0, (drag.size.x - value[1].max(0.0)).max(0.0));
        } else if x > 0 {
            value[1] =
                (drag.size.x - local.x).clamp(0.0, (drag.size.x - value[3].max(0.0)).max(0.0));
        }
    } else {
        if y < 0 {
            value[0] = (local.y / drag.size.y.max(1.0) * 100.0)
                .clamp(0.0, (99.999 - value[2].max(0.0)).max(0.0));
        } else if y > 0 {
            value[2] = ((drag.size.y - local.y) / drag.size.y.max(1.0) * 100.0)
                .clamp(0.0, (99.999 - value[0].max(0.0)).max(0.0));
        }
        if x < 0 {
            value[3] = (local.x / drag.size.x.max(1.0) * 100.0)
                .clamp(0.0, (99.999 - value[1].max(0.0)).max(0.0));
        } else if x > 0 {
            value[1] = ((drag.size.x - local.x) / drag.size.x.max(1.0) * 100.0)
                .clamp(0.0, (99.999 - value[3].max(0.0)).max(0.0));
        }
    }
    let edges = crop.edges_mut();
    let changed = preview::set_scalar(&mut edges.top, time, value[0])
        | preview::set_scalar(&mut edges.right, time, value[1])
        | preview::set_scalar(&mut edges.bottom, time, value[2])
        | preview::set_scalar(&mut edges.left, time, value[3]);
    Some((changed, value))
}
#[cfg(feature = "preview")]
fn cursor(value: shrimply_preview_provider_skia::Cursor) -> PreviewResponse {
    PreviewResponse {
        handled: true,
        redraw: false,
        cursor: CursorUpdate::Set(value),
        edit: PreviewEditOutcome::UNCHANGED,
    }
}
#[cfg(feature = "preview")]
fn edited(changed: bool, commit: bool) -> PreviewResponse {
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
