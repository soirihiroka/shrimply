use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
};

use glam::Vec2;
use hashbrown::HashSet;
use serde::{Deserialize, Deserializer, Serialize};
use shrimply_property_model::{Time, timeline_value::TimelineValue};
use uuid::Uuid;

use super::{KeyframeSpan, ModifierModel, ensure_unique_id};
#[cfg(feature = "preview")]
use shrimply_preview_provider_skia::{
    Cursor, CursorUpdate, Modifiers, PointerButton, PointerEvent, PreviewBuilder, PreviewContext,
    PreviewEditOutcome, PreviewEditSink, PreviewProvider, PreviewRefresh, PreviewResponse,
    PreviewTarget, Rect,
};

#[cfg(feature = "preview")]
use super::preview;

#[cfg(feature = "preview")]
const BOX_DRAG_THRESHOLD: f32 = 4.0;

#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Eq,
    PartialEq,
    Serialize,
    Deserialize,
    strum::Display,
    strum::EnumIter,
)]
#[serde(rename_all = "snake_case")]
pub enum Sam2PointLabel {
    #[default]
    Foreground,
    Background,
}

#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Eq,
    Hash,
    PartialEq,
    Serialize,
    Deserialize,
    strum::Display,
    strum::EnumIter,
)]
#[serde(rename_all = "snake_case")]
pub enum Sam2Model {
    #[default]
    Tiny,
    Small,
    #[strum(to_string = "Base+")]
    BasePlus,
    Large,
}

impl Sam2Model {
    pub const fn directory_name(self) -> &'static str {
        match self {
            Self::Tiny => "tiny",
            Self::Small => "small",
            Self::BasePlus => "base_plus",
            Self::Large => "large",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sam2Point {
    #[serde(default = "Uuid::new_v4")]
    pub id: Uuid,
    #[serde(deserialize_with = "deserialize_point_position")]
    pub position: TimelineValue<Vec2>,
    #[serde(default)]
    pub label: Sam2PointLabel,
}

impl Sam2Point {
    pub fn new(position: Vec2, label: Sam2PointLabel) -> Self {
        Self {
            id: Uuid::new_v4(),
            position: TimelineValue::new_const(position.clamp(Vec2::ZERO, Vec2::ONE)),
            label,
        }
    }
}

fn deserialize_point_position<'de, D>(deserializer: D) -> Result<TimelineValue<Vec2>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StoredPosition {
        Timeline(TimelineValue<Vec2>),
        Legacy(Vec2),
    }

    Ok(match StoredPosition::deserialize(deserializer)? {
        StoredPosition::Timeline(value) => value,
        StoredPosition::Legacy(value) => TimelineValue::new_const(value),
    })
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Sam2Box {
    #[serde(default = "Uuid::new_v4")]
    pub id: Uuid,
    pub min: Vec2,
    pub max: Vec2,
}

impl Sam2Box {
    pub fn new(a: Vec2, b: Vec2) -> Self {
        Self {
            id: Uuid::new_v4(),
            min: a.min(b).clamp(Vec2::ZERO, Vec2::ONE),
            max: a.max(b).clamp(Vec2::ZERO, Vec2::ONE),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sam2Modifier {
    #[serde(default)]
    pub model: Sam2Model,
    #[serde(default)]
    pub seed_position: Option<Time>,
    #[serde(default)]
    pub points: Vec<Sam2Point>,
    #[serde(default)]
    pub box_prompt: Option<Sam2Box>,
    #[serde(default)]
    pub analysis_generation: u64,
    #[serde(default = "default_threshold")]
    pub threshold: TimelineValue<f32>,
    #[serde(default = "default_softness")]
    pub softness: TimelineValue<f32>,
    #[serde(default)]
    pub invert: bool,
}

fn default_threshold() -> TimelineValue<f32> {
    TimelineValue::new_const(0.0)
}

fn default_softness() -> TimelineValue<f32> {
    TimelineValue::new_const(0.0)
}

impl Default for Sam2Modifier {
    fn default() -> Self {
        Self {
            model: Sam2Model::default(),
            seed_position: None,
            points: Vec::new(),
            box_prompt: None,
            analysis_generation: 0,
            threshold: default_threshold(),
            softness: default_softness(),
            invert: false,
        }
    }
}

impl Sam2Modifier {
    pub fn prompt_signature(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        serde_json::to_string(&(
            self.model,
            self.seed_position,
            &self.points,
            self.box_prompt,
        ))
        .expect("serialize SAM2 prompts")
        .hash(&mut hasher);
        hasher.finish()
    }
}

impl ModifierModel for Sam2Modifier {
    fn display_name(&self) -> &'static str {
        "Segment Anything 2"
    }

    fn keywords(&self) -> &'static [&'static str] {
        &[
            "SAM2",
            "SAM 2",
            "AI mask",
            "segmentation",
            "object selection",
        ]
    }

    fn ensure_ids(&mut self, seen: &mut HashSet<Uuid>) {
        for point in &mut self.points {
            ensure_unique_id(&mut point.id, seen);
            super::ensure_timeline_value_ids(&mut point.position, seen);
        }
        super::ensure_timeline_value_ids(&mut self.threshold, seen);
        super::ensure_timeline_value_ids(&mut self.softness, seen);
        if let Some(box_prompt) = &mut self.box_prompt {
            ensure_unique_id(&mut box_prompt.id, seen);
            let min = box_prompt
                .min
                .min(box_prompt.max)
                .clamp(Vec2::ZERO, Vec2::ONE);
            let max = box_prompt
                .min
                .max(box_prompt.max)
                .clamp(Vec2::ZERO, Vec2::ONE);
            box_prompt.min = min;
            box_prompt.max = max;
        }
    }

    fn keyframe_span(&self) -> KeyframeSpan {
        super::combine(
            self.points
                .iter()
                .map(|point| super::timeline_value_span(&point.position))
                .chain([
                    super::timeline_value_span(&self.threshold),
                    super::timeline_value_span(&self.softness),
                ]),
        )
    }

    fn number(&self, id: Uuid) -> Option<&TimelineValue<f32>> {
        [&self.threshold, &self.softness]
            .into_iter()
            .find(|value| value.id == id)
    }

    fn number_mut(&mut self, id: Uuid) -> Option<&mut TimelineValue<f32>> {
        [&mut self.threshold, &mut self.softness]
            .into_iter()
            .find(|value| value.id == id)
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
struct Sam2Preview {
    target: PreviewTarget,
    snapshot: Sam2Modifier,
    item_map: glam::Mat3,
    item_size: Vec2,
    prompt_map: glam::Mat3,
    prompt_size: Vec2,
    points: Vec<(Uuid, Vec2, Sam2PointLabel, bool)>,
    box_prompt: Option<Sam2Box>,
    pending: Option<Sam2Pending>,
    active: Option<Sam2Drag>,
    changed: bool,
}

#[derive(Clone, Copy)]
#[cfg(feature = "preview")]
struct Sam2Pending {
    screen: Vec2,
    position: Vec2,
    label: Sam2PointLabel,
}

#[derive(Clone, Copy)]
#[cfg(feature = "preview")]
enum Sam2Drag {
    Point {
        map: glam::Mat3,
        size: Vec2,
        id: Uuid,
    },
    BoxCorner {
        map: glam::Mat3,
        size: Vec2,
        corner: usize,
        box_prompt: Sam2Box,
    },
}

impl Sam2Modifier {
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
        Some(Box::new(Sam2Preview {
            target,
            snapshot: self.clone(),
            item_map,
            item_size,
            prompt_map: viewport.canvas_to_screen,
            prompt_size: viewport.canvas_size,
            points: self
                .points
                .iter()
                .map(|point| {
                    (
                        point.id,
                        builder.resolve(&point.position),
                        point.label,
                        preview::editable(&point.position),
                    )
                })
                .collect(),
            box_prompt: self.box_prompt,
            pending: None,
            active: None,
            changed: false,
        }))
    }
}

#[cfg(feature = "preview")]
impl Sam2Preview {
    fn box_corners(box_prompt: Sam2Box) -> [Vec2; 4] {
        [
            box_prompt.min,
            Vec2::new(box_prompt.max.x, box_prompt.min.y),
            box_prompt.max,
            Vec2::new(box_prompt.min.x, box_prompt.max.y),
        ]
    }

    fn prompt_point(&self, point: Vec2) -> Option<Vec2> {
        let local = preview::inverse_point(self.item_map, point)?;
        if !local.cmpge(Vec2::ZERO).all() || !local.cmple(self.item_size).all() {
            return None;
        }
        self.clamped_prompt_point(point)
    }

    fn clamped_prompt_point(&self, point: Vec2) -> Option<Vec2> {
        preview::inverse_point(self.prompt_map, point)
            .map(|point| (point / self.prompt_size.max(Vec2::ONE)).clamp(Vec2::ZERO, Vec2::ONE))
    }
}

#[cfg(feature = "preview")]
impl PreviewProvider for Sam2Preview {
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
        if let Some(box_prompt) = self.box_prompt {
            let rect = Rect {
                min: box_prompt.min * self.prompt_size,
                max: box_prompt.max * self.prompt_size,
            };
            preview::draw_rect(painter, self.prompt_map, rect, context.selection_color());
            for point in Self::box_corners(box_prompt) {
                preview::draw_handle(
                    painter,
                    self.prompt_map.transform_point2(point * self.prompt_size),
                    context.selection_color(),
                );
            }
        }
        for (_, point, label, _) in &self.points {
            let color = if *label == Sam2PointLabel::Foreground {
                shrimply_preview_provider_skia::Color::new(0.21, 0.83, 0.34, 1.0)
            } else {
                shrimply_preview_provider_skia::Color::new(0.94, 0.27, 0.27, 1.0)
            };
            preview::draw_handle(
                painter,
                self.prompt_map.transform_point2(*point * self.prompt_size),
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
        let hit = |point| {
            for (id, position, _, editable) in &self.points {
                if *editable
                    && preview::hit(
                        point,
                        self.prompt_map
                            .transform_point2(*position * self.prompt_size),
                    )
                {
                    return Some(Sam2Drag::Point {
                        map: self.prompt_map,
                        size: self.prompt_size,
                        id: *id,
                    });
                }
            }
            if let Some(box_prompt) = self.box_prompt {
                for (corner, position) in Self::box_corners(box_prompt).into_iter().enumerate() {
                    if preview::hit(
                        point,
                        self.prompt_map
                            .transform_point2(position * self.prompt_size),
                    ) {
                        return Some(Sam2Drag::BoxCorner {
                            map: self.prompt_map,
                            size: self.prompt_size,
                            corner,
                            box_prompt,
                        });
                    }
                }
            }
            None
        };
        match event {
            PointerEvent::Hover(input) if hit(input.sample.position).is_some() => PreviewResponse {
                handled: true,
                redraw: false,
                cursor: CursorUpdate::Set(Cursor::Move),
                edit: PreviewEditOutcome::UNCHANGED,
            },
            PointerEvent::Hover(input) if self.prompt_point(input.sample.position).is_some() => {
                PreviewResponse {
                    handled: true,
                    redraw: false,
                    cursor: CursorUpdate::Set(Cursor::Crosshair),
                    edit: PreviewEditOutcome::UNCHANGED,
                }
            }
            PointerEvent::Begin(input) => {
                self.active = hit(input.sample.position);
                if self.active.is_some() {
                    return PreviewResponse::handled();
                }
                if !matches!(
                    input.button,
                    PointerButton::Primary | PointerButton::Secondary
                ) {
                    return PreviewResponse::IGNORED;
                }
                let Some(position) = self.prompt_point(input.sample.position) else {
                    return PreviewResponse::IGNORED;
                };
                let label = if input.button == PointerButton::Secondary
                    || input.modifiers.contains(Modifiers::CONTROL)
                {
                    Sam2PointLabel::Background
                } else {
                    Sam2PointLabel::Foreground
                };
                self.pending = Some(Sam2Pending {
                    screen: input.sample.position,
                    position,
                    label,
                });
                PreviewResponse::handled()
            }
            PointerEvent::Samples { input, .. } => {
                if let Some(pending) = self.pending
                    && input.sample.position.distance_squared(pending.screen)
                        >= BOX_DRAG_THRESHOLD * BOX_DRAG_THRESHOLD
                {
                    let Some(position) = self.clamped_prompt_point(input.sample.position) else {
                        return PreviewResponse::IGNORED;
                    };
                    self.pending = None;
                    let box_prompt = Sam2Box::new(pending.position, position);
                    let drag_box = Sam2Box {
                        id: box_prompt.id,
                        min: pending.position,
                        max: pending.position,
                    };
                    let modifier = edits
                        .target_mut(self.target)
                        .downcast_mut::<Sam2Modifier>()
                        .expect("SAM2 preview target has wrong type");
                    modifier.box_prompt = Some(box_prompt);
                    modifier
                        .seed_position
                        .get_or_insert(context.timeline_position());
                    self.box_prompt = Some(box_prompt);
                    self.active = Some(Sam2Drag::BoxCorner {
                        map: self.prompt_map,
                        size: self.prompt_size,
                        corner: 2,
                        box_prompt: drag_box,
                    });
                    self.changed = true;
                    return sam2_edit(true, false);
                }
                if self.pending.is_some() {
                    return PreviewResponse::handled();
                }
                let Some(drag) = self.active else {
                    return PreviewResponse::IGNORED;
                };
                let modifier = edits
                    .target_mut(self.target)
                    .downcast_mut::<Sam2Modifier>()
                    .expect("SAM2 preview target has wrong type");
                let changed = update_sam2(modifier, &drag, input.sample.position, time);
                if changed {
                    match drag {
                        Sam2Drag::Point { id, .. } => {
                            let position = modifier
                                .points
                                .iter()
                                .find(|point| point.id == id)
                                .expect("dragged SAM2 point is missing")
                                .position
                                .value_at(context.local_time());
                            self.points
                                .iter_mut()
                                .find(|(stored_id, ..)| *stored_id == id)
                                .expect("dragged SAM2 preview point is missing")
                                .1 = position;
                        }
                        Sam2Drag::BoxCorner { .. } => self.box_prompt = modifier.box_prompt,
                    }
                }
                self.changed |= changed;
                sam2_edit(changed, false)
            }
            PointerEvent::End(input) if self.pending.is_some() => {
                let pending = self.pending.take().expect("checked pending SAM2 prompt");
                if input.sample.position.distance_squared(pending.screen)
                    >= BOX_DRAG_THRESHOLD * BOX_DRAG_THRESHOLD
                {
                    let Some(position) = self.clamped_prompt_point(input.sample.position) else {
                        return PreviewResponse::IGNORED;
                    };
                    let box_prompt = Sam2Box::new(pending.position, position);
                    let modifier = edits
                        .target_mut(self.target)
                        .downcast_mut::<Sam2Modifier>()
                        .expect("SAM2 preview target has wrong type");
                    modifier.box_prompt = Some(box_prompt);
                    modifier
                        .seed_position
                        .get_or_insert(context.timeline_position());
                    self.box_prompt = Some(box_prompt);
                } else {
                    let point = Sam2Point::new(pending.position, pending.label);
                    let id = point.id;
                    let modifier = edits
                        .target_mut(self.target)
                        .downcast_mut::<Sam2Modifier>()
                        .expect("SAM2 preview target has wrong type");
                    modifier.points.push(point);
                    modifier
                        .seed_position
                        .get_or_insert(context.timeline_position());
                    self.points
                        .push((id, pending.position, pending.label, true));
                }
                sam2_edit(true, true)
            }
            PointerEvent::End(_) if self.active.is_some() => {
                self.active = None;
                sam2_edit(std::mem::take(&mut self.changed), true)
            }
            PointerEvent::Cancel => {
                if self.changed {
                    let modifier = edits
                        .target_mut(self.target)
                        .downcast_mut::<Sam2Modifier>()
                        .expect("SAM2 preview target has wrong type");
                    *modifier = self.snapshot.clone();
                    self.points.retain(|(id, ..)| {
                        self.snapshot.points.iter().any(|point| point.id == *id)
                    });
                    for (id, position, label, editable) in &mut self.points {
                        let point = self
                            .snapshot
                            .points
                            .iter()
                            .find(|point| point.id == *id)
                            .expect("restored SAM2 preview point is missing");
                        if *editable {
                            *position = point.position.value_at(context.local_time());
                        }
                        *label = point.label;
                    }
                    self.box_prompt = self.snapshot.box_prompt;
                }
                self.pending = None;
                self.active = None;
                sam2_edit(std::mem::take(&mut self.changed), false)
            }
            _ => PreviewResponse::IGNORED,
        }
    }
}

#[cfg(feature = "preview")]
fn sam2_edit(changed: bool, commit: bool) -> PreviewResponse {
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
fn update_sam2(modifier: &mut Sam2Modifier, drag: &Sam2Drag, point: Vec2, time: Time) -> bool {
    let (map, size) = match *drag {
        Sam2Drag::Point { map, size, .. } | Sam2Drag::BoxCorner { map, size, .. } => (map, size),
    };
    let Some(local) = preview::inverse_point(map, point) else {
        return false;
    };
    let position = (local / size.max(Vec2::ONE)).clamp(Vec2::ZERO, Vec2::ONE);
    match *drag {
        Sam2Drag::Point { id, .. } => modifier
            .points
            .iter_mut()
            .find(|point| point.id == id)
            .is_some_and(|point| preview::set_vec2(&mut point.position, time, position)),
        Sam2Drag::BoxCorner {
            corner, box_prompt, ..
        } => {
            let opposite = match corner {
                0 => box_prompt.max,
                1 => Vec2::new(box_prompt.min.x, box_prompt.max.y),
                2 => box_prompt.min,
                3 => Vec2::new(box_prompt.max.x, box_prompt.min.y),
                _ => unreachable!(),
            };
            let next = Sam2Box {
                id: box_prompt.id,
                min: opposite.min(position),
                max: opposite.max(position),
            };
            let changed = modifier
                .box_prompt
                .is_none_or(|current| current.min != next.min || current.max != next.max);
            modifier.box_prompt = Some(next);
            changed
        }
    }
}
