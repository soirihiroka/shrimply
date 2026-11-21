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
pub struct ScanlinesCrtModifier {
    pub spacing: TimelineValue<f32>,
    pub intensity: TimelineValue<f32>,
    pub curvature: TimelineValue<f32>,
    pub mask_strength: TimelineValue<f32>,
}

#[cfg(feature = "preview")]
const SPACING_COLOR: Color = Color::new(0.35, 0.82, 1.0, 1.0);
#[cfg(feature = "preview")]
const CURVATURE_COLOR: Color = Color::new(1.0, 0.82, 0.24, 1.0);
#[cfg(feature = "preview")]
const CURVATURE_SCALE: f32 = 0.25;
#[derive(Clone, Copy)]
#[cfg(feature = "preview")]
enum CrtControl {
    Spacing,
    Curvature,
}
#[cfg(feature = "preview")]
struct CrtPreview {
    target: PreviewTarget,
    original: ScanlinesCrtModifier,
    map: glam::Mat3,
    size: glam::Vec2,
    values: glam::Vec2,
    editable: [bool; 2],
    active: Option<CrtControl>,
    changed: bool,
}
#[cfg(feature = "preview")]
impl ScanlinesCrtModifier {
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
            super::preview::editable(&self.spacing),
            super::preview::editable(&self.curvature),
        ];
        if !editable.iter().any(|value| *value) {
            return None;
        }
        let (map, size) = super::preview::screen_map(target, builder)?;
        Some(Box::new(CrtPreview {
            target,
            original: self.clone(),
            map,
            size,
            values: glam::Vec2::new(
                builder.resolve(&self.spacing),
                builder.resolve(&self.curvature),
            ),
            editable,
            active: None,
            changed: false,
        }))
    }
}
#[cfg(feature = "preview")]
impl CrtPreview {
    fn points(&self) -> (glam::Vec2, [glam::Vec2; 2]) {
        let center = self.size * 0.5;
        (
            self.map.transform_point2(center),
            [
                self.map
                    .transform_point2(center + glam::Vec2::X * self.values.x.max(1.0)),
                self.map.transform_point2(
                    center + glam::Vec2::Y * self.values.y * self.size.y * CURVATURE_SCALE,
                ),
            ],
        )
    }
    fn hit(&self, point: glam::Vec2) -> Option<CrtControl> {
        let (_, handles) = self.points();
        if self.editable[0] && super::preview::hit(point, handles[0]) {
            Some(CrtControl::Spacing)
        } else if self.editable[1] && super::preview::hit(point, handles[1]) {
            Some(CrtControl::Curvature)
        } else {
            None
        }
    }
}
#[cfg(feature = "preview")]
impl PreviewProvider for CrtPreview {
    fn on_draw(
        &mut self,
        painter: &shrimply_preview_provider_skia::PreviewCanvas,
        _context: &dyn PreviewContext,
    ) {
        let (center, handles) = self.points();
        for (handle, color) in [(handles[0], SPACING_COLOR), (handles[1], CURVATURE_COLOR)] {
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
                let cursor = if matches!(control, CrtControl::Spacing) {
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
                    .downcast_mut::<ScanlinesCrtModifier>()
                    .expect("CRT preview target has wrong type");
                let (index, value) = match control {
                    CrtControl::Spacing => (0, (local.x - self.size.x * 0.5).max(1.0)),
                    CrtControl::Curvature => (
                        1,
                        (local.y - self.size.y * 0.5) / (self.size.y * CURVATURE_SCALE).max(1.0),
                    ),
                };
                let changed = match control {
                    CrtControl::Spacing => {
                        super::preview::set_scalar(&mut owner.spacing, time, value)
                    }
                    CrtControl::Curvature => {
                        super::preview::set_scalar(&mut owner.curvature, time, value)
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
                        .downcast_mut::<ScanlinesCrtModifier>()
                        .expect("CRT preview target has wrong type") = self.original.clone();
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

impl Default for ScanlinesCrtModifier {
    fn default() -> Self {
        Self {
            spacing: TimelineValue::<f32>::new_const(4.0),
            intensity: TimelineValue::<f32>::new_const(0.3),
            curvature: TimelineValue::<f32>::new_const(0.0),
            mask_strength: TimelineValue::<f32>::new_const(0.25),
        }
    }
}

impl ModifierModel for ScanlinesCrtModifier {
    fn display_name(&self) -> &'static str {
        "CRT"
    }

    fn keywords(&self) -> &'static [&'static str] {
        &["scanlines", "television", "TV", "monitor", "retro", "VHS"]
    }

    fn ensure_ids(&mut self, seen: &mut HashSet<Uuid>) {
        ensure_timeline_value_ids(&mut self.spacing, seen);
        ensure_timeline_value_ids(&mut self.intensity, seen);
        ensure_timeline_value_ids(&mut self.curvature, seen);
        ensure_timeline_value_ids(&mut self.mask_strength, seen);
    }

    fn keyframe_span(&self) -> KeyframeSpan {
        combine([
            timeline_value_span(&self.spacing),
            timeline_value_span(&self.intensity),
            timeline_value_span(&self.curvature),
            timeline_value_span(&self.mask_strength),
        ])
    }

    fn number(&self, id: Uuid) -> Option<&TimelineValue<f32>> {
        [
            &self.spacing,
            &self.intensity,
            &self.curvature,
            &self.mask_strength,
        ]
        .into_iter()
        .find(|value| value.id == id)
    }

    fn number_mut(&mut self, id: Uuid) -> Option<&mut TimelineValue<f32>> {
        [
            &mut self.spacing,
            &mut self.intensity,
            &mut self.curvature,
            &mut self.mask_strength,
        ]
        .into_iter()
        .find(|value| value.id == id)
    }
}
