use glam::{Mat3, Vec2};
use shrimply_math_geometry::{
    ResolvedTransform2D, normalized_angle_degrees, try_inverse, vector_angle_degrees,
};
use shrimply_paint_model::{
    PaintDrawing, PaintItem, PaintPoint, PaintStroke, ResolvedPaintFillOptions,
    ResolvedPaintStrokeEndOptions, ResolvedPaintStrokeOptions, ResolvedPaintTextureOptions,
};
use shrimply_preview_core::drawing::CanvasOperation;
use shrimply_preview_core::{
    Color, Cursor, CursorUpdate, Key, KeyState, KeyboardEvent, LayerBlendMode, Modifiers, Paint,
    PointerEvent, PointerInput, PointerSample, PreviewBuilder, PreviewContext, PreviewEditOutcome,
    PreviewEditSink, PreviewExtensionKey, PreviewFacetKey, PreviewItemGeometry, PreviewProvider,
    PreviewRefresh, PreviewResponse, PreviewTarget, Stroke,
};
use shrimply_timeline_value::{
    Time, TimelineBase, TimelineKeyframe, TimelineValue, TimelineValueType,
};
use uuid::Uuid;

use crate::{append_samples, erase_objects, erase_sweep, move_samples, toggle_fill};

pub const PAINT_PREVIEW_FACET: PreviewFacetKey = PreviewFacetKey::new("paint");
pub const PAINT_PREVIEW_STATE: PreviewExtensionKey =
    PreviewExtensionKey::new("paint.preview-state");
pub const DEFAULT_PAINT_ERASER_SCALE: f32 = 2.0;

#[derive(Clone, Copy)]
pub struct ResolvedShakyPath {
    pub amplitude: f32,
    pub step_size: f32,
    pub seed: u32,
}

pub struct PaintOnionFrame {
    drawing: PaintDrawing,
    revision: u64,
    mapping: PaintMapping,
    stroke_options: ResolvedPaintStrokeOptions,
    fill_options: ResolvedPaintFillOptions,
    path_offsets: Vec<shrimply_paint_geometry::ResolvedPathOffset>,
    path_effect: Option<skia_safe::PathEffect>,
    canvas_operations: Vec<CanvasOperation>,
    opacity: f32,
    blend_mode: LayerBlendMode,
    palette: Vec<shrimply_paint_skia::ResolvedPaintPaletteEntry>,
}

pub struct PaintPreviewRender {
    pub path_offsets: Vec<shrimply_paint_geometry::ResolvedPathOffset>,
    pub shaky_paths: Vec<ResolvedShakyPath>,
    pub canvas_operations: Vec<CanvasOperation>,
    pub opacity: f32,
    pub blend_mode: LayerBlendMode,
}

pub fn resolve_onion_frame(
    paint: &PaintItem,
    drawing: PaintDrawing,
    geometry: PreviewItemGeometry,
    render: PaintPreviewRender,
    builder: &impl PreviewBuilder,
) -> PaintOnionFrame {
    let PaintPreviewRender {
        path_offsets,
        shaky_paths,
        canvas_operations,
        opacity,
        blend_mode,
    } = render;
    PaintOnionFrame {
        drawing,
        revision: paint.revision,
        mapping: PaintMapping::new(
            geometry.local_to_canvas,
            builder.viewport().canvas_to_screen,
            resolve_stroke_transform(paint, builder),
            geometry.source_size,
            &canvas_operations,
        ),
        stroke_options: resolve_stroke_options(paint, builder),
        fill_options: ResolvedPaintFillOptions {
            closure_tolerance: builder.resolve(&paint.fill.closure_tolerance),
        },
        path_offsets,
        path_effect: resolved_path_effect(shaky_paths),
        canvas_operations,
        opacity,
        blend_mode,
        palette: resolve_palette(paint, builder),
    }
}

const HIT_RADIUS: f32 = 10.0;
const HANDLE_RADIUS: f32 = 6.0;
const ROTATION_HANDLE_DISTANCE: f32 = 30.0;
const OUTLINE_WIDTH: f32 = 3.0;
const CONNECTION_WIDTH: f32 = 1.5;
const MIN_SCALE: f32 = -10_000.0;
const SELECTION_ALPHA: f32 = 0.75;
const MIN_TOOL_SIZE: f32 = 0.1;
const MAX_TOOL_SIZE: f32 = 99.0;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PaintPreviewMode {
    #[default]
    Pen,
    Fill,
    StrokeTransform,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PaintPointSelection {
    Stroke {
        stroke_id: Uuid,
        sample_index: usize,
    },
    Fill {
        fill_id: Uuid,
        boundary_index: usize,
        point_index: usize,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct PaintPreviewState {
    pub mode: PaintPreviewMode,
    pub eraser: bool,
    pub pen_scale: f32,
    pub eraser_scale: f32,
    pub fill_tolerance: f32,
    pub palette_index: usize,
    pub onion_previous: bool,
    pub onion_next: bool,
    pub adjusting: bool,
    pub focused: Option<PaintPointSelection>,
    pub selected: Vec<PaintPointSelection>,
}

impl Default for PaintPreviewState {
    fn default() -> Self {
        Self {
            mode: PaintPreviewMode::Pen,
            eraser: false,
            pen_scale: shrimply_paint_model::DEFAULT_STROKE_WIDTH_SCALE,
            eraser_scale: 1.0,
            fill_tolerance: shrimply_paint_model::DEFAULT_FILL_CLOSURE_TOLERANCE,
            palette_index: 0,
            onion_previous: false,
            onion_next: false,
            adjusting: false,
            focused: None,
            selected: Vec::new(),
        }
    }
}

pub fn preview_provider(
    paint: &PaintItem,
    render: PaintPreviewRender,
    onion_frames: [Option<PaintOnionFrame>; 2],
    target: PreviewTarget,
    builder: &impl PreviewBuilder,
) -> Box<dyn PreviewProvider> {
    assert_eq!(
        target.facet(),
        PAINT_PREVIEW_FACET,
        "invalid paint preview facet"
    );
    Box::new(PaintHandler::new(
        paint,
        render,
        onion_frames,
        target,
        builder,
    ))
}

struct PaintHandler {
    target: PreviewTarget,
    paint: PaintItem,
    drawing: PaintDrawing,
    mapping: PaintMapping,
    stroke_options: ResolvedPaintStrokeOptions,
    fill_options: ResolvedPaintFillOptions,
    path_offsets: Vec<shrimply_paint_geometry::ResolvedPathOffset>,
    path_effect: Option<skia_safe::PathEffect>,
    canvas_operations: Vec<CanvasOperation>,
    opacity: f32,
    blend_mode: LayerBlendMode,
    onion_frames: [Option<PaintOnionFrame>; 2],
    palette: Vec<shrimply_paint_skia::ResolvedPaintPaletteEntry>,
    drawing_cache: std::cell::RefCell<shrimply_paint_skia::PaintCache>,
    active_cache: std::cell::RefCell<shrimply_paint_skia::PaintCache>,
    onion_cache: std::cell::RefCell<shrimply_paint_skia::PaintCache>,
    state: PaintPreviewState,
    previous_mode: PaintPreviewMode,
    control_adjusting: bool,
    pointer: Option<PointerInput>,
    gesture: Option<PaintGesture>,
    committed_stroke: Option<PaintStroke>,
    committed_revision: Option<u64>,
}

impl PaintHandler {
    fn new(
        paint: &PaintItem,
        render: PaintPreviewRender,
        onion_frames: [Option<PaintOnionFrame>; 2],
        target: PreviewTarget,
        builder: &impl PreviewBuilder,
    ) -> Self {
        let PaintPreviewRender {
            path_offsets,
            shaky_paths,
            canvas_operations,
            opacity,
            blend_mode,
        } = render;
        let geometry = builder
            .target_geometry(target)
            .expect("paint preview target geometry is missing");
        let stroke_transform = resolve_stroke_transform(paint, builder);
        let mapping = PaintMapping::new(
            geometry.local_to_canvas,
            builder.viewport().canvas_to_screen,
            stroke_transform,
            geometry.source_size,
            &canvas_operations,
        );
        let state = builder
            .extension(target, PAINT_PREVIEW_STATE)
            .and_then(|state| state.downcast_ref::<PaintPreviewState>())
            .cloned()
            .unwrap_or_default();
        Self {
            target,
            paint: paint.clone(),
            drawing: builder.resolve(&paint.drawing),
            mapping,
            stroke_options: resolve_stroke_options(paint, builder),
            fill_options: ResolvedPaintFillOptions {
                closure_tolerance: builder.resolve(&paint.fill.closure_tolerance),
            },
            path_offsets,
            path_effect: resolved_path_effect(shaky_paths),
            canvas_operations,
            opacity,
            blend_mode,
            onion_frames,
            palette: resolve_palette(paint, builder),
            drawing_cache: std::cell::RefCell::new(shrimply_paint_skia::PaintCache::default()),
            active_cache: std::cell::RefCell::new(shrimply_paint_skia::PaintCache::default()),
            onion_cache: std::cell::RefCell::new(shrimply_paint_skia::PaintCache::default()),
            previous_mode: state.mode,
            state,
            control_adjusting: false,
            pointer: None,
            gesture: None,
            committed_stroke: None,
            committed_revision: None,
        }
    }

    fn gesture(&self, input: PointerInput) -> PaintGesture {
        let maximum_palette_index = self
            .paint
            .palette
            .len()
            .checked_sub(1)
            .expect("paint palette is empty");
        PaintGesture {
            target: self.target,
            original: self.paint.clone(),
            mode: self.state.mode,
            eraser: self.state.eraser
                || matches!(input.tool, shrimply_preview_core::PointerTool::Eraser),
            adjusting: self.state.adjusting
                || self.control_adjusting
                || input.modifiers.contains(Modifiers::CONTROL),
            pen_scale: self.state.pen_scale,
            eraser_scale: self.state.eraser_scale,
            fill_tolerance: self.state.fill_tolerance,
            palette_index: self.state.palette_index.min(maximum_palette_index),
            mapping: self.mapping,
            stroke_options: self.stroke_options,
            palette: self.palette.clone(),
            path_offsets: self.path_offsets.clone(),
            path_effect: self.path_effect.clone(),
            base_drawing: self.drawing.clone(),
            base_revision: self.paint.revision,
            initial: Some(input.sample),
            latest: input.sample,
            active_stroke: None,
            live_stroke: None,
            previous_raw: None,
            changed: false,
            state_changed: false,
            acted: false,
            point_drag: None,
            object_erase_path: Vec::new(),
            transform_drag: None,
            dragged: false,
        }
    }

    fn accepts_pointer(&self, point: Vec2) -> bool {
        self.mapping.contains_screen(point)
            || (self.state.mode == PaintPreviewMode::StrokeTransform
                && self.mapping.hits_transform_control(point))
    }

    fn cursor(&self) -> Cursor {
        match self.state.mode {
            PaintPreviewMode::Pen | PaintPreviewMode::Fill => Cursor::Crosshair,
            PaintPreviewMode::StrokeTransform => Cursor::Move,
        }
    }

    fn sync_state(&mut self, context: &dyn PreviewContext) {
        if let Some(state) = context
            .extension(self.target, PAINT_PREVIEW_STATE)
            .and_then(|state| state.downcast_ref::<PaintPreviewState>())
        {
            self.state = state.clone();
        }
    }

    fn sync_state_from_edits(&mut self, edits: &mut dyn PreviewEditSink) {
        self.state = paint_state_mut(edits, self.target).clone();
    }

    fn persist_state(&self, edits: &mut dyn PreviewEditSink) {
        *paint_state_mut(edits, self.target) = self.state.clone();
    }

    fn refresh_paint(&mut self, context: &dyn PreviewContext, edits: &mut dyn PreviewEditSink) {
        self.paint = paint_mut(edits, self.target).clone();
        self.drawing = self.paint.drawing.value_at(context.local_time());
        self.mapping = self
            .mapping
            .with_stroke(resolved_stroke_at(&self.paint, context.local_time()));
    }

    fn set_mode(&mut self, mode: PaintPreviewMode) {
        if mode == PaintPreviewMode::StrokeTransform {
            self.previous_mode = self.state.mode;
            self.state.eraser = false;
        }
        self.state.mode = mode;
        if mode != PaintPreviewMode::Pen {
            self.state.focused = None;
        }
    }

    fn delete(
        &mut self,
        context: &dyn PreviewContext,
        edits: &mut dyn PreviewEditSink,
    ) -> PreviewResponse {
        if self.state.selected.is_empty() {
            return PreviewResponse::IGNORED;
        }
        let time = edits.keyframe_time();
        let changed = delete_selected(paint_mut(edits, self.target), time, &self.state.selected);
        if !changed {
            return PreviewResponse::IGNORED;
        }
        self.state.focused = None;
        self.state.selected.clear();
        self.persist_state(edits);
        self.refresh_paint(context, edits);
        PreviewResponse::edited(PreviewEditOutcome::committed(PreviewRefresh::ALL))
    }

    fn handle_key(
        &mut self,
        input: KeyboardEvent,
        context: &dyn PreviewContext,
        edits: &mut dyn PreviewEditSink,
    ) -> PreviewResponse {
        if input.key == Key::Control {
            self.control_adjusting = input.state == KeyState::Pressed;
            return PreviewResponse::handled().redraw();
        }
        if self.gesture.is_some() {
            return PreviewResponse::IGNORED;
        }
        if input.state != KeyState::Pressed {
            return PreviewResponse::IGNORED;
        }
        if input.key == Key::Delete {
            return self.delete(context, edits);
        }
        if input.modifiers.contains(Modifiers::ALT) || input.modifiers.contains(Modifiers::META) {
            return PreviewResponse::IGNORED;
        }
        let character = match input.key {
            Key::Character(character) => character.to_ascii_lowercase(),
            _ => return PreviewResponse::IGNORED,
        };
        let larger = matches!(character, '+' | ']')
            || (character == '=' && input.modifiers.contains(Modifiers::CONTROL));
        let smaller = matches!(character, '-' | '[');
        if larger || smaller {
            if self.state.mode == PaintPreviewMode::Fill {
                self.state.fill_tolerance = step_fill_tolerance(self.state.fill_tolerance, larger);
            } else if self.state.eraser {
                self.state.eraser_scale = step_tool_size(self.state.eraser_scale, larger);
            } else {
                self.state.pen_scale = step_tool_size(self.state.pen_scale, larger);
            }
            self.persist_state(edits);
            return PreviewResponse::handled().redraw();
        }
        match character {
            'b' if !input.modifiers.contains(Modifiers::CONTROL) => {
                self.state.eraser = false;
                self.state.adjusting = false;
                self.set_mode(PaintPreviewMode::Pen);
            }
            'e' if !input.modifiers.contains(Modifiers::CONTROL) => {
                if self.state.mode == PaintPreviewMode::StrokeTransform {
                    self.set_mode(PaintPreviewMode::Pen);
                }
                self.state.eraser = !self.state.eraser;
                if self.state.eraser {
                    self.state.focused = None;
                }
            }
            'f' if !input.modifiers.contains(Modifiers::CONTROL) => {
                self.state.eraser = false;
                self.set_mode(if self.state.mode == PaintPreviewMode::Fill {
                    PaintPreviewMode::Pen
                } else {
                    PaintPreviewMode::Fill
                });
            }
            't' if !input.modifiers.contains(Modifiers::CONTROL) => {
                self.state.eraser = false;
                if self.state.mode == PaintPreviewMode::StrokeTransform {
                    let mode = match self.previous_mode {
                        PaintPreviewMode::StrokeTransform => PaintPreviewMode::Pen,
                        mode => mode,
                    };
                    self.set_mode(mode);
                } else {
                    self.set_mode(PaintPreviewMode::StrokeTransform);
                }
            }
            _ => return PreviewResponse::IGNORED,
        }
        self.persist_state(edits);
        PreviewResponse::handled().redraw()
    }
}

impl PreviewProvider for PaintHandler {
    fn on_draw(
        &mut self,
        canvas: &shrimply_preview_core::PreviewCanvas,
        context: &dyn PreviewContext,
    ) {
        let mut state = context
            .extension(self.target, PAINT_PREVIEW_STATE)
            .and_then(|state| state.downcast_ref::<PaintPreviewState>())
            .cloned()
            .unwrap_or_else(|| self.state.clone());
        state.adjusting |= self.control_adjusting;
        let renderer = PreviewPaintRenderer {
            mapping: self.mapping,
            stroke_options: &self.stroke_options,
            fill_options: self.fill_options,
            path_offsets: &self.path_offsets,
            path_effect: self.path_effect.as_ref(),
            canvas_operations: &self.canvas_operations,
            opacity: self.opacity,
            blend_mode: self.blend_mode,
            palette: &self.palette,
        };
        let drew_active = self.gesture.as_ref().is_some_and(|gesture| {
            gesture.draw_active(
                canvas,
                context,
                &mut self.drawing_cache.borrow_mut(),
                &mut self.active_cache.borrow_mut(),
                renderer,
            )
        });
        if !drew_active {
            draw_preview_drawing(
                &mut self.drawing_cache.borrow_mut(),
                canvas,
                &self.drawing,
                self.paint.revision,
                renderer,
            );
        }
        draw_onion_skins(
            canvas,
            &self.onion_frames,
            &mut self.onion_cache.borrow_mut(),
            &state,
        );
        match state.mode {
            PaintPreviewMode::Pen if state.adjusting && !state.eraser => draw_adjust_nodes(
                canvas,
                &self.drawing,
                &self.mapping,
                &state,
                context.selection_color(),
            ),
            PaintPreviewMode::StrokeTransform => {
                draw_stroke_transform(canvas, &self.mapping, context.selection_color());
            }
            PaintPreviewMode::Pen | PaintPreviewMode::Fill => {}
        }
        if self.gesture.is_none() {
            let Some(pointer) = self.pointer else { return };
            draw_tool_cursor(
                canvas,
                self.mapping,
                &state,
                self.stroke_options,
                pointer,
                context.selection_color(),
            );
        }
    }

    fn on_pointer(
        &mut self,
        event: PointerEvent<'_>,
        context: &dyn PreviewContext,
        edits: &mut dyn PreviewEditSink,
    ) -> PreviewResponse {
        self.sync_state(context);
        match event {
            PointerEvent::Hover(input) => {
                self.pointer = Some(input);
                if self.accepts_pointer(input.sample.position) {
                    PreviewResponse {
                        handled: true,
                        redraw: true,
                        cursor: CursorUpdate::Set(self.cursor()),
                        edit: PreviewEditOutcome::UNCHANGED,
                    }
                } else {
                    PreviewResponse {
                        cursor: CursorUpdate::Clear,
                        ..PreviewResponse::handled()
                    }
                    .redraw()
                }
            }
            PointerEvent::Leave => {
                self.pointer = None;
                PreviewResponse {
                    cursor: CursorUpdate::Clear,
                    ..PreviewResponse::handled()
                }
                .redraw()
            }
            PointerEvent::Scroll { .. } => PreviewResponse::IGNORED,
            PointerEvent::Begin(input) => {
                self.pointer = Some(input);
                if input.button != shrimply_preview_core::PointerButton::Primary {
                    return PreviewResponse::IGNORED;
                }
                if self.gesture.is_none() && self.accepts_pointer(input.sample.position) {
                    let gesture = self.gesture(input);
                    self.gesture = Some(gesture);
                }
                if self.gesture.is_some() {
                    PreviewResponse {
                        handled: true,
                        redraw: false,
                        cursor: CursorUpdate::Set(Cursor::Grabbing),
                        edit: PreviewEditOutcome::UNCHANGED,
                    }
                } else {
                    PreviewResponse::IGNORED
                }
            }
            PointerEvent::Samples { input, samples } => {
                self.pointer = Some(input);
                let Some(gesture) = self.gesture.as_mut() else {
                    return PreviewResponse::IGNORED;
                };
                gesture.dragged = true;
                let mut samples = samples.to_vec();
                if samples.last().copied() != Some(input.sample) {
                    samples.push(input.sample);
                }
                let edit = gesture.drag(&samples, context, edits);
                let refresh_drawing = edit.changed()
                    && (gesture.eraser
                        || gesture.adjusting
                        || gesture.mode != PaintPreviewMode::Pen);
                self.sync_state_from_edits(edits);
                if refresh_drawing {
                    self.refresh_paint(context, edits);
                }
                PreviewResponse::edited(edit)
            }
            PointerEvent::End(input) => {
                self.pointer = Some(input);
                let Some(mut gesture) = self.gesture.take() else {
                    return PreviewResponse::IGNORED;
                };
                let mut edit = PreviewEditOutcome::UNCHANGED;
                if !gesture.dragged {
                    edit.merge(gesture.click(input, context, edits));
                }
                let committed_stroke = gesture.live_stroke.clone();
                edit.merge(gesture.finish(context, edits));
                self.sync_state_from_edits(edits);
                if edit.changed() {
                    self.refresh_paint(context, edits);
                }
                self.committed_stroke = edit.commits().then_some(committed_stroke).flatten();
                PreviewResponse::edited(edit)
            }
            PointerEvent::Cancel => {
                let Some(gesture) = self.gesture.take() else {
                    return PreviewResponse::IGNORED;
                };
                let edit = gesture.cancel(context, edits);
                self.sync_state_from_edits(edits);
                self.refresh_paint(context, edits);
                self.committed_stroke = None;
                self.committed_revision = None;
                PreviewResponse::edited(edit)
            }
        }
    }

    fn on_keyboard(
        &mut self,
        event: KeyboardEvent,
        context: &dyn PreviewContext,
        edits: &mut dyn PreviewEditSink,
    ) -> PreviewResponse {
        self.sync_state(context);
        self.handle_key(event, context, edits)
    }

    fn on_project_committed(&mut self, revision: u64) {
        if self.committed_stroke.is_some() {
            self.committed_revision = Some(revision);
        }
    }

    fn on_base_frame_presented(&mut self, revision: u64) {
        if self
            .committed_revision
            .is_some_and(|committed| revision >= committed)
        {
            self.committed_stroke = None;
            self.committed_revision = None;
        }
    }

    fn keeps_frame_until_base(&self) -> bool {
        self.committed_stroke.is_some()
    }

    fn base_frame_exclusion(&self) -> Option<Uuid> {
        Some(self.target.owner_id())
    }
}

#[derive(Clone, Copy)]
struct PaintMapping {
    stroke: ResolvedTransform2D,
    local_to_canvas: Mat3,
    canvas_to_screen: Mat3,
    raw_to_screen: Mat3,
    item_to_screen: Mat3,
    screen_to_raw: Option<Mat3>,
    screen_to_item: Option<Mat3>,
    source_size: Vec2,
}

impl PaintMapping {
    fn new(
        local_to_canvas: Mat3,
        canvas_to_screen: Mat3,
        stroke: ResolvedTransform2D,
        source_size: Vec2,
        canvas_operations: &[CanvasOperation],
    ) -> Self {
        let interactive_to_canvas =
            canvas_operations
                .iter()
                .fold(local_to_canvas, |matrix, operation| match operation {
                    CanvasOperation::Transform(transform) => transform.matrix * matrix,
                    _ => matrix,
                });
        let item_to_screen = canvas_to_screen * interactive_to_canvas;
        let raw_to_screen = item_to_screen * stroke.matrix();
        Self {
            stroke,
            local_to_canvas,
            canvas_to_screen,
            raw_to_screen,
            item_to_screen,
            screen_to_raw: try_inverse(raw_to_screen),
            screen_to_item: try_inverse(item_to_screen),
            source_size,
        }
    }

    fn screen_to_raw(self, point: Vec2) -> Option<Vec2> {
        self.screen_to_raw
            .map(|matrix| matrix.transform_point2(point))
    }

    fn with_stroke(self, stroke: ResolvedTransform2D) -> Self {
        let raw_to_screen = self.item_to_screen * stroke.matrix();
        Self {
            stroke,
            raw_to_screen,
            screen_to_raw: try_inverse(raw_to_screen),
            ..self
        }
    }

    fn screen_to_item(self, point: Vec2) -> Option<Vec2> {
        self.screen_to_item
            .map(|matrix| matrix.transform_point2(point))
    }

    fn contains_screen(self, point: Vec2) -> bool {
        self.screen_to_raw(point).is_some_and(|point| {
            point.cmpge(Vec2::ZERO).all() && point.cmple(self.source_size).all()
        })
    }

    fn hits_transform_control(self, point: Vec2) -> bool {
        let anchor = self.item_to_screen.transform_point2(self.stroke.position);
        let (_, rotation) = rotation_handle(self);
        anchor.distance(point) <= HIT_RADIUS
            || rotation.distance(point) <= HIT_RADIUS
            || raw_handles(self.source_size)
                .into_iter()
                .map(|handle| self.raw_to_screen.transform_point2(handle))
                .any(|handle| handle.distance(point) <= HIT_RADIUS)
    }

    fn screen_radius_to_raw(self, radius: f32) -> f32 {
        let Some(inverse) = self.screen_to_raw else {
            return 0.0;
        };
        inverse
            .transform_vector2(Vec2::X * radius)
            .length()
            .max(inverse.transform_vector2(Vec2::Y * radius).length())
    }

    fn item_radius_to_screen(self, radius: f32) -> f32 {
        self.item_to_screen
            .transform_vector2(Vec2::X * radius)
            .length()
            .max(
                self.item_to_screen
                    .transform_vector2(Vec2::Y * radius)
                    .length(),
            )
    }
}

#[derive(Clone, Copy)]
enum StrokeTransformDrag {
    Move,
    Anchor,
    Scale { x: bool, y: bool },
    Rotate { start_angle: f32 },
}

struct PointDrag {
    start: Vec2,
    points: Vec<(PaintPointSelection, Vec2)>,
}

struct PaintGesture {
    target: PreviewTarget,
    original: PaintItem,
    mode: PaintPreviewMode,
    eraser: bool,
    adjusting: bool,
    pen_scale: f32,
    eraser_scale: f32,
    fill_tolerance: f32,
    palette_index: usize,
    mapping: PaintMapping,
    stroke_options: ResolvedPaintStrokeOptions,
    palette: Vec<shrimply_paint_skia::ResolvedPaintPaletteEntry>,
    path_offsets: Vec<shrimply_paint_geometry::ResolvedPathOffset>,
    path_effect: Option<skia_safe::PathEffect>,
    base_drawing: PaintDrawing,
    base_revision: u64,
    initial: Option<PointerSample>,
    latest: PointerSample,
    active_stroke: Option<Uuid>,
    live_stroke: Option<PaintStroke>,
    previous_raw: Option<Vec2>,
    changed: bool,
    state_changed: bool,
    acted: bool,
    point_drag: Option<PointDrag>,
    object_erase_path: Vec<(Vec2, Option<f32>)>,
    transform_drag: Option<(StrokeTransformDrag, ResolvedTransform2D, Vec2)>,
    dragged: bool,
}

#[derive(Clone, Copy)]
struct PreviewPaintRenderer<'a> {
    mapping: PaintMapping,
    stroke_options: &'a ResolvedPaintStrokeOptions,
    fill_options: ResolvedPaintFillOptions,
    path_offsets: &'a [shrimply_paint_geometry::ResolvedPathOffset],
    path_effect: Option<&'a skia_safe::PathEffect>,
    canvas_operations: &'a [CanvasOperation],
    opacity: f32,
    blend_mode: LayerBlendMode,
    palette: &'a [shrimply_paint_skia::ResolvedPaintPaletteEntry],
}

fn draw_preview_drawing(
    cache: &mut shrimply_paint_skia::PaintCache,
    canvas: &shrimply_preview_core::PreviewCanvas,
    drawing: &PaintDrawing,
    revision: u64,
    renderer: PreviewPaintRenderer<'_>,
) {
    let frame = shrimply_paint_skia::prepare_frame(
        cache,
        (drawing, revision),
        renderer.stroke_options,
        renderer.fill_options,
        renderer.path_offsets,
        renderer.mapping.stroke,
        renderer.mapping.source_size,
    );
    draw_paint_layer(canvas, renderer, |canvas| {
        shrimply_paint_skia::draw(
            cache,
            canvas,
            &frame,
            shrimply_paint_skia::ResolvedPaintAppearance {
                palette: renderer.palette,
                reveal: None,
            },
            renderer.path_effect,
        )
        .expect("preview paint could not be rendered");
    });
}

fn draw_paint_layer(
    canvas: &shrimply_preview_core::PreviewCanvas,
    renderer: PreviewPaintRenderer<'_>,
    mut draw: impl FnMut(&shrimply_preview_core::PreviewCanvas),
) {
    canvas.save();
    canvas.concat(&shrimply_math_geometry::to_skia_matrix(
        renderer.mapping.canvas_to_screen,
    ));
    shrimply_preview_core::drawing::draw_composited(
        canvas,
        renderer.opacity,
        renderer.blend_mode,
        |canvas| {
            shrimply_preview_core::drawing::draw_with_operations(
                canvas,
                renderer.canvas_operations,
                |canvas| {
                    canvas.save();
                    canvas.concat(&shrimply_math_geometry::to_skia_matrix(
                        renderer.mapping.local_to_canvas,
                    ));
                    draw(canvas);
                    canvas.restore();
                },
            );
        },
    );
    canvas.restore();
}

impl PaintGesture {
    fn apply(
        &mut self,
        samples: &[PointerSample],
        context: &dyn PreviewContext,
        edits: &mut dyn PreviewEditSink,
    ) -> bool {
        let mut mapped =
            Vec::with_capacity(samples.len() + if self.initial.is_some() { 1 } else { 0 });
        if let Some(initial) = self.initial.take() {
            mapped.push(initial);
        }
        mapped.extend_from_slice(samples);
        mapped.retain(|sample| sample.position.is_finite());
        if mapped.is_empty() {
            return false;
        }
        self.latest = *mapped.last().expect("paint samples are empty");
        let changed = match self.mode {
            PaintPreviewMode::Pen if self.eraser => self.erase(&mapped, context, edits),
            PaintPreviewMode::Pen if self.adjusting => self.adjust(&mapped, context, edits),
            PaintPreviewMode::Pen => self.pen(&mapped, context, edits),
            PaintPreviewMode::Fill if self.eraser => {
                self.queue_object_erase(&mapped);
                false
            }
            PaintPreviewMode::Fill => self.fill(&mapped, context, edits),
            PaintPreviewMode::StrokeTransform => self.transform(&mapped, context, edits),
        };
        self.changed |= changed;
        changed
    }

    fn pen(
        &mut self,
        samples: &[PointerSample],
        _context: &dyn PreviewContext,
        edits: &mut dyn PreviewEditSink,
    ) -> bool {
        let points: Vec<_> = samples
            .iter()
            .filter_map(|sample| {
                Some(PaintPoint {
                    position: self.mapping.screen_to_raw(sample.position)?,
                    pressure: sample.pressure,
                })
            })
            .collect();
        let time = edits.keyframe_time();
        let paint = paint_mut(edits, self.target);
        let (drawing, revision) = editable_drawing_mut(paint, time);
        let result = append_samples(
            drawing,
            revision,
            self.active_stroke,
            &points,
            self.pen_scale,
            self.palette_index,
        );
        self.active_stroke = result.stroke_id;
        if let Some(stroke) = result
            .stroke_id
            .and_then(|stroke_id| drawing.strokes.iter().find(|stroke| stroke.id == stroke_id))
        {
            if let Some(live) = &mut self.live_stroke {
                live.points
                    .extend_from_slice(&stroke.points[live.points.len()..]);
            } else {
                self.live_stroke = Some(stroke.clone());
            }
        }
        result.changed
    }

    fn erase(
        &mut self,
        samples: &[PointerSample],
        _context: &dyn PreviewContext,
        edits: &mut dyn PreviewEditSink,
    ) -> bool {
        let mut changed = false;
        let time = edits.keyframe_time();
        let paint = paint_mut(edits, self.target);
        let (drawing, revision) = editable_drawing_mut(paint, time);
        for sample in samples {
            let Some(raw) = self.mapping.screen_to_raw(sample.position) else {
                continue;
            };
            let diameter = self.stroke_options.width
                * self.eraser_scale
                * shrimply_paint_geometry::pressure_diameter_scale(
                    self.stroke_options.thinning,
                    sample.pressure,
                );
            let raw_radius = self
                .mapping
                .screen_radius_to_raw(self.mapping.item_radius_to_screen(diameter * 0.5))
                .max(f32::EPSILON);
            changed |= erase_sweep(
                drawing,
                revision,
                self.previous_raw.unwrap_or(raw),
                raw,
                raw_radius,
            );
            self.previous_raw = Some(raw);
        }
        if changed {
            clear_selection(edits, self.target);
        }
        changed
    }

    fn fill(
        &mut self,
        samples: &[PointerSample],
        _context: &dyn PreviewContext,
        edits: &mut dyn PreviewEditSink,
    ) -> bool {
        if self.acted {
            return false;
        }
        self.acted = true;
        let Some(raw) = self.mapping.screen_to_raw(samples[0].position) else {
            return false;
        };
        let time = edits.keyframe_time();
        let paint = paint_mut(edits, self.target);
        let transform = self.mapping.stroke;
        let source_size = self.mapping.source_size;
        let (drawing, revision) = editable_drawing_mut(paint, time);
        let geometry = shrimply_paint_geometry::prepare_geometry(
            drawing,
            *revision,
            &self.stroke_options,
            ResolvedPaintFillOptions {
                closure_tolerance: self.fill_tolerance,
            },
            transform,
            source_size,
        );
        toggle_fill(
            drawing,
            revision,
            &geometry,
            raw,
            transform.matrix(),
            self.palette_index,
        )
    }

    fn queue_object_erase(&mut self, samples: &[PointerSample]) {
        for sample in samples {
            let Some(raw) = self.mapping.screen_to_raw(sample.position) else {
                continue;
            };
            if self
                .object_erase_path
                .last()
                .is_none_or(|(point, _)| *point != raw)
            {
                self.object_erase_path.push((raw, sample.pressure));
            }
        }
    }

    fn finish_object_erase(
        &mut self,
        _context: &dyn PreviewContext,
        edits: &mut dyn PreviewEditSink,
    ) -> bool {
        if self.object_erase_path.is_empty() {
            return false;
        }
        let transform = self.mapping.stroke;
        let source_size = self.mapping.source_size;
        let time = edits.keyframe_time();
        let paint = paint_mut(edits, self.target);
        let (drawing, revision) = editable_drawing_mut(paint, time);
        let geometry = shrimply_paint_geometry::prepare_geometry(
            drawing,
            *revision,
            &self.stroke_options,
            ResolvedPaintFillOptions {
                closure_tolerance: self.fill_tolerance,
            },
            transform,
            source_size,
        );
        let path: Vec<_> = self
            .object_erase_path
            .iter()
            .map(|(point, _)| transform.matrix().transform_point2(*point))
            .collect();
        let radii: Vec<_> = self
            .object_erase_path
            .iter()
            .map(|(_, pressure)| {
                self.stroke_options.width
                    * self.eraser_scale
                    * shrimply_paint_geometry::pressure_diameter_scale(
                        self.stroke_options.thinning,
                        *pressure,
                    )
                    * 0.5
            })
            .collect();
        let hits = shrimply_paint_geometry::hit_test_objects_sweep(&geometry, &path, &radii);
        let changed = erase_objects(drawing, revision, &hits.stroke_ids, &hits.fill_ids);
        if changed {
            clear_selection(edits, self.target);
        }
        changed
    }

    fn adjust(
        &mut self,
        samples: &[PointerSample],
        context: &dyn PreviewContext,
        edits: &mut dyn PreviewEditSink,
    ) -> bool {
        let Some(raw) = self
            .mapping
            .screen_to_raw(samples.last().expect("paint samples are empty").position)
        else {
            return false;
        };
        if !self.acted {
            self.acted = true;
            let paint = paint_mut(edits, self.target);
            let drawing = paint.drawing.value_at(context.local_time());
            let Some(start) = self.mapping.screen_to_raw(samples[0].position) else {
                return false;
            };
            let hit = hit_point(
                &drawing,
                start,
                self.mapping.screen_radius_to_raw(HIT_RADIUS),
            );
            let state = paint_state_mut(edits, self.target);
            state.focused = hit;
            self.state_changed = true;
            if let Some(hit) = hit {
                if !state.selected.contains(&hit) {
                    state.selected.push(hit);
                }
                self.point_drag = Some(PointDrag {
                    start,
                    points: selected_positions(&drawing, &state.selected),
                });
            } else {
                state.selected.clear();
            }
        }
        let Some(drag) = &self.point_drag else {
            return false;
        };
        let delta = raw - drag.start;
        let mut strokes = Vec::new();
        let mut fills = Vec::new();
        for &(selection, position) in &drag.points {
            match selection {
                PaintPointSelection::Stroke {
                    stroke_id,
                    sample_index,
                } => strokes.push((stroke_id, sample_index, position + delta)),
                PaintPointSelection::Fill {
                    fill_id,
                    boundary_index,
                    point_index,
                } => fills.push((fill_id, boundary_index, point_index, position + delta)),
            }
        }
        let time = edits.keyframe_time();
        let paint = paint_mut(edits, self.target);
        let (drawing, revision) = editable_drawing_mut(paint, time);
        move_samples(drawing, revision, &strokes, &fills)
    }

    fn transform(
        &mut self,
        samples: &[PointerSample],
        _context: &dyn PreviewContext,
        edits: &mut dyn PreviewEditSink,
    ) -> bool {
        let sample = samples.last().expect("paint samples are empty");
        let Some(item) = self.mapping.screen_to_item(sample.position) else {
            return false;
        };
        let (drag, start, start_item) = *self.transform_drag.get_or_insert_with(|| {
            let drag = hit_stroke_transform(self.mapping, samples[0].position);
            let start = self.mapping.stroke;
            let start_item = self
                .mapping
                .screen_to_item(samples[0].position)
                .unwrap_or(item);
            (drag, start, start_item)
        });
        let time = edits.keyframe_time();
        let paint = paint_mut(edits, self.target);
        let mut changed = false;
        match drag {
            StrokeTransformDrag::Move => {
                changed |= set_timeline_value(
                    &mut paint.stroke_transform.position,
                    time,
                    start.position + item - start_item,
                );
            }
            StrokeTransformDrag::Anchor => {
                let Some(inverse) = try_inverse(start.matrix()) else {
                    return false;
                };
                let anchor = inverse.transform_point2(item);
                let moved = ((anchor - start.anchor) * start.scale)
                    .rotate(Vec2::from_angle(start.rotation_degrees.to_radians()));
                changed |= set_timeline_value(
                    &mut paint.stroke_transform.position,
                    time,
                    start.position + moved,
                );
                changed |= set_timeline_value(&mut paint.stroke_transform.anchor, time, anchor);
            }
            StrokeTransformDrag::Scale { x, y } => {
                let rotation = Vec2::from_angle(-start.rotation_degrees.to_radians());
                let local = (item - start.position).rotate(rotation);
                let start_local = (start_item - start.position).rotate(rotation);
                let scale = Vec2::new(
                    if x {
                        scale_component(local.x, start_local.x, start.scale.x)
                    } else {
                        start.scale.x
                    },
                    if y {
                        scale_component(local.y, start_local.y, start.scale.y)
                    } else {
                        start.scale.y
                    },
                );
                changed |= set_timeline_value(&mut paint.stroke_transform.scale, time, scale);
            }
            StrokeTransformDrag::Rotate { start_angle } => {
                let Some(angle) = vector_angle_degrees(item - start.position) else {
                    return false;
                };
                changed |= set_timeline_value(
                    &mut paint.stroke_transform.rotation_degrees,
                    time,
                    start.rotation_degrees + normalized_angle_degrees(angle - start_angle),
                );
            }
        }
        if changed {
            paint.revision = paint
                .revision
                .checked_add(1)
                .expect("paint revision overflow");
        }
        changed
    }
}

impl PaintGesture {
    fn live_edit(&self, changed: bool, redraw: bool) -> PreviewEditOutcome {
        if changed || redraw {
            PreviewEditOutcome::live(PreviewRefresh::INSPECTOR)
        } else {
            PreviewEditOutcome::UNCHANGED
        }
    }

    fn drag(
        &mut self,
        samples: &[PointerSample],
        context: &dyn PreviewContext,
        edits: &mut dyn PreviewEditSink,
    ) -> PreviewEditOutcome {
        let changed = self.apply(samples, context, edits);
        let redraw = std::mem::take(&mut self.state_changed);
        self.live_edit(changed, redraw)
    }

    fn click(
        &mut self,
        input: PointerInput,
        context: &dyn PreviewContext,
        edits: &mut dyn PreviewEditSink,
    ) -> PreviewEditOutcome {
        let changed = self.apply(&[input.sample], context, edits);
        let redraw = std::mem::take(&mut self.state_changed);
        self.live_edit(changed, redraw)
    }

    fn finish(
        mut self,
        context: &dyn PreviewContext,
        edits: &mut dyn PreviewEditSink,
    ) -> PreviewEditOutcome {
        if !self.acted && self.active_stroke.is_none() {
            self.apply(&[], context, edits);
        }
        if self.mode == PaintPreviewMode::Fill && self.eraser {
            let changed = self.finish_object_erase(context, edits);
            self.changed |= changed;
        }
        if self.changed {
            PreviewEditOutcome::committed(PreviewRefresh::ALL)
        } else {
            PreviewEditOutcome::UNCHANGED
        }
    }

    fn cancel(
        self,
        _context: &dyn PreviewContext,
        edits: &mut dyn PreviewEditSink,
    ) -> PreviewEditOutcome {
        if !self.changed {
            return PreviewEditOutcome::UNCHANGED;
        }
        *paint_mut(edits, self.target) = self.original;
        PreviewEditOutcome::live(PreviewRefresh::ALL)
    }

    fn draw_active(
        &self,
        canvas: &shrimply_preview_core::PreviewCanvas,
        context: &dyn PreviewContext,
        drawing_cache: &mut shrimply_paint_skia::PaintCache,
        active_cache: &mut shrimply_paint_skia::PaintCache,
        renderer: PreviewPaintRenderer<'_>,
    ) -> bool {
        let erasing = self.eraser;
        if self.adjusting
            || self.mode == PaintPreviewMode::StrokeTransform
            || (self.mode == PaintPreviewMode::Fill && !erasing)
        {
            return false;
        }
        if !erasing {
            let Some(stroke) = &self.live_stroke else {
                return false;
            };
            let base = shrimply_paint_skia::prepare_frame(
                drawing_cache,
                (&self.base_drawing, self.base_revision),
                &self.stroke_options,
                ResolvedPaintFillOptions {
                    closure_tolerance: self.fill_tolerance,
                },
                &self.path_offsets,
                self.mapping.stroke,
                self.mapping.source_size,
            );
            let active_drawing = PaintDrawing {
                strokes: vec![stroke.clone()],
                fills: Vec::new(),
            };
            let active = shrimply_paint_skia::prepare_frame(
                active_cache,
                (&active_drawing, stroke.points.len() as u64),
                &self.stroke_options,
                ResolvedPaintFillOptions {
                    closure_tolerance: self.fill_tolerance,
                },
                &self.path_offsets,
                self.mapping.stroke,
                self.mapping.source_size,
            );
            draw_paint_layer(canvas, renderer, |canvas| {
                for (cache, frame) in [(&mut *drawing_cache, &base), (&mut *active_cache, &active)]
                {
                    shrimply_paint_skia::draw(
                        cache,
                        canvas,
                        frame,
                        shrimply_paint_skia::ResolvedPaintAppearance {
                            palette: &self.palette,
                            reveal: None,
                        },
                        self.path_effect.as_ref(),
                    )
                    .expect("active paint could not be rendered");
                }
            });
            return true;
        }
        let pressure = self.latest.pressure;
        let diameter = self.stroke_options.width
            * self.eraser_scale
            * shrimply_paint_geometry::pressure_diameter_scale(
                self.stroke_options.thinning,
                pressure,
            );
        let radius = self.mapping.item_radius_to_screen(diameter * 0.5);
        shrimply_preview_core::drawing::circle(
            canvas,
            self.latest.position,
            radius,
            Paint::stroke(Stroke::new(context.selection_color(), 2.0)),
        );
        false
    }
}

fn resolve_stroke_transform(
    paint: &PaintItem,
    builder: &impl PreviewBuilder,
) -> ResolvedTransform2D {
    ResolvedTransform2D {
        position: builder.resolve(&paint.stroke_transform.position),
        anchor: builder.resolve(&paint.stroke_transform.anchor),
        scale: builder.resolve(&paint.stroke_transform.scale),
        shear: builder.resolve(&paint.stroke_transform.shear),
        rotation_degrees: builder.resolve(&paint.stroke_transform.rotation_degrees),
    }
}

fn resolve_stroke_options(
    paint: &PaintItem,
    builder: &impl PreviewBuilder,
) -> ResolvedPaintStrokeOptions {
    ResolvedPaintStrokeOptions {
        width: builder.resolve(&paint.stroke.width),
        thinning: builder.resolve(&paint.stroke.thinning),
        smoothing: builder.resolve(&paint.stroke.smoothing),
        streamline: builder.resolve(&paint.stroke.streamline),
        simplification_tolerance: builder.resolve(&paint.stroke.simplification_tolerance),
        maximum_subdivision_spacing: builder.resolve(&paint.stroke.maximum_subdivision_spacing),
        start: ResolvedPaintStrokeEndOptions {
            cap: builder.resolve(&paint.stroke.start.cap).get(),
            taper: builder.resolve(&paint.stroke.start.taper),
            taper_distance: builder.resolve(&paint.stroke.start.taper_distance),
        },
        end: ResolvedPaintStrokeEndOptions {
            cap: builder.resolve(&paint.stroke.end.cap).get(),
            taper: builder.resolve(&paint.stroke.end.taper),
            taper_distance: builder.resolve(&paint.stroke.end.taper_distance),
        },
    }
}

fn resolve_palette(
    paint: &PaintItem,
    builder: &impl PreviewBuilder,
) -> Vec<shrimply_paint_skia::ResolvedPaintPaletteEntry> {
    paint
        .palette
        .iter()
        .map(|entry| shrimply_paint_skia::ResolvedPaintPaletteEntry {
            color: builder.resolve(&entry.color),
            texture: entry.texture.as_ref().map(|texture| {
                shrimply_paint_skia::ResolvedPaintTexture {
                    image_path: texture.image_path.clone(),
                    options: ResolvedPaintTextureOptions {
                        repeat_scale: builder.resolve(&texture.repeat_scale),
                        rotation_degrees: builder.resolve(&texture.rotation_degrees),
                    },
                }
            }),
        })
        .collect()
}

fn resolved_path_effect(shaky_paths: Vec<ResolvedShakyPath>) -> Option<skia_safe::PathEffect> {
    shaky_paths
        .into_iter()
        .filter_map(|effect| {
            skia_safe::PathEffect::discrete(effect.step_size, effect.amplitude, effect.seed)
        })
        .reduce(|previous, next| skia_safe::PathEffect::compose(next, previous))
}

fn resolved_stroke_at(paint: &PaintItem, time: Time) -> ResolvedTransform2D {
    ResolvedTransform2D {
        position: paint.stroke_transform.position.value_at(time),
        anchor: paint.stroke_transform.anchor.value_at(time),
        scale: paint.stroke_transform.scale.value_at(time),
        shear: paint.stroke_transform.shear.value_at(time),
        rotation_degrees: paint.stroke_transform.rotation_degrees.value_at(time),
    }
}

fn editable_drawing_mut(paint: &mut PaintItem, time: Time) -> (&mut PaintDrawing, &mut u64) {
    let drawing = match &mut paint.drawing.base {
        TimelineBase::Const(drawing) => drawing,
        TimelineBase::Keyframes(keyframes) => {
            if let Some(key) = keyframes.iter_mut().find(|key| key.time().approx_eq(time)) {
                *key.time_mut() = time;
            } else {
                keyframes.push(PaintDrawing::keyframe(time, PaintDrawing::default()));
            }
            keyframes.sort_by_key(|keyframe| keyframe.time());
            let index = keyframes
                .iter()
                .position(|keyframe| keyframe.time() == time)
                .expect("paint drawing keyframe is missing");
            keyframes[index].value_mut()
        }
    };
    (drawing, &mut paint.revision)
}

fn set_timeline_value<T: TimelineValueType + PartialEq>(
    timeline: &mut TimelineValue<T>,
    time: Time,
    value: T,
) -> bool {
    let mut changed_time = false;
    let current = match &mut timeline.base {
        TimelineBase::Const(current) => current,
        TimelineBase::Keyframes(keyframes) => {
            if let Some(key) = keyframes.iter_mut().find(|key| key.time().approx_eq(time)) {
                changed_time = key.time() != time;
                *key.time_mut() = time;
            } else {
                changed_time = true;
                keyframes.push(T::keyframe(time, value.clone()));
            }
            keyframes.sort_by_key(|keyframe| keyframe.time());
            let index = keyframes
                .iter()
                .position(|keyframe| keyframe.time() == time)
                .expect("transform keyframe is missing");
            keyframes[index].value_mut()
        }
    };
    if *current == value {
        return changed_time;
    }
    *current = value;
    true
}

fn paint_mut(edits: &mut dyn PreviewEditSink, target: PreviewTarget) -> &mut PaintItem {
    edits
        .target_mut(target)
        .downcast_mut::<PaintItem>()
        .expect("paint preview target is not a PaintItem")
}

fn paint_state_mut(
    edits: &mut dyn PreviewEditSink,
    target: PreviewTarget,
) -> &mut PaintPreviewState {
    edits
        .extension_mut(target, PAINT_PREVIEW_STATE)
        .expect("paint preview state is missing")
        .downcast_mut::<PaintPreviewState>()
        .expect("paint preview state has the wrong type")
}

fn clear_selection(edits: &mut dyn PreviewEditSink, target: PreviewTarget) {
    let state = paint_state_mut(edits, target);
    state.focused = None;
    state.selected.clear();
}

fn delete_selected(paint: &mut PaintItem, time: Time, selected: &[PaintPointSelection]) -> bool {
    let mut strokes = Vec::new();
    let mut fills = Vec::new();
    for selection in selected {
        match *selection {
            PaintPointSelection::Stroke {
                stroke_id,
                sample_index,
            } => strokes.push((stroke_id, sample_index)),
            PaintPointSelection::Fill {
                fill_id,
                boundary_index,
                point_index,
            } => fills.push((fill_id, boundary_index, point_index)),
        }
    }
    let (drawing, revision) = editable_drawing_mut(paint, time);
    crate::remove_samples(drawing, revision, &strokes, &fills)
}

fn hit_point(drawing: &PaintDrawing, point: Vec2, radius: f32) -> Option<PaintPointSelection> {
    shrimply_paint_geometry::hit_test_samples(drawing, Mat3::IDENTITY, point, radius)
        .map(|hit| PaintPointSelection::Stroke {
            stroke_id: hit.stroke_id,
            sample_index: hit.sample_index,
        })
        .or_else(|| {
            let maximum_distance = radius * radius;
            drawing
                .fills
                .iter()
                .flat_map(|fill| {
                    fill.loops
                        .iter()
                        .enumerate()
                        .flat_map(move |(boundary_index, boundary)| {
                            boundary
                                .iter()
                                .enumerate()
                                .map(move |(point_index, value)| {
                                    (
                                        PaintPointSelection::Fill {
                                            fill_id: fill.id,
                                            boundary_index,
                                            point_index,
                                        },
                                        value.distance_squared(point),
                                    )
                                })
                        })
                })
                .filter(|(_, distance)| *distance <= maximum_distance)
                .min_by(|left, right| left.1.total_cmp(&right.1))
                .map(|(selection, _)| selection)
        })
}

fn selected_positions(
    drawing: &PaintDrawing,
    selected: &[PaintPointSelection],
) -> Vec<(PaintPointSelection, Vec2)> {
    selected
        .iter()
        .filter_map(|selection| {
            let position = match *selection {
                PaintPointSelection::Stroke {
                    stroke_id,
                    sample_index,
                } => {
                    drawing
                        .strokes
                        .iter()
                        .find(|stroke| stroke.id == stroke_id)?
                        .points
                        .get(sample_index)?
                        .position
                }
                PaintPointSelection::Fill {
                    fill_id,
                    boundary_index,
                    point_index,
                } => *drawing
                    .fills
                    .iter()
                    .find(|fill| fill.id == fill_id)?
                    .loops
                    .get(boundary_index)?
                    .get(point_index)?,
            };
            Some((*selection, position))
        })
        .collect()
}

fn draw_adjust_nodes(
    painter: &shrimply_preview_core::PreviewCanvas,
    drawing: &PaintDrawing,
    mapping: &PaintMapping,
    state: &PaintPreviewState,
    selection_color: Color,
) {
    let connection = Stroke::new(
        Color::new(0.125, 0.5, 0.875, SELECTION_ALPHA),
        CONNECTION_WIDTH,
    );
    for stroke in &drawing.strokes {
        let points: Vec<_> = stroke
            .points
            .iter()
            .map(|point| mapping.raw_to_screen.transform_point2(point.position))
            .collect();
        shrimply_preview_core::drawing::polyline(
            painter,
            &points,
            false,
            Paint::stroke(connection),
        );
    }
    for fill in &drawing.fills {
        for boundary in &fill.loops {
            let points: Vec<_> = boundary
                .iter()
                .map(|point| mapping.raw_to_screen.transform_point2(*point))
                .collect();
            shrimply_preview_core::drawing::polyline(
                painter,
                &points,
                true,
                Paint::stroke(connection),
            );
        }
    }
    let node = |position: Vec2, selection: PaintPointSelection| {
        let focused = state.focused == Some(selection);
        let selected = state.selected.contains(&selection);
        shrimply_preview_core::drawing::circle(
            painter,
            position,
            if focused {
                7.0
            } else if selected {
                6.0
            } else {
                5.0
            },
            Paint::fill(Color::new(1.0, 1.0, 1.0, 0.86)),
        );
        shrimply_preview_core::drawing::circle(
            painter,
            position,
            if focused {
                4.5
            } else if selected {
                4.0
            } else {
                3.0
            },
            Paint::fill(if focused {
                Color::new(1.0, 0.5, 0.0, 1.0)
            } else {
                selection_color
            }),
        );
    };
    for stroke in &drawing.strokes {
        for (sample_index, point) in stroke.points.iter().enumerate() {
            node(
                mapping.raw_to_screen.transform_point2(point.position),
                PaintPointSelection::Stroke {
                    stroke_id: stroke.id,
                    sample_index,
                },
            );
        }
    }
    for fill in &drawing.fills {
        for (boundary_index, boundary) in fill.loops.iter().enumerate() {
            for (point_index, point) in boundary.iter().enumerate() {
                node(
                    mapping.raw_to_screen.transform_point2(*point),
                    PaintPointSelection::Fill {
                        fill_id: fill.id,
                        boundary_index,
                        point_index,
                    },
                );
            }
        }
    }
}

fn draw_onion_skins(
    canvas: &shrimply_preview_core::PreviewCanvas,
    frames: &[Option<PaintOnionFrame>; 2],
    cache: &mut shrimply_paint_skia::PaintCache,
    state: &PaintPreviewState,
) {
    for (frame, color) in [
        (
            state.onion_previous.then_some(&frames[0]),
            Color::<u8>::new(240, 71, 71, 148),
        ),
        (
            state.onion_next.then_some(&frames[1]),
            Color::<u8>::new(71, 143, 255, 148),
        ),
    ] {
        let Some(frame) = frame.and_then(Option::as_ref) else {
            continue;
        };
        let palette: Vec<_> = frame
            .palette
            .iter()
            .map(|entry| shrimply_paint_skia::ResolvedPaintPaletteEntry {
                color,
                texture: entry.texture.clone(),
            })
            .collect();
        let prepared = shrimply_paint_skia::prepare_frame(
            cache,
            (&frame.drawing, frame.revision),
            &frame.stroke_options,
            frame.fill_options,
            &frame.path_offsets,
            frame.mapping.stroke,
            frame.mapping.source_size,
        );
        draw_paint_layer(
            canvas,
            PreviewPaintRenderer {
                mapping: frame.mapping,
                stroke_options: &frame.stroke_options,
                fill_options: frame.fill_options,
                path_offsets: &frame.path_offsets,
                path_effect: frame.path_effect.as_ref(),
                canvas_operations: &frame.canvas_operations,
                opacity: frame.opacity,
                blend_mode: frame.blend_mode,
                palette: &palette,
            },
            |canvas| {
                shrimply_paint_skia::draw(
                    cache,
                    canvas,
                    &prepared,
                    shrimply_paint_skia::ResolvedPaintAppearance {
                        palette: &palette,
                        reveal: None,
                    },
                    frame.path_effect.as_ref(),
                )
                .expect("paint onion skin could not be rendered");
            },
        );
    }
}

fn draw_stroke_transform(
    painter: &shrimply_preview_core::PreviewCanvas,
    mapping: &PaintMapping,
    selection_color: Color,
) {
    let corners =
        raw_corners(mapping.source_size).map(|point| mapping.raw_to_screen.transform_point2(point));
    shrimply_preview_core::drawing::polyline(
        painter,
        &corners,
        true,
        Paint::stroke(Stroke::new(selection_color, OUTLINE_WIDTH)),
    );
    for handle in
        raw_handles(mapping.source_size).map(|point| mapping.raw_to_screen.transform_point2(point))
    {
        shrimply_preview_core::drawing::circle(
            painter,
            handle,
            HANDLE_RADIUS,
            Paint::fill(Color::<f32>::WHITE),
        );
        shrimply_preview_core::drawing::circle(
            painter,
            handle,
            HANDLE_RADIUS - 2.0,
            Paint::fill(selection_color),
        );
    }
    let (stem, rotation) = rotation_handle(*mapping);
    shrimply_preview_core::drawing::line(
        painter,
        stem,
        rotation,
        Stroke::new(selection_color, OUTLINE_WIDTH),
    );
    shrimply_preview_core::drawing::circle(
        painter,
        rotation,
        HANDLE_RADIUS,
        Paint::fill(Color::<f32>::WHITE),
    );
    shrimply_preview_core::drawing::circle(
        painter,
        rotation,
        HANDLE_RADIUS - 2.0,
        Paint::fill(selection_color),
    );
    let anchor = mapping
        .item_to_screen
        .transform_point2(mapping.stroke.position);
    shrimply_preview_core::drawing::circle(
        painter,
        anchor,
        HANDLE_RADIUS + 2.0,
        Paint::stroke(Stroke::new(selection_color, 2.0)),
    );
}

fn hit_stroke_transform(mapping: PaintMapping, point: Vec2) -> StrokeTransformDrag {
    let anchor = mapping
        .item_to_screen
        .transform_point2(mapping.stroke.position);
    if anchor.distance(point) <= HIT_RADIUS {
        return StrokeTransformDrag::Anchor;
    }
    let (_, rotation) = rotation_handle(mapping);
    if rotation.distance(point) <= HIT_RADIUS {
        let start_angle = mapping
            .screen_to_item(point)
            .and_then(|point| vector_angle_degrees(point - mapping.stroke.position))
            .unwrap_or(mapping.stroke.rotation_degrees);
        return StrokeTransformDrag::Rotate { start_angle };
    }
    let handles =
        raw_handles(mapping.source_size).map(|raw| mapping.raw_to_screen.transform_point2(raw));
    let axes = [
        (true, true),
        (false, true),
        (true, true),
        (true, false),
        (true, true),
        (false, true),
        (true, true),
        (true, false),
    ];
    handles
        .into_iter()
        .zip(axes)
        .find(|(handle, _)| handle.distance(point) <= HIT_RADIUS)
        .map_or(StrokeTransformDrag::Move, |(_, (x, y))| {
            StrokeTransformDrag::Scale { x, y }
        })
}

fn raw_corners(size: Vec2) -> [Vec2; 4] {
    [
        Vec2::ZERO,
        Vec2::new(size.x, 0.0),
        size,
        Vec2::new(0.0, size.y),
    ]
}

fn raw_handles(size: Vec2) -> [Vec2; 8] {
    let center = size * 0.5;
    [
        Vec2::ZERO,
        Vec2::new(center.x, 0.0),
        Vec2::new(size.x, 0.0),
        Vec2::new(size.x, center.y),
        size,
        Vec2::new(center.x, size.y),
        Vec2::new(0.0, size.y),
        Vec2::new(0.0, center.y),
    ]
}

fn rotation_handle(mapping: PaintMapping) -> (Vec2, Vec2) {
    let corners =
        raw_corners(mapping.source_size).map(|point| mapping.raw_to_screen.transform_point2(point));
    let stem = (corners[0] + corners[1]) * 0.5;
    let center = corners.into_iter().sum::<Vec2>() * 0.25;
    let outward = (stem - center).try_normalize().unwrap_or(-Vec2::Y);
    (stem, stem + outward * ROTATION_HANDLE_DISTANCE)
}

fn scale_component(pointer: f32, start_pointer: f32, start_scale: f32) -> f32 {
    if start_pointer.abs() <= f32::EPSILON {
        start_scale
    } else {
        (start_scale * pointer / start_pointer).clamp(MIN_SCALE, -MIN_SCALE)
    }
}

fn step_tool_size(size: f32, larger: bool) -> f32 {
    if larger {
        if size < 1.0 {
            (size * 10.0 + 1.0).round().min(10.0) / 10.0
        } else {
            (size.round() + 1.0).min(MAX_TOOL_SIZE)
        }
    } else if size > 1.0 {
        (size.round() - 1.0).max(1.0)
    } else {
        (size * 10.0 - 1.0).round().max(1.0) / 10.0
    }
    .clamp(MIN_TOOL_SIZE, MAX_TOOL_SIZE)
}

fn step_fill_tolerance(tolerance: f32, larger: bool) -> f32 {
    let direction = if larger { 1.0 } else { -1.0 };
    (tolerance.round() + direction).clamp(1.0, MAX_TOOL_SIZE)
}

fn draw_tool_cursor(
    painter: &shrimply_preview_core::PreviewCanvas,
    mapping: PaintMapping,
    state: &PaintPreviewState,
    options: ResolvedPaintStrokeOptions,
    input: PointerInput,
    color: Color,
) {
    let eraser = state.eraser || matches!(input.tool, shrimply_preview_core::PointerTool::Eraser);
    if state.adjusting
        || state.mode == PaintPreviewMode::StrokeTransform
        || (state.mode == PaintPreviewMode::Fill && !eraser)
    {
        return;
    }
    let scale = if eraser {
        state.eraser_scale
    } else {
        state.pen_scale
    };
    let diameter = options.width
        * scale
        * shrimply_paint_geometry::pressure_diameter_scale(options.thinning, input.sample.pressure);
    shrimply_preview_core::drawing::circle(
        painter,
        input.sample.position,
        mapping.item_radius_to_screen(diameter * 0.5),
        Paint::stroke(Stroke::new(color, 2.0)),
    );
}
