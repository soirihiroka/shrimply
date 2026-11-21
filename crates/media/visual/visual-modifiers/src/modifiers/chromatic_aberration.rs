#[cfg(feature = "preview")]
use super::preview;
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
pub struct ChromaticAberrationModifier {
    pub red_offset_x: TimelineValue<f32>,
    pub red_offset_y: TimelineValue<f32>,
    pub blue_offset_x: TimelineValue<f32>,
    pub blue_offset_y: TimelineValue<f32>,
}

#[derive(Clone)]
#[cfg(feature = "preview")]
struct ChromaticPreview {
    target: PreviewTarget,
    snapshot: ChromaticAberrationModifier,
    map: glam::Mat3,
    center: glam::Vec2,
    offsets: [glam::Vec2; 2],
    editable: [bool; 4],
    active: Option<ChromaticDrag>,
    changed: bool,
}
#[derive(Clone, Copy)]
#[cfg(feature = "preview")]
struct ChromaticDrag {
    map: glam::Mat3,
    center: glam::Vec2,
    red: bool,
}

impl ChromaticAberrationModifier {
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
        Some(Box::new(ChromaticPreview {
            target,
            snapshot: self.clone(),
            map,
            center: size * 0.5,
            offsets: [
                glam::Vec2::new(
                    builder.resolve(&self.red_offset_x),
                    builder.resolve(&self.red_offset_y),
                ),
                glam::Vec2::new(
                    builder.resolve(&self.blue_offset_x),
                    builder.resolve(&self.blue_offset_y),
                ),
            ],
            editable: [
                preview::editable(&self.red_offset_x),
                preview::editable(&self.red_offset_y),
                preview::editable(&self.blue_offset_x),
                preview::editable(&self.blue_offset_y),
            ],
            active: None,
            changed: false,
        }))
    }
}

#[cfg(feature = "preview")]
impl PreviewProvider for ChromaticPreview {
    fn on_draw(
        &mut self,
        painter: &shrimply_preview_provider_skia::PreviewCanvas,
        _context: &dyn PreviewContext,
    ) {
        let center = self.map.transform_point2(self.center);
        for (offset, color) in [
            (
                self.offsets[0],
                shrimply_preview_provider_skia::Color::new(1.0, 0.27, 0.27, 1.0),
            ),
            (
                self.offsets[1],
                shrimply_preview_provider_skia::Color::new(0.31, 0.51, 1.0, 1.0),
            ),
        ] {
            let endpoint = self.map.transform_point2(self.center + offset);
            preview::draw_line(painter, center, endpoint, color);
            preview::draw_handle(painter, endpoint, color);
        }
    }
    fn on_pointer(
        &mut self,
        event: PointerEvent<'_>,
        _context: &dyn PreviewContext,
        edits: &mut dyn PreviewEditSink,
    ) -> PreviewResponse {
        let time = edits.keyframe_time();
        let hit = |point| {
            [true, false]
                .into_iter()
                .enumerate()
                .find_map(|(index, red)| {
                    (self.editable[index * 2..index * 2 + 2]
                        .iter()
                        .any(|value| *value)
                        && preview::hit(
                            point,
                            self.map.transform_point2(self.center + self.offsets[index]),
                        ))
                    .then_some(ChromaticDrag {
                        map: self.map,
                        center: self.center,
                        red,
                    })
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
                let Some((changed, red, value)) = update_chromatic(
                    edits
                        .target_mut(self.target)
                        .downcast_mut::<ChromaticAberrationModifier>()
                        .expect("chromatic preview target has wrong type"),
                    &drag,
                    input.sample.position,
                    time,
                ) else {
                    return PreviewResponse::IGNORED;
                };
                if changed {
                    self.offsets[usize::from(!red)] = value;
                }
                self.changed |= changed;
                chromatic_edit(changed, false)
            }
            PointerEvent::End(_) if self.active.is_some() => {
                self.active = None;
                chromatic_edit(std::mem::take(&mut self.changed), true)
            }
            PointerEvent::Cancel => {
                if self.changed {
                    *edits
                        .target_mut(self.target)
                        .downcast_mut::<ChromaticAberrationModifier>()
                        .expect("chromatic preview target has wrong type") = self.snapshot.clone();
                }
                self.active = None;
                chromatic_edit(std::mem::take(&mut self.changed), false)
            }
            _ => PreviewResponse::IGNORED,
        }
    }
}

#[cfg(feature = "preview")]
fn chromatic_edit(changed: bool, commit: bool) -> PreviewResponse {
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
fn update_chromatic(
    modifier: &mut ChromaticAberrationModifier,
    drag: &ChromaticDrag,
    point: glam::Vec2,
    time: shrimply_property_model::Time,
) -> Option<(bool, bool, glam::Vec2)> {
    let local = preview::inverse_point(drag.map, point)?;
    let value = local - drag.center;
    let changed = if drag.red {
        preview::set_scalar(&mut modifier.red_offset_x, time, value.x)
            | preview::set_scalar(&mut modifier.red_offset_y, time, value.y)
    } else {
        preview::set_scalar(&mut modifier.blue_offset_x, time, value.x)
            | preview::set_scalar(&mut modifier.blue_offset_y, time, value.y)
    };
    Some((changed, drag.red, value))
}

impl Default for ChromaticAberrationModifier {
    fn default() -> Self {
        Self {
            red_offset_x: TimelineValue::<f32>::new_const(-2.0),
            red_offset_y: TimelineValue::<f32>::new_const(0.0),
            blue_offset_x: TimelineValue::<f32>::new_const(2.0),
            blue_offset_y: TimelineValue::<f32>::new_const(0.0),
        }
    }
}

impl ModifierModel for ChromaticAberrationModifier {
    fn display_name(&self) -> &'static str {
        "Chromatic aberration"
    }

    fn keywords(&self) -> &'static [&'static str] {
        &["RGB split", "color fringe", "colour fringe", "glitch"]
    }

    fn ensure_ids(&mut self, seen: &mut HashSet<Uuid>) {
        ensure_timeline_value_ids(&mut self.red_offset_x, seen);
        ensure_timeline_value_ids(&mut self.red_offset_y, seen);
        ensure_timeline_value_ids(&mut self.blue_offset_x, seen);
        ensure_timeline_value_ids(&mut self.blue_offset_y, seen);
    }

    fn keyframe_span(&self) -> KeyframeSpan {
        combine([
            timeline_value_span(&self.red_offset_x),
            timeline_value_span(&self.red_offset_y),
            timeline_value_span(&self.blue_offset_x),
            timeline_value_span(&self.blue_offset_y),
        ])
    }

    fn number(&self, id: Uuid) -> Option<&TimelineValue<f32>> {
        [
            &self.red_offset_x,
            &self.red_offset_y,
            &self.blue_offset_x,
            &self.blue_offset_y,
        ]
        .into_iter()
        .find(|value| value.id == id)
    }

    fn number_mut(&mut self, id: Uuid) -> Option<&mut TimelineValue<f32>> {
        [
            &mut self.red_offset_x,
            &mut self.red_offset_y,
            &mut self.blue_offset_x,
            &mut self.blue_offset_y,
        ]
        .into_iter()
        .find(|value| value.id == id)
    }
}
