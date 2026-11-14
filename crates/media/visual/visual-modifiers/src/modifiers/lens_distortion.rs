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
pub struct LensDistortionModifier {
    pub distortion: TimelineValue<f32>,
    pub center: TimelineValue<glam::Vec2>,
}

#[cfg(feature = "preview")]
struct LensDistortionPreview {
    target: PreviewTarget,
    original: LensDistortionModifier,
    map: glam::Mat3,
    size: glam::Vec2,
    point: glam::Vec2,
    active: bool,
    changed: bool,
}
#[cfg(feature = "preview")]
impl LensDistortionModifier {
    #[cfg(feature = "preview")]
    pub(crate) fn preview_provider(
        &self,
        target: PreviewTarget,
        builder: &impl PreviewBuilder,
    ) -> Option<Box<dyn PreviewProvider>> {
        if !super::preview::is_target(target) || !super::preview::editable(&self.center) {
            return None;
        }
        let (map, size) = super::preview::screen_map(target, builder)?;
        Some(Box::new(LensDistortionPreview {
            target,
            original: self.clone(),
            map,
            size,
            point: builder.resolve(&self.center),
            active: false,
            changed: false,
        }))
    }
}
#[cfg(feature = "preview")]
impl PreviewProvider for LensDistortionPreview {
    fn on_draw(
        &mut self,
        painter: &shrimply_preview_provider_skia::PreviewCanvas,
        context: &dyn PreviewContext,
    ) {
        super::preview::draw_handle(
            painter,
            self.map.transform_point2(self.point * self.size),
            context.selection_color(),
        );
    }
    fn on_pointer(
        &mut self,
        event: PointerEvent<'_>,
        _context: &dyn PreviewContext,
        edits: &mut dyn PreviewEditSink,
    ) -> PreviewResponse {
        let time = edits.keyframe_time();
        let handle = self.map.transform_point2(self.point * self.size);
        match event {
            PointerEvent::Hover(input) if super::preview::hit(input.sample.position, handle) => {
                PreviewResponse {
                    handled: true,
                    redraw: false,
                    cursor: CursorUpdate::Set(Cursor::Move),
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
                let point = (local / self.size.max(glam::Vec2::ONE))
                    .clamp(glam::Vec2::ZERO, glam::Vec2::ONE);
                let changed = super::preview::set_vec2(
                    &mut edits
                        .target_mut(self.target)
                        .downcast_mut::<LensDistortionModifier>()
                        .expect("lens distortion preview target has wrong type")
                        .center,
                    time,
                    point,
                );
                if changed {
                    self.point = point;
                }
                self.changed |= changed;
                point_edit(changed, false)
            }
            PointerEvent::End(_) if self.active => {
                self.active = false;
                point_edit(std::mem::take(&mut self.changed), true)
            }
            PointerEvent::Cancel => {
                if self.changed {
                    *edits
                        .target_mut(self.target)
                        .downcast_mut::<LensDistortionModifier>()
                        .expect("lens distortion preview target has wrong type") =
                        self.original.clone();
                }
                self.active = false;
                point_edit(std::mem::take(&mut self.changed), false)
            }
            _ => PreviewResponse::IGNORED,
        }
    }
}
#[cfg(feature = "preview")]
fn point_edit(changed: bool, commit: bool) -> PreviewResponse {
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

impl Default for LensDistortionModifier {
    fn default() -> Self {
        Self {
            distortion: TimelineValue::<f32>::new_const(0.2),
            center: TimelineValue::<glam::Vec2>::new_const(glam::Vec2::splat(0.5)),
        }
    }
}

impl ModifierModel for LensDistortionModifier {
    fn display_name(&self) -> &'static str {
        "Lens distortion"
    }

    fn keywords(&self) -> &'static [&'static str] {
        &["barrel", "pincushion", "warp", "optics"]
    }

    fn ensure_ids(&mut self, seen: &mut HashSet<Uuid>) {
        ensure_timeline_value_ids(&mut self.distortion, seen);
        ensure_timeline_value_ids(&mut self.center, seen);
    }

    fn keyframe_span(&self) -> KeyframeSpan {
        combine([
            timeline_value_span(&self.distortion),
            timeline_value_span(&self.center),
        ])
    }

    fn number(&self, id: Uuid) -> Option<&TimelineValue<f32>> {
        [&self.distortion].into_iter().find(|value| value.id == id)
    }

    fn number_mut(&mut self, id: Uuid) -> Option<&mut TimelineValue<f32>> {
        [&mut self.distortion]
            .into_iter()
            .find(|value| value.id == id)
    }

    fn number2(&self, id: Uuid) -> Option<&TimelineValue<glam::Vec2>> {
        [&self.center].into_iter().find(|value| value.id == id)
    }

    fn number2_mut(&mut self, id: Uuid) -> Option<&mut TimelineValue<glam::Vec2>> {
        [&mut self.center].into_iter().find(|value| value.id == id)
    }
}
