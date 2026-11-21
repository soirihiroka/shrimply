use hashbrown::HashSet;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{KeyframeSpan, ModifierModel, combine, ensure_timeline_value_ids, timeline_value_span};
#[cfg(feature = "preview")]
use shrimply_preview_provider_skia::{
    CursorUpdate, PointerEvent, PreviewBuilder, PreviewContext, PreviewEditOutcome,
    PreviewEditSink, PreviewProvider, PreviewRefresh, PreviewResponse, PreviewTarget, Rect,
};
use shrimply_property_model::{TextureAddressMode, VisualEdges, timeline_value::*};

#[cfg(feature = "preview")]
use super::preview::{self, HANDLES, Handle};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TextureBoundsModifier {
    pub edges: VisualEdges,
    #[serde(default, deserialize_with = "deserialize_timeline_value")]
    pub address_mode: TimelineValue<TextureAddressMode>,
}

impl ModifierModel for TextureBoundsModifier {
    fn display_name(&self) -> &'static str {
        "Texture bounds"
    }

    fn keywords(&self) -> &'static [&'static str] {
        &["UV", "texture coordinates", "mapping"]
    }

    fn ensure_ids(&mut self, seen: &mut HashSet<Uuid>) {
        ensure_timeline_value_ids(&mut self.edges.top, seen);
        ensure_timeline_value_ids(&mut self.edges.right, seen);
        ensure_timeline_value_ids(&mut self.edges.bottom, seen);
        ensure_timeline_value_ids(&mut self.edges.left, seen);
        ensure_timeline_value_ids(&mut self.address_mode, seen);
    }

    fn keyframe_span(&self) -> KeyframeSpan {
        combine([
            timeline_value_span(&self.edges.top),
            timeline_value_span(&self.edges.right),
            timeline_value_span(&self.edges.bottom),
            timeline_value_span(&self.edges.left),
            timeline_value_span(&self.address_mode),
        ])
    }

    fn number(
        &self,
        id: Uuid,
    ) -> Option<&shrimply_property_model::timeline_value::TimelineValue<f32>> {
        [
            &self.edges.top,
            &self.edges.right,
            &self.edges.bottom,
            &self.edges.left,
        ]
        .into_iter()
        .find(|value| value.id == id)
    }

    fn number_mut(
        &mut self,
        id: Uuid,
    ) -> Option<&mut shrimply_property_model::timeline_value::TimelineValue<f32>> {
        [
            &mut self.edges.top,
            &mut self.edges.right,
            &mut self.edges.bottom,
            &mut self.edges.left,
        ]
        .into_iter()
        .find(|value| value.id == id)
        .map(|value| &mut *value)
    }
}

#[derive(Clone)]
#[cfg(feature = "preview")]
struct TextureBoundsPreview {
    target: PreviewTarget,
    snapshot: TextureBoundsModifier,
    map: glam::Mat3,
    size: glam::Vec2,
    values: [f32; 4],
    editable: [bool; 4],
    active: Option<BoundsDrag>,
    changed: bool,
}

#[derive(Clone, Copy)]
#[cfg(feature = "preview")]
struct BoundsDrag {
    map: glam::Mat3,
    size: glam::Vec2,
    values: [f32; 4],
    handle: Handle,
}

impl TextureBoundsModifier {
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
        Some(Box::new(TextureBoundsPreview {
            target,
            snapshot: self.clone(),
            map,
            size,
            values: [
                builder.resolve(&self.edges.top),
                builder.resolve(&self.edges.right),
                builder.resolve(&self.edges.bottom),
                builder.resolve(&self.edges.left),
            ],
            editable: [
                preview::editable(&self.edges.top),
                preview::editable(&self.edges.right),
                preview::editable(&self.edges.bottom),
                preview::editable(&self.edges.left),
            ],
            active: None,
            changed: false,
        }))
    }
}

#[cfg(feature = "preview")]
impl TextureBoundsPreview {
    fn rect(&self) -> Rect {
        Rect {
            min: glam::Vec2::new(-self.values[3], -self.values[0]),
            max: self.size + glam::Vec2::new(self.values[1], self.values[2]),
        }
    }
}

#[cfg(feature = "preview")]
impl PreviewProvider for TextureBoundsPreview {
    fn on_draw(
        &mut self,
        painter: &shrimply_preview_provider_skia::PreviewCanvas,
        context: &dyn PreviewContext,
    ) {
        let rect = self.rect();
        let color = context.selection_color();
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
        let rect = self.rect();
        let hit = |point| {
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
                .then_some(BoundsDrag {
                    map: self.map,
                    size: self.size,
                    values: self.values,
                    handle,
                })
            })
        };
        match event {
            PointerEvent::Hover(input) => {
                let Some(drag) = hit(input.sample.position) else {
                    return PreviewResponse::IGNORED;
                };
                PreviewResponse {
                    handled: true,
                    redraw: false,
                    cursor: CursorUpdate::Set(preview::resize_cursor(drag.handle)),
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
                let Some((changed, values)) = update_bounds(
                    edits
                        .target_mut(self.target)
                        .downcast_mut::<TextureBoundsModifier>()
                        .expect("texture bounds preview target has wrong type"),
                    &drag,
                    input.sample.position,
                    time,
                ) else {
                    return PreviewResponse::IGNORED;
                };
                if changed {
                    self.values = values;
                }
                self.changed |= changed;
                bounds_edit(changed, false)
            }
            PointerEvent::End(_) if self.active.is_some() => {
                self.active = None;
                bounds_edit(std::mem::take(&mut self.changed), true)
            }
            PointerEvent::Cancel => {
                if self.changed {
                    *edits
                        .target_mut(self.target)
                        .downcast_mut::<TextureBoundsModifier>()
                        .expect("texture bounds preview target has wrong type") =
                        self.snapshot.clone();
                }
                self.active = None;
                bounds_edit(std::mem::take(&mut self.changed), false)
            }
            _ => PreviewResponse::IGNORED,
        }
    }
}

#[cfg(feature = "preview")]
fn bounds_edit(changed: bool, commit: bool) -> PreviewResponse {
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
fn update_bounds(
    modifier: &mut TextureBoundsModifier,
    drag: &BoundsDrag,
    point: glam::Vec2,
    time: shrimply_property_model::Time,
) -> Option<(bool, [f32; 4])> {
    let local = preview::inverse_point(drag.map, point)?;
    let (x, y) = preview::handle_axes(drag.handle);
    let mut values = drag.values;
    if y < 0 {
        values[0] = -local.y;
    }
    if x > 0 {
        values[1] = local.x - drag.size.x;
    }
    if y > 0 {
        values[2] = local.y - drag.size.y;
    }
    if x < 0 {
        values[3] = -local.x;
    }
    let changed = preview::set_scalar(&mut modifier.edges.top, time, values[0])
        | preview::set_scalar(&mut modifier.edges.right, time, values[1])
        | preview::set_scalar(&mut modifier.edges.bottom, time, values[2])
        | preview::set_scalar(&mut modifier.edges.left, time, values[3]);
    Some((changed, values))
}
