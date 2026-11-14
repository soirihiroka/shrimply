#[cfg(feature = "preview")]
use super::preview;
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
};

use glam::Vec2;
use hashbrown::HashSet;
use serde::{Deserialize, Serialize};
#[cfg(feature = "preview")]
use shrimply_preview_provider_skia::{
    Cursor, CursorUpdate, PointerButton, PointerEvent, PreviewBuilder, PreviewContext,
    PreviewEditOutcome, PreviewEditSink, PreviewProvider, PreviewRefresh, PreviewResponse,
    PreviewTarget, Rect,
};
use shrimply_property_model::timeline_value::TimelineValue;
use uuid::Uuid;

use super::{KeyframeSpan, ModifierModel, ensure_unique_id};

pub const DEFAULT_MAXIMUM_GAP: u32 = 100;
pub const MAXIMUM_GAP: u32 = 1_000;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TransparentFillPoint {
    #[serde(default = "Uuid::new_v4")]
    pub id: Uuid,
    pub position: TimelineValue<Vec2>,
}

impl TransparentFillPoint {
    #[cfg(feature = "preview")]
    fn new(position: Vec2) -> Self {
        Self {
            id: Uuid::new_v4(),
            position: TimelineValue::new_const(position.clamp(Vec2::ZERO, Vec2::ONE)),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TransparentFillModifier {
    #[serde(default)]
    pub points: Vec<TransparentFillPoint>,
    #[serde(default = "default_tolerance")]
    pub tolerance: TimelineValue<f32>,
    #[serde(default = "default_maximum_gap")]
    pub maximum_gap: u32,
    #[serde(default)]
    pub analysis_generation: u64,
}

fn default_tolerance() -> TimelineValue<f32> {
    TimelineValue::new_const(0.1)
}

const fn default_maximum_gap() -> u32 {
    DEFAULT_MAXIMUM_GAP
}

impl Default for TransparentFillModifier {
    fn default() -> Self {
        Self {
            points: Vec::new(),
            tolerance: default_tolerance(),
            maximum_gap: DEFAULT_MAXIMUM_GAP,
            analysis_generation: 0,
        }
    }
}

impl TransparentFillModifier {
    pub fn prompt_signature(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        serde_json::to_string(&(&self.points, &self.tolerance, self.maximum_gap))
            .expect("serialize transparent fill prompts")
            .hash(&mut hasher);
        hasher.finish()
    }
}

impl ModifierModel for TransparentFillModifier {
    fn display_name(&self) -> &'static str {
        "Transparent Fill"
    }

    fn keywords(&self) -> &'static [&'static str] {
        &["bucket fill", "flood fill", "transparency", "remove color"]
    }

    fn ensure_ids(&mut self, seen: &mut HashSet<Uuid>) {
        for point in &mut self.points {
            ensure_unique_id(&mut point.id, seen);
            super::ensure_timeline_value_ids(&mut point.position, seen);
        }
        super::ensure_timeline_value_ids(&mut self.tolerance, seen);
        self.maximum_gap = self.maximum_gap.min(MAXIMUM_GAP);
    }

    fn keyframe_span(&self) -> KeyframeSpan {
        super::combine(
            self.points
                .iter()
                .map(|point| super::timeline_value_span(&point.position))
                .chain(std::iter::once(super::timeline_value_span(&self.tolerance))),
        )
    }

    fn number(&self, id: Uuid) -> Option<&TimelineValue<f32>> {
        (self.tolerance.id == id).then_some(&self.tolerance)
    }

    fn number_mut(&mut self, id: Uuid) -> Option<&mut TimelineValue<f32>> {
        (self.tolerance.id == id).then_some(&mut self.tolerance)
    }

    fn number2(&self, id: Uuid) -> Option<&TimelineValue<Vec2>> {
        self.points
            .iter()
            .map(|point| &point.position)
            .find(|position| position.id == id)
    }

    fn number2_mut(&mut self, id: Uuid) -> Option<&mut TimelineValue<Vec2>> {
        self.points
            .iter_mut()
            .map(|point| &mut point.position)
            .find(|position| position.id == id)
    }
}

#[derive(Clone)]
#[cfg(feature = "preview")]
struct TransparentFillPreview {
    target: PreviewTarget,
    snapshot: TransparentFillModifier,
    item_map: glam::Mat3,
    item_size: Vec2,
    canvas_map: glam::Mat3,
    canvas_size: Vec2,
    points: Vec<(Uuid, Vec2, bool)>,
    active: Option<Uuid>,
    changed: bool,
}

impl TransparentFillModifier {
    #[cfg(feature = "preview")]
    pub(crate) fn preview_provider(
        &self,
        target: PreviewTarget,
        builder: &impl PreviewBuilder,
    ) -> Option<Box<dyn PreviewProvider>> {
        if !preview::is_target(target) {
            return None;
        }
        let (item_map, item_size) = preview::screen_map(target, builder)?;
        let viewport = builder.viewport();
        Some(Box::new(TransparentFillPreview {
            target,
            snapshot: self.clone(),
            item_map,
            item_size,
            canvas_map: viewport.canvas_to_screen,
            canvas_size: viewport.canvas_size,
            points: self
                .points
                .iter()
                .map(|point| {
                    (
                        point.id,
                        builder.resolve(&point.position),
                        preview::editable(&point.position),
                    )
                })
                .collect(),
            active: None,
            changed: false,
        }))
    }
}

#[cfg(feature = "preview")]
impl TransparentFillPreview {
    fn point(&self, screen: Vec2) -> Option<Vec2> {
        let item = preview::inverse_point(self.item_map, screen)?;
        if !item.cmpge(Vec2::ZERO).all() || !item.cmple(self.item_size).all() {
            return None;
        }
        preview::inverse_point(self.canvas_map, screen)
            .map(|point| (point / self.canvas_size.max(Vec2::ONE)).clamp(Vec2::ZERO, Vec2::ONE))
    }

    fn hit(&self, screen: Vec2) -> Option<Uuid> {
        self.points.iter().find_map(|(id, position, editable)| {
            (*editable
                && preview::hit(
                    screen,
                    self.canvas_map
                        .transform_point2(*position * self.canvas_size),
                ))
            .then_some(*id)
        })
    }
}

#[cfg(feature = "preview")]
impl PreviewProvider for TransparentFillPreview {
    fn on_draw(
        &mut self,
        painter: &shrimply_preview_provider_skia::PreviewCanvas,
        context: &dyn PreviewContext,
    ) {
        preview::draw_rect(
            painter,
            self.item_map,
            Rect::from_min_size(Vec2::ZERO, self.item_size),
            context.selection_color(),
        );
        let color = shrimply_preview_provider_skia::Color::new(0.94, 0.27, 0.27, 1.0);
        for (_, point, _) in &self.points {
            preview::draw_handle(
                painter,
                self.canvas_map.transform_point2(*point * self.canvas_size),
                color,
            );
        }
    }

    fn on_pointer(
        &mut self,
        event: PointerEvent<'_>,
        context: &dyn PreviewContext,
        edits: &mut dyn PreviewEditSink,
    ) -> PreviewResponse {
        let time = edits.keyframe_time();
        match event {
            PointerEvent::Hover(input) if self.hit(input.sample.position).is_some() => {
                PreviewResponse {
                    handled: true,
                    redraw: false,
                    cursor: CursorUpdate::Set(Cursor::Move),
                    edit: PreviewEditOutcome::UNCHANGED,
                }
            }
            PointerEvent::Hover(input) if self.point(input.sample.position).is_some() => {
                PreviewResponse {
                    handled: true,
                    redraw: false,
                    cursor: CursorUpdate::Set(Cursor::Crosshair),
                    edit: PreviewEditOutcome::UNCHANGED,
                }
            }
            PointerEvent::Begin(input) if input.button == PointerButton::Primary => {
                if let Some(id) = self.hit(input.sample.position) {
                    self.active = Some(id);
                    return PreviewResponse::handled();
                }
                let Some(position) = self.point(input.sample.position) else {
                    return PreviewResponse::IGNORED;
                };
                let point = TransparentFillPoint::new(position);
                let id = point.id;
                edits
                    .target_mut(self.target)
                    .downcast_mut::<TransparentFillModifier>()
                    .expect("transparent fill preview target has wrong type")
                    .points
                    .push(point);
                self.points.push((id, position, true));
                self.changed = true;
                fill_edit(true, true)
            }
            PointerEvent::Samples { input, .. } => {
                let Some(id) = self.active else {
                    return PreviewResponse::IGNORED;
                };
                let Some(position) = preview::inverse_point(self.canvas_map, input.sample.position)
                    .map(|point| {
                        (point / self.canvas_size.max(Vec2::ONE)).clamp(Vec2::ZERO, Vec2::ONE)
                    })
                else {
                    return PreviewResponse::IGNORED;
                };
                let modifier = edits
                    .target_mut(self.target)
                    .downcast_mut::<TransparentFillModifier>()
                    .expect("transparent fill preview target has wrong type");
                let Some(point) = modifier.points.iter_mut().find(|point| point.id == id) else {
                    return PreviewResponse::IGNORED;
                };
                let changed = preview::set_vec2(&mut point.position, time, position);
                if changed {
                    self.points
                        .iter_mut()
                        .find(|(stored, ..)| *stored == id)
                        .expect("transparent fill preview point is missing")
                        .1 = position;
                    self.changed = true;
                }
                fill_edit(changed, false)
            }
            PointerEvent::End(_) if self.active.take().is_some() => {
                fill_edit(std::mem::take(&mut self.changed), true)
            }
            PointerEvent::Cancel => {
                if self.changed {
                    *edits
                        .target_mut(self.target)
                        .downcast_mut::<TransparentFillModifier>()
                        .expect("transparent fill preview target has wrong type") =
                        self.snapshot.clone();
                    self.points = self
                        .snapshot
                        .points
                        .iter()
                        .map(|point| {
                            (
                                point.id,
                                point.position.value_at(context.local_time()),
                                preview::editable(&point.position),
                            )
                        })
                        .collect();
                }
                self.active = None;
                fill_edit(std::mem::take(&mut self.changed), false)
            }
            _ => PreviewResponse::IGNORED,
        }
    }
}

#[cfg(feature = "preview")]
fn fill_edit(changed: bool, commit: bool) -> PreviewResponse {
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
