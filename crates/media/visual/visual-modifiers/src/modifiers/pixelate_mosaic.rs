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
pub struct PixelateMosaicModifier {
    pub block_width: TimelineValue<f32>,
    pub block_height: TimelineValue<f32>,
}

#[cfg(feature = "preview")]
const WIDTH_COLOR: Color = Color::new(0.35, 0.82, 1.0, 1.0);
#[cfg(feature = "preview")]
const HEIGHT_COLOR: Color = Color::new(1.0, 0.82, 0.24, 1.0);
#[derive(Clone, Copy)]
#[cfg(feature = "preview")]
enum MosaicControl {
    Width,
    Height,
}
#[cfg(feature = "preview")]
struct MosaicPreview {
    target: PreviewTarget,
    original: PixelateMosaicModifier,
    map: glam::Mat3,
    size: glam::Vec2,
    values: glam::Vec2,
    editable: [bool; 2],
    active: Option<MosaicControl>,
    changed: bool,
}
#[cfg(feature = "preview")]
impl PixelateMosaicModifier {
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
            super::preview::editable(&self.block_width),
            super::preview::editable(&self.block_height),
        ];
        if !editable.iter().any(|value| *value) {
            return None;
        }
        let (map, size) = super::preview::screen_map(target, builder)?;
        Some(Box::new(MosaicPreview {
            target,
            original: self.clone(),
            map,
            size,
            values: glam::Vec2::new(
                builder.resolve(&self.block_width),
                builder.resolve(&self.block_height),
            ),
            editable,
            active: None,
            changed: false,
        }))
    }
}
#[cfg(feature = "preview")]
impl MosaicPreview {
    fn points(&self) -> (glam::Vec2, [glam::Vec2; 2]) {
        let center = self.size * 0.5;
        (
            self.map.transform_point2(center),
            [
                self.map
                    .transform_point2(center + glam::Vec2::X * self.values.x.max(1.0)),
                self.map
                    .transform_point2(center + glam::Vec2::Y * self.values.y.max(1.0)),
            ],
        )
    }
    fn hit(&self, point: glam::Vec2) -> Option<MosaicControl> {
        let (_, handles) = self.points();
        if self.editable[0] && super::preview::hit(point, handles[0]) {
            Some(MosaicControl::Width)
        } else if self.editable[1] && super::preview::hit(point, handles[1]) {
            Some(MosaicControl::Height)
        } else {
            None
        }
    }
}
#[cfg(feature = "preview")]
impl PreviewProvider for MosaicPreview {
    fn on_draw(
        &mut self,
        painter: &shrimply_preview_provider_skia::PreviewCanvas,
        _context: &dyn PreviewContext,
    ) {
        let (center, handles) = self.points();
        for (handle, color) in [(handles[0], WIDTH_COLOR), (handles[1], HEIGHT_COLOR)] {
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
                let cursor = if matches!(control, MosaicControl::Width) {
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
                    .downcast_mut::<PixelateMosaicModifier>()
                    .expect("mosaic preview target has wrong type");
                let value = match control {
                    MosaicControl::Width => (local.x - self.size.x * 0.5).max(1.0),
                    MosaicControl::Height => (local.y - self.size.y * 0.5).max(1.0),
                };
                let changed = match control {
                    MosaicControl::Width => {
                        super::preview::set_scalar(&mut owner.block_width, time, value)
                    }
                    MosaicControl::Height => {
                        super::preview::set_scalar(&mut owner.block_height, time, value)
                    }
                };
                if changed {
                    match control {
                        MosaicControl::Width => self.values.x = value,
                        MosaicControl::Height => self.values.y = value,
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
                        .downcast_mut::<PixelateMosaicModifier>()
                        .expect("mosaic preview target has wrong type") = self.original.clone();
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

impl Default for PixelateMosaicModifier {
    fn default() -> Self {
        Self {
            block_width: TimelineValue::<f32>::new_const(16.0),
            block_height: TimelineValue::<f32>::new_const(16.0),
        }
    }
}

impl ModifierModel for PixelateMosaicModifier {
    fn display_name(&self) -> &'static str {
        "Mosaic"
    }

    fn keywords(&self) -> &'static [&'static str] {
        &["pixelate", "pixels", "blocky", "censor"]
    }

    fn ensure_ids(&mut self, seen: &mut HashSet<Uuid>) {
        ensure_timeline_value_ids(&mut self.block_width, seen);
        ensure_timeline_value_ids(&mut self.block_height, seen);
    }

    fn keyframe_span(&self) -> KeyframeSpan {
        combine([
            timeline_value_span(&self.block_width),
            timeline_value_span(&self.block_height),
        ])
    }

    fn number(&self, id: Uuid) -> Option<&TimelineValue<f32>> {
        [&self.block_width, &self.block_height]
            .into_iter()
            .find(|value| value.id == id)
    }

    fn number_mut(&mut self, id: Uuid) -> Option<&mut TimelineValue<f32>> {
        [&mut self.block_width, &mut self.block_height]
            .into_iter()
            .find(|value| value.id == id)
    }
}
