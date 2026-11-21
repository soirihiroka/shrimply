use std::any::Any;

use glam::Vec2;
pub use shrimply_paint_edit_skia::{
    DEFAULT_PAINT_ERASER_SCALE, PAINT_PREVIEW_STATE, PaintPreviewState,
};
use shrimply_paint_edit_skia::{
    PAINT_PREVIEW_FACET, PaintOnionFrame, PaintPreviewRender, ResolvedPathOffset,
    ResolvedShakyPath, preview_provider as paint_preview_provider, resolve_onion_frame,
};
use shrimply_preview_provider_skia::drawing::CanvasOperation;
use shrimply_preview_provider_skia::{
    PreviewBuilder, PreviewContext, PreviewExtensionKey, PreviewItemGeometry, PreviewProvider,
    PreviewTarget, PreviewViewport, SnapProvider2d, SnapScene, SnapTarget2d,
};
use shrimply_property_model::timeline_value::{
    TimelineBase, TimelineExpressionValue, TimelineKeyframe, TimelineValue,
};
use uuid::Uuid;

use super::{PaintDrawingKeyframe, PaintItem, Project, Time, VisualItem, VisualSource};

mod alpha_mask;
mod math;
mod scene_3d;
mod shape;
mod text;
mod visual_item;

pub use alpha_mask::{
    COMPOSITING_FACET as COMPOSITING_ALPHA_MASK_PREVIEW_FACET,
    MODIFIER_FACET as MODIFIER_ALPHA_MASK_PREVIEW_FACET,
};
pub use scene_3d::{TRACKED_CAMERA_PREVIEW, TrackedCameraPreview};
pub use shape::{
    APPEARANCE_FACET as SHAPE_APPEARANCE_PREVIEW_FACET,
    CONTENT_FACET as SHAPE_CONTENT_PREVIEW_FACET,
};
pub use text::APPEARANCE_FACET as TEXT_APPEARANCE_PREVIEW_FACET;
pub use visual_item::FACET as ITEM_PREVIEW_FACET;

struct LocalTimeBuilder<'a, B> {
    base: &'a B,
    time: Time,
}

impl<B: PreviewBuilder> PreviewContext for LocalTimeBuilder<'_, B> {
    fn timeline_position(&self) -> Time {
        self.base.timeline_position()
    }

    fn local_time(&self) -> Time {
        self.time
    }

    fn viewport(&self) -> PreviewViewport {
        self.base.viewport()
    }

    fn selection_color(&self) -> shrimply_preview_provider_skia::Color {
        self.base.selection_color()
    }

    fn selection_foreground_color(&self) -> shrimply_preview_provider_skia::Color {
        self.base.selection_foreground_color()
    }

    fn target_geometry(&self, target: PreviewTarget) -> Option<PreviewItemGeometry> {
        self.base.target_geometry(target)
    }

    fn source_size(&self, item_id: Uuid) -> Option<Vec2> {
        self.base.source_size(item_id)
    }

    fn item_geometry(&self, item_id: Uuid) -> Option<PreviewItemGeometry> {
        self.base.item_geometry(item_id)
    }

    fn snapping(&self) -> Option<&SnapScene> {
        self.base.snapping()
    }

    fn extension(&self, target: PreviewTarget, key: PreviewExtensionKey) -> Option<&dyn Any> {
        self.base.extension(target, key)
    }
}

impl<B: PreviewBuilder> PreviewBuilder for LocalTimeBuilder<'_, B> {
    fn resolve<T: TimelineExpressionValue>(&self, value: &TimelineValue<T>) -> T {
        self.base.resolve_at(value, self.time)
    }

    fn resolve_at<T: TimelineExpressionValue>(&self, value: &TimelineValue<T>, time: Time) -> T {
        self.base.resolve_at(value, time)
    }
}

fn resolved_paint_modifiers(
    item: &VisualItem,
    builder: &impl PreviewBuilder,
) -> (
    Vec<ResolvedPathOffset>,
    Vec<ResolvedShakyPath>,
    Vec<CanvasOperation>,
) {
    let mut path_offsets = Vec::new();
    let mut shaky_paths = Vec::new();
    let mut canvas_operations = Vec::new();
    for modifier in item.modifiers.iter().filter(|modifier| modifier.enabled) {
        let shrimply_visual_modifiers::ModifierEffect::Vector(effect) = &modifier.effect else {
            continue;
        };
        match &**effect {
            shrimply_visual_modifiers::VectorModifierEffect::Transform(effect) => {
                canvas_operations.push(CanvasOperation::Transform(
                    shrimply_math_geometry::ResolvedTransform2D {
                        position: builder.resolve(effect.position()),
                        anchor: builder.resolve(effect.anchor()),
                        scale: builder.resolve(effect.scale()),
                        shear: builder.resolve(effect.shear()),
                        rotation_degrees: builder.resolve(effect.rotation_degrees()),
                    }
                    .composed(),
                ));
            }
            shrimply_visual_modifiers::VectorModifierEffect::Repeat(effect) => {
                let row_offset = builder.resolve(&effect.row_offset);
                let row_offset = match builder.resolve(&effect.row_offset_axis) {
                    shrimply_visual_modifiers::repeat::RepeatOffsetAxis::X => {
                        Vec2::new(row_offset, 0.0)
                    }
                    shrimply_visual_modifiers::repeat::RepeatOffsetAxis::Y => {
                        Vec2::new(0.0, row_offset)
                    }
                };
                canvas_operations.push(CanvasOperation::Repeat {
                    copies_x: builder.resolve(&effect.copies_x).round().max(1.0) as u32,
                    copies_y: builder.resolve(&effect.copies_y).round().max(1.0) as u32,
                    step: builder.resolve(&effect.step),
                    row_offset,
                });
            }
            shrimply_visual_modifiers::VectorModifierEffect::PathOffset(effect) => {
                path_offsets.push(ResolvedPathOffset {
                    amplitude: builder.resolve(&effect.amplitude),
                    spacing: builder.resolve(&effect.spacing),
                    seed: builder.resolve(&effect.seed),
                    evolution: builder.resolve(&effect.evolution),
                });
            }
            shrimply_visual_modifiers::VectorModifierEffect::ShakyPath(effect) => {
                let amplitude = builder.resolve(&effect.amplitude).max(0.0);
                if amplitude > f32::EPSILON {
                    let seed = builder
                        .resolve(&effect.seed)
                        .round()
                        .clamp(0.0, u32::MAX as f32) as u32;
                    shaky_paths.push(ResolvedShakyPath {
                        amplitude,
                        step_size: builder.resolve(&effect.step_size).max(0.1),
                        seed: shrimply_math_media::shaky_path_seed(
                            seed,
                            builder.resolve(&effect.evolution),
                        ),
                    });
                }
            }
            shrimply_visual_modifiers::VectorModifierEffect::Opacity(effect) => {
                canvas_operations.push(CanvasOperation::Opacity(
                    builder.resolve(&effect.opacity).clamp(0.0, 1.0),
                ));
            }
            shrimply_visual_modifiers::VectorModifierEffect::Hsv(effect) => {
                canvas_operations.push(CanvasOperation::Hsv {
                    hue_turns: builder.resolve(&effect.hue_degrees) / 360.0,
                    saturation: builder.resolve(&effect.saturation).clamp(0.0, 2.0),
                    value: builder.resolve(&effect.value).clamp(0.0, 2.0),
                });
            }
            shrimply_visual_modifiers::VectorModifierEffect::TextMask(_) => {
                panic!("text mask modifier cannot be applied to paint")
            }
        }
    }
    (path_offsets, shaky_paths, canvas_operations)
}

fn paint_onion_frame(
    item: &VisualItem,
    paint: &PaintItem,
    keyframe: &PaintDrawingKeyframe,
    builder: &impl PreviewBuilder,
) -> Option<PaintOnionFrame> {
    let builder = LocalTimeBuilder {
        base: builder,
        time: keyframe.time(),
    };
    let geometry = item.preview_geometry(&builder)?;
    let (path_offsets, shaky_paths, canvas_operations) = resolved_paint_modifiers(item, &builder);
    Some(resolve_onion_frame(
        paint,
        keyframe.value().clone(),
        geometry,
        PaintPreviewRender {
            path_offsets,
            shaky_paths,
            canvas_operations,
            opacity: builder.resolve(&item.compositing.opacity).clamp(0.0, 1.0),
            blend_mode: builder.resolve(&item.compositing.blend_mode),
        },
        &builder,
    ))
}

impl VisualItem {
    pub fn tracking_camera_source(&self) -> Option<&shrimply_transform_3d::TrackingCameraSource> {
        let source = match &self.content {
            VisualSource::Obj(scene) => &scene.camera.source,
            VisualSource::Gaussian(scene) => &scene.camera.source,
            _ => return None,
        };
        match source {
            shrimply_transform_3d::CameraSource::Tracking(source) => Some(source),
            shrimply_transform_3d::CameraSource::Custom => None,
        }
    }

    pub fn default_preview_target(&self) -> PreviewTarget {
        PreviewTarget::new(
            self.id,
            if matches!(self.content, VisualSource::Paint(_)) {
                PAINT_PREVIEW_FACET
            } else {
                ITEM_PREVIEW_FACET
            },
        )
    }

    pub fn preview_geometry(&self, builder: &impl PreviewBuilder) -> Option<PreviewItemGeometry> {
        visual_item::geometry(self, builder)
    }

    pub fn owns_preview_target(&self, target: PreviewTarget) -> bool {
        if target.owner_id() == self.id {
            return match target.facet() {
                ITEM_PREVIEW_FACET | COMPOSITING_ALPHA_MASK_PREVIEW_FACET => true,
                SHAPE_CONTENT_PREVIEW_FACET | SHAPE_APPEARANCE_PREVIEW_FACET => {
                    matches!(self.content, VisualSource::Shape(_))
                }
                TEXT_APPEARANCE_PREVIEW_FACET => {
                    matches!(self.content, VisualSource::Text(_))
                }
                PAINT_PREVIEW_FACET => matches!(self.content, VisualSource::Paint(_)),
                _ => false,
            };
        }
        self.modifiers
            .iter()
            .any(|modifier| modifier.id == target.owner_id())
    }

    pub fn preview_provider(
        &self,
        target: PreviewTarget,
        builder: &impl PreviewBuilder,
    ) -> Option<Box<dyn PreviewProvider>> {
        if !self.owns_preview_target(target) {
            return None;
        }
        if matches!(
            self.content,
            VisualSource::Obj(_) | VisualSource::Gaussian(_)
        ) && ((target.owner_id() == self.id && target.facet() == ITEM_PREVIEW_FACET)
            || self.modifiers.iter().any(|modifier| {
                modifier.id == target.owner_id()
                    && matches!(
                        &modifier.effect,
                        shrimply_visual_modifiers::ModifierEffect::Scene3d(_)
                    )
            }))
        {
            return scene_3d::provider(self, target, builder);
        }
        let geometry = self.preview_geometry(builder)?;
        if target.owner_id() == self.id {
            return match (&self.content, target.facet()) {
                (_, ITEM_PREVIEW_FACET) => {
                    Some(visual_item::provider(self, target, geometry, builder))
                }
                (VisualSource::Shape(shape), SHAPE_CONTENT_PREVIEW_FACET) => {
                    Some(shape::provider(shape, target, geometry, builder))
                }
                (VisualSource::Shape(shape), SHAPE_APPEARANCE_PREVIEW_FACET) => {
                    Some(shape::provider(shape, target, geometry, builder))
                }
                (VisualSource::Text(text), TEXT_APPEARANCE_PREVIEW_FACET) => {
                    Some(text::provider(text, target, geometry, builder))
                }
                (VisualSource::Paint(paint), PAINT_PREVIEW_FACET) => {
                    let (path_offsets, shaky_paths, canvas_operations) =
                        resolved_paint_modifiers(self, builder);
                    let onion_frames = match &paint.drawing.base {
                        TimelineBase::Const(_) => [None, None],
                        TimelineBase::Keyframes(keyframes) => {
                            let time = builder.local_time();
                            [
                                keyframes
                                    .iter()
                                    .rev()
                                    .find(|keyframe| keyframe.time() < time)
                                    .and_then(|keyframe| {
                                        paint_onion_frame(self, paint, keyframe, builder)
                                    }),
                                keyframes
                                    .iter()
                                    .find(|keyframe| keyframe.time() > time)
                                    .and_then(|keyframe| {
                                        paint_onion_frame(self, paint, keyframe, builder)
                                    }),
                            ]
                        }
                    };
                    Some(paint_preview_provider(
                        paint,
                        PaintPreviewRender {
                            path_offsets,
                            shaky_paths,
                            canvas_operations,
                            opacity: builder.resolve(&self.compositing.opacity).clamp(0.0, 1.0),
                            blend_mode: builder.resolve(&self.compositing.blend_mode),
                        },
                        onion_frames,
                        target,
                        builder,
                    ))
                }
                (_, COMPOSITING_ALPHA_MASK_PREVIEW_FACET) => alpha_mask::provider(
                    self.compositing.alpha_mask.as_ref()?,
                    target,
                    geometry,
                    builder,
                ),
                _ => None,
            };
        }

        let modifier = self
            .modifiers
            .iter()
            .find(|modifier| modifier.id == target.owner_id())?;
        if target.facet() == MODIFIER_ALPHA_MASK_PREVIEW_FACET {
            return alpha_mask::provider(modifier.alpha_mask.as_ref()?, target, geometry, builder);
        }
        modifier.effect.preview_provider(target, builder)
    }

    pub fn preview_target_mut(&mut self, target: PreviewTarget) -> Option<&mut dyn Any> {
        if target.owner_id() == self.id {
            return match target.facet() {
                ITEM_PREVIEW_FACET => Some(self),
                SHAPE_CONTENT_PREVIEW_FACET | SHAPE_APPEARANCE_PREVIEW_FACET => {
                    let VisualSource::Shape(shape) = &mut self.content else {
                        return None;
                    };
                    Some(&mut **shape)
                }
                TEXT_APPEARANCE_PREVIEW_FACET => {
                    let VisualSource::Text(text) = &mut self.content else {
                        return None;
                    };
                    Some(&mut **text)
                }
                PAINT_PREVIEW_FACET => {
                    let VisualSource::Paint(paint) = &mut self.content else {
                        return None;
                    };
                    Some(&mut **paint)
                }
                COMPOSITING_ALPHA_MASK_PREVIEW_FACET => self
                    .compositing
                    .alpha_mask
                    .as_mut()
                    .map(|mask| mask as &mut dyn Any),
                _ => None,
            };
        }
        let modifier = self
            .modifiers
            .iter_mut()
            .find(|modifier| modifier.id == target.owner_id())?;
        if target.facet() == MODIFIER_ALPHA_MASK_PREVIEW_FACET {
            return modifier
                .alpha_mask
                .as_mut()
                .map(|mask| mask as &mut dyn Any);
        }
        match &mut modifier.effect {
            shrimply_visual_modifiers::ModifierEffect::Scene3d(effect) => Some(effect.as_mut()),
            effect => effect.preview_target_mut(target),
        }
    }
}

impl SnapProvider2d for VisualItem {
    fn provide_snap_targets(&self, builder: &impl PreviewBuilder, targets: &mut Vec<SnapTarget2d>) {
        if self
            .modifier_output_kind()
            .expect("visual modifier chain has no output kind")
            == shrimply_visual_modifiers::VisualKind::Scene3d
        {
            return;
        }
        if let Some(geometry) = self.preview_geometry(builder) {
            geometry.provide_snap_targets(builder, targets);
        }
    }
}

impl Project {
    pub fn preview_target_mut(&mut self, target: PreviewTarget) -> Option<&mut dyn Any> {
        for item in self
            .video_tracks
            .iter_mut()
            .flat_map(|track| &mut track.items)
            .chain(
                self.folded_sequences
                    .iter_mut()
                    .flat_map(|sequence| &mut sequence.video_tracks)
                    .flat_map(|track| &mut track.items),
            )
        {
            if item.owns_preview_target(target) {
                return item.preview_target_mut(target);
            }
        }
        None
    }
}
