use std::any::Any;
pub mod accuracy;
pub mod performance;
pub mod playback;

use glam::{Mat3, Vec2};
pub use shrimply_math_color::{Color, LayerBlendMode};
use shrimply_math_geometry::snap::{AxisFeature, AxisTarget};
pub use shrimply_math_geometry::snap::{AxisFeatures, AxisGap, AxisSnap, AxisSnapKind, SnapAxis};
pub use shrimply_math_geometry::{Rect, ResolvedTransform2D};
pub use shrimply_preview_skia::{Paint, Stroke};
use shrimply_timeline_value::{Time, TimelineExpressionValue, TimelineValue};
pub use skia_safe::Canvas as PreviewCanvas;
use uuid::Uuid;

const GUIDE_SNAP_PRIORITY: u8 = 0;
const CANVAS_SNAP_PRIORITY: u8 = 1;
const RECT_SNAP_PRIORITY: u8 = 2;

pub mod drawing {
    pub use shrimply_preview_skia::{
        CanvasOperation, circle, draw_composited, draw_with_operations, line, polyline, rect, text,
    };
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PointerTool {
    #[default]
    Mouse,
    Pen,
    Eraser,
    Touch,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PointerButton {
    #[default]
    Primary,
    Middle,
    Secondary,
    Other(u32),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Modifiers(u8);

impl Modifiers {
    pub const NONE: Self = Self(0);
    pub const SHIFT: Self = Self(1 << 0);
    pub const CONTROL: Self = Self(1 << 1);
    pub const ALT: Self = Self(1 << 2);
    pub const META: Self = Self(1 << 3);

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl std::ops::BitOr for Modifiers {
    type Output = Self;

    fn bitor(self, right: Self) -> Self::Output {
        Self(self.0 | right.0)
    }
}

impl std::ops::BitOrAssign for Modifiers {
    fn bitor_assign(&mut self, right: Self) {
        self.0 |= right.0;
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PointerSample {
    pub position: Vec2,
    pub pressure: Option<f32>,
    pub tilt: Option<Vec2>,
    pub time_millis: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PointerInput {
    pub sample: PointerSample,
    pub tool: PointerTool,
    pub button: PointerButton,
    pub modifiers: Modifiers,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Cursor {
    #[default]
    Default,
    Pointer,
    Crosshair,
    Move,
    Grab,
    Grabbing,
    Text,
    ResizeHorizontal,
    ResizeVertical,
    ResizeDiagonalDown,
    ResizeDiagonalUp,
    Hidden,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PreviewFacetKey(&'static str);

impl PreviewFacetKey {
    pub const fn new(key: &'static str) -> Self {
        assert!(!key.is_empty(), "preview facet key cannot be empty");
        Self(key)
    }

    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PreviewExtensionKey(&'static str);

impl PreviewExtensionKey {
    pub const fn new(key: &'static str) -> Self {
        assert!(!key.is_empty(), "preview extension key cannot be empty");
        Self(key)
    }

    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PreviewTarget {
    owner_id: Uuid,
    facet: PreviewFacetKey,
}

impl PreviewTarget {
    pub const fn new(owner_id: Uuid, facet: PreviewFacetKey) -> Self {
        Self { owner_id, facet }
    }

    pub const fn owner_id(self) -> Uuid {
        self.owner_id
    }

    pub const fn facet(self) -> PreviewFacetKey {
        self.facet
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PreviewViewport {
    pub canvas_size: Vec2,
    pub content_rect: Rect,
    pub canvas_to_screen: Mat3,
}

impl PreviewViewport {
    pub fn new(canvas_size: Vec2, content_rect: Rect) -> Self {
        assert!(canvas_size.min_element() > 0.0, "preview canvas is empty");
        assert!(
            content_rect.size().min_element() > 0.0,
            "preview viewport is empty"
        );
        Self {
            canvas_size,
            content_rect,
            canvas_to_screen: Mat3::from_scale_angle_translation(
                content_rect.size() / canvas_size,
                0.0,
                content_rect.min,
            ),
        }
    }

    pub fn canvas_point_to_screen(self, point: Vec2) -> Vec2 {
        self.canvas_to_screen.transform_point2(point)
    }

    pub fn screen_point_to_canvas(self, point: Vec2) -> Vec2 {
        self.canvas_to_screen.inverse().transform_point2(point)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PreviewItemGeometry {
    pub source_size: Vec2,
    pub bounds: Rect,
    pub decoration_size: Vec2,
    pub anchor_offset: Vec2,
    pub transform: ResolvedTransform2D,
    pub local_to_canvas: Mat3,
}

#[derive(Clone)]
pub struct SnapScene {
    radius_px: f32,
    radius_canvas: f32,
    canvas_size: Vec2,
    points: Vec<Vec2>,
    x_targets: Vec<AxisTarget>,
    y_targets: Vec<AxisTarget>,
    rects: Vec<Rect>,
}

#[derive(Clone, Copy)]
pub enum SnapTarget2d {
    Point(Vec2),
    VerticalLine(f32),
    HorizontalLine(f32),
    Rect(Rect),
}

pub trait SnapProvider2d {
    fn provide_snap_targets(&self, builder: &impl PreviewBuilder, targets: &mut Vec<SnapTarget2d>);
}

#[derive(Clone, Copy, Default)]
pub struct SnapResult {
    pub delta: Vec2,
    pub x: Option<AxisSnap>,
    pub y: Option<AxisSnap>,
}

impl SnapScene {
    pub fn new(viewport: PreviewViewport, radius_px: f32) -> Self {
        let pixels_per_canvas = viewport
            .canvas_to_screen
            .transform_vector2(Vec2::X)
            .length()
            .max(f32::EPSILON);
        Self {
            radius_px: radius_px.max(0.0),
            radius_canvas: radius_px.max(0.0) / pixels_per_canvas,
            canvas_size: viewport.canvas_size,
            points: Vec::new(),
            x_targets: [0.0, viewport.canvas_size.x * 0.5, viewport.canvas_size.x]
                .map(|value| AxisTarget {
                    value,
                    priority: CANVAS_SNAP_PRIORITY,
                })
                .to_vec(),
            y_targets: [0.0, viewport.canvas_size.y * 0.5, viewport.canvas_size.y]
                .map(|value| AxisTarget {
                    value,
                    priority: CANVAS_SNAP_PRIORITY,
                })
                .to_vec(),
            rects: Vec::new(),
        }
    }

    pub fn add_guides(&mut self, vertical: &[f32], horizontal: &[f32]) {
        self.x_targets.extend(
            vertical
                .iter()
                .copied()
                .filter(|value| value.is_finite())
                .map(|value| AxisTarget {
                    value,
                    priority: GUIDE_SNAP_PRIORITY,
                }),
        );
        self.y_targets.extend(
            horizontal
                .iter()
                .copied()
                .filter(|value| value.is_finite())
                .map(|value| AxisTarget {
                    value,
                    priority: GUIDE_SNAP_PRIORITY,
                }),
        );
        self.x_targets.sort_by_key(|target| target.priority);
        self.y_targets.sort_by_key(|target| target.priority);
    }

    pub fn add_provider(&mut self, provider: &impl SnapProvider2d, builder: &impl PreviewBuilder) {
        let mut targets = Vec::new();
        provider.provide_snap_targets(builder, &mut targets);
        for target in targets {
            match target {
                SnapTarget2d::Point(point) if point.is_finite() => self.points.push(point),
                SnapTarget2d::VerticalLine(value) if value.is_finite() => {
                    self.x_targets.push(AxisTarget {
                        value,
                        priority: RECT_SNAP_PRIORITY,
                    })
                }
                SnapTarget2d::HorizontalLine(value) if value.is_finite() => {
                    self.y_targets.push(AxisTarget {
                        value,
                        priority: RECT_SNAP_PRIORITY,
                    })
                }
                SnapTarget2d::Rect(rect)
                    if rect.min.is_finite()
                        && rect.max.is_finite()
                        && rect.min.cmple(rect.max).all() =>
                {
                    self.rects.push(rect)
                }
                _ => {}
            }
        }
    }

    pub fn snap_point(&self, point: Vec2) -> SnapResult {
        self.snap_point_to_lines(point, None)
    }

    pub fn snap_point_to_geometry(&self, point: Vec2, geometry: PreviewItemGeometry) -> SnapResult {
        let lines = shrimply_math_geometry::snap::transformed_rect_lines(
            geometry.local_to_canvas,
            geometry.bounds,
        );
        self.snap_point_to_lines(point, Some(lines))
    }

    pub fn snap_geometry(&self, transform: Mat3, bounds: Rect) -> SnapResult {
        self.snap_geometry_with_features(transform, bounds, AxisFeatures::ALL, AxisFeatures::ALL)
    }

    pub fn snap_geometry_with_features(
        &self,
        transform: Mat3,
        bounds: Rect,
        x_features: AxisFeatures,
        y_features: AxisFeatures,
    ) -> SnapResult {
        self.snap_rect_with_features(
            shrimply_math_geometry::snap::transformed_rect(transform, bounds),
            x_features,
            y_features,
        )
    }

    pub fn snap_rect(&self, rect: Rect) -> SnapResult {
        self.snap_rect_with_features(rect, AxisFeatures::ALL, AxisFeatures::ALL)
    }

    pub fn snap_rect_with_features(
        &self,
        rect: Rect,
        x_features: AxisFeatures,
        y_features: AxisFeatures,
    ) -> SnapResult {
        let x = shrimply_math_geometry::snap::nearest_rect_axis_snap(
            rect,
            &self.rects,
            &self.x_targets,
            self.canvas_size,
            SnapAxis::X,
            x_features,
            self.radius_canvas,
        );
        let y = shrimply_math_geometry::snap::nearest_rect_axis_snap(
            rect,
            &self.rects,
            &self.y_targets,
            self.canvas_size,
            SnapAxis::Y,
            y_features,
            self.radius_canvas,
        );
        SnapResult {
            delta: Vec2::new(
                x.map_or(0.0, |snap| snap.delta),
                y.map_or(0.0, |snap| snap.delta),
            ),
            x,
            y,
        }
    }

    pub fn snap_along(&self, point: Vec2, direction: Vec2) -> SnapResult {
        let direction = direction.normalize_or_zero();
        if direction == Vec2::ZERO {
            return SnapResult::default();
        }
        let mut nearest = None;
        for target in self.x_lines() {
            if direction.x.abs() > f32::EPSILON {
                let delta = direction * ((target.value - point.x) / direction.x);
                add_snap_result(
                    &mut nearest,
                    SnapResult {
                        delta,
                        x: Some(axis_snap(point.x, target)),
                        y: None,
                    },
                    self.radius_canvas,
                );
            }
        }
        for target in self.y_lines() {
            if direction.y.abs() > f32::EPSILON {
                let delta = direction * ((target.value - point.y) / direction.y);
                add_snap_result(
                    &mut nearest,
                    SnapResult {
                        delta,
                        x: None,
                        y: Some(axis_snap(point.y, target)),
                    },
                    self.radius_canvas,
                );
            }
        }
        for target in &self.points {
            let delta = *target - point;
            let along = direction * delta.dot(direction);
            if (delta - along).length() <= f32::EPSILON * 64.0 {
                add_snap_result(
                    &mut nearest,
                    SnapResult {
                        delta,
                        x: Some(axis_snap(
                            point.x,
                            AxisTarget {
                                value: target.x,
                                priority: RECT_SNAP_PRIORITY,
                            },
                        )),
                        y: Some(axis_snap(
                            point.y,
                            AxisTarget {
                                value: target.y,
                                priority: RECT_SNAP_PRIORITY,
                            },
                        )),
                    },
                    self.radius_canvas,
                );
            }
        }
        nearest.unwrap_or_default()
    }

    pub fn snap_angle(&self, angle_degrees: f32, radius_px: f32, step_degrees: f32) -> Option<f32> {
        if radius_px <= f32::EPSILON {
            return None;
        }
        shrimply_math_geometry::snap::nearest_angle_degrees(
            angle_degrees,
            step_degrees,
            (self.radius_px / radius_px).to_degrees(),
        )
    }

    fn snap_point_to_lines(&self, point: Vec2, extra: Option<([f32; 3], [f32; 3])>) -> SnapResult {
        let x_targets = self
            .x_lines()
            .chain(extra.into_iter().flat_map(|lines| {
                lines.0.map(|value| AxisTarget {
                    value,
                    priority: RECT_SNAP_PRIORITY,
                })
            }))
            .collect::<Vec<_>>();
        let y_targets = self
            .y_lines()
            .chain(extra.into_iter().flat_map(|lines| {
                lines.1.map(|value| AxisTarget {
                    value,
                    priority: RECT_SNAP_PRIORITY,
                })
            }))
            .collect::<Vec<_>>();
        let snap = shrimply_math_geometry::snap::nearest_2d_snap(
            [point.x; 3],
            [point.y; 3],
            x_targets.iter().map(|target| target.value),
            y_targets.iter().map(|target| target.value),
            self.radius_canvas,
        );
        let axis = SnapResult {
            delta: snap.delta(),
            x: snap.x.map(|snap| {
                axis_snap(
                    point.x,
                    x_targets
                        .iter()
                        .copied()
                        .find(|target| target.value == snap.target)
                        .expect("snapped x target is absent"),
                )
            }),
            y: snap.y.map(|snap| {
                axis_snap(
                    point.y,
                    y_targets
                        .iter()
                        .copied()
                        .find(|target| target.value == snap.target)
                        .expect("snapped y target is absent"),
                )
            }),
        };
        let mut nearest = (snap.x.is_some() || snap.y.is_some()).then_some(axis);
        for target in &self.points {
            add_snap_result(
                &mut nearest,
                SnapResult {
                    delta: *target - point,
                    x: Some(axis_snap(
                        point.x,
                        AxisTarget {
                            value: target.x,
                            priority: RECT_SNAP_PRIORITY,
                        },
                    )),
                    y: Some(axis_snap(
                        point.y,
                        AxisTarget {
                            value: target.y,
                            priority: RECT_SNAP_PRIORITY,
                        },
                    )),
                },
                self.radius_canvas,
            );
        }
        nearest.unwrap_or_default()
    }

    fn x_lines(&self) -> impl Iterator<Item = AxisTarget> + '_ {
        self.x_targets
            .iter()
            .copied()
            .chain(self.rects.iter().flat_map(|rect| {
                [rect.min.x, rect.center().x, rect.max.x].map(|value| AxisTarget {
                    value,
                    priority: RECT_SNAP_PRIORITY,
                })
            }))
    }

    fn y_lines(&self) -> impl Iterator<Item = AxisTarget> + '_ {
        self.y_targets
            .iter()
            .copied()
            .chain(self.rects.iter().flat_map(|rect| {
                [rect.min.y, rect.center().y, rect.max.y].map(|value| AxisTarget {
                    value,
                    priority: RECT_SNAP_PRIORITY,
                })
            }))
    }
}

fn axis_snap(source: f32, target: AxisTarget) -> AxisSnap {
    AxisSnap {
        delta: target.value - source,
        source,
        target: target.value,
        feature: AxisFeature::Center,
        kind: AxisSnapKind::Align,
        priority: target.priority,
    }
}

fn add_snap_result(nearest: &mut Option<SnapResult>, candidate: SnapResult, radius: f32) {
    let distance = candidate.delta.length();
    if distance <= radius
        && nearest.is_none_or(|current| {
            distance < current.delta.length()
                || (distance == current.delta.length()
                    && snap_priority(candidate) < snap_priority(current))
        })
    {
        *nearest = Some(candidate);
    }
}

fn snap_priority(snap: SnapResult) -> u8 {
    snap.x
        .into_iter()
        .chain(snap.y)
        .map(|axis| axis.priority)
        .min()
        .unwrap_or(u8::MAX)
}

impl SnapProvider2d for PreviewItemGeometry {
    fn provide_snap_targets(
        &self,
        _builder: &impl PreviewBuilder,
        targets: &mut Vec<SnapTarget2d>,
    ) {
        targets.push(SnapTarget2d::Rect(
            shrimply_math_geometry::snap::transformed_rect(self.local_to_canvas, self.bounds),
        ));
    }
}

/// Object-safe services available while a provider draws and receives events.
pub trait PreviewContext {
    fn timeline_position(&self) -> Time;
    fn local_time(&self) -> Time;
    fn viewport(&self) -> PreviewViewport;
    fn selection_color(&self) -> Color;
    fn target_geometry(&self, target: PreviewTarget) -> Option<PreviewItemGeometry>;
    fn source_size(&self, item_id: Uuid) -> Option<Vec2>;
    fn item_geometry(&self, item_id: Uuid) -> Option<PreviewItemGeometry>;

    fn snapping(&self) -> Option<&SnapScene> {
        None
    }

    fn extension(&self, _target: PreviewTarget, _key: PreviewExtensionKey) -> Option<&dyn Any> {
        None
    }
}

/// Construction-time services. This is intentionally not object-safe: concrete
/// owner modules resolve their own typed timeline values before returning a
/// dynamically dispatched provider.
pub trait PreviewBuilder: PreviewContext {
    fn resolve<T: TimelineExpressionValue>(&self, value: &TimelineValue<T>) -> T;
    fn resolve_at<T: TimelineExpressionValue>(&self, value: &TimelineValue<T>, time: Time) -> T;
}

/// Provides the requested domain object without exposing the project model.
///
/// Implementations should panic when `target` does not resolve. Callbacks should
/// downcast the returned value with `Any::downcast_mut` and panic on a type
/// mismatch so an invalid edit cannot leave a partially changed project.
pub trait PreviewEditSink {
    fn keyframe_time(&self) -> Time;
    fn target_mut(&mut self, target: PreviewTarget) -> &mut dyn Any;

    fn updated_geometry(&self, _target: PreviewTarget) -> Option<PreviewItemGeometry> {
        None
    }

    fn extension_mut(
        &mut self,
        _target: PreviewTarget,
        _key: PreviewExtensionKey,
    ) -> Option<&mut dyn Any> {
        None
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Key {
    Character(char),
    Backspace,
    Delete,
    Escape,
    Enter,
    Space,
    Tab,
    Control,
    Shift,
    Alt,
    #[default]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum KeyState {
    #[default]
    Pressed,
    Released,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct KeyboardEvent {
    pub key: Key,
    pub state: KeyState,
    pub repeat: bool,
    pub modifiers: Modifiers,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PointerEvent<'a> {
    Hover(PointerInput),
    Leave,
    Scroll {
        input: PointerInput,
        delta: Vec2,
    },
    Begin(PointerInput),
    Samples {
        input: PointerInput,
        samples: &'a [PointerSample],
    },
    End(PointerInput),
    Cancel,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PreviewRefresh(u8);

impl PreviewRefresh {
    pub const NONE: Self = Self(0);
    pub const PREVIEW: Self = Self(1 << 0);
    pub const INSPECTOR: Self = Self(1 << 1);
    pub const TIMELINE: Self = Self(1 << 2);
    pub const ALL: Self = Self(Self::PREVIEW.0 | Self::INSPECTOR.0 | Self::TIMELINE.0);

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl std::ops::BitOr for PreviewRefresh {
    type Output = Self;

    fn bitor(self, right: Self) -> Self::Output {
        Self(self.0 | right.0)
    }
}

impl std::ops::BitOrAssign for PreviewRefresh {
    fn bitor_assign(&mut self, right: Self) {
        self.0 |= right.0;
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PreviewEditOutcome {
    kind: PreviewEditKind,
    pub refresh: PreviewRefresh,
}

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
enum PreviewEditKind {
    #[default]
    None,
    Live,
    Cancel,
    Commit,
}

impl PreviewEditOutcome {
    pub const UNCHANGED: Self = Self {
        kind: PreviewEditKind::None,
        refresh: PreviewRefresh::NONE,
    };

    pub const fn live(refresh: PreviewRefresh) -> Self {
        Self {
            kind: PreviewEditKind::Live,
            refresh,
        }
    }

    pub const fn committed(refresh: PreviewRefresh) -> Self {
        Self {
            kind: PreviewEditKind::Commit,
            refresh,
        }
    }

    pub const fn changed(self) -> bool {
        !matches!(self.kind, PreviewEditKind::None)
    }

    pub const fn commits(self) -> bool {
        matches!(self.kind, PreviewEditKind::Commit)
    }

    pub const fn is_live(self) -> bool {
        matches!(self.kind, PreviewEditKind::Live)
    }

    pub const fn canceled(mut self) -> Self {
        if self.changed() {
            self.kind = PreviewEditKind::Cancel;
        }
        self
    }

    pub fn merge(&mut self, other: Self) {
        self.kind = self.kind.max(other.kind);
        self.refresh |= other.refresh;
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CursorUpdate {
    #[default]
    Keep,
    Set(Cursor),
    Clear,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PreviewResponse {
    pub handled: bool,
    pub redraw: bool,
    pub cursor: CursorUpdate,
    pub edit: PreviewEditOutcome,
}

impl PreviewResponse {
    pub const IGNORED: Self = Self {
        handled: false,
        redraw: false,
        cursor: CursorUpdate::Keep,
        edit: PreviewEditOutcome::UNCHANGED,
    };

    pub const fn handled() -> Self {
        Self {
            handled: true,
            ..Self::IGNORED
        }
    }

    pub const fn edited(edit: PreviewEditOutcome) -> Self {
        Self {
            handled: true,
            redraw: true,
            cursor: CursorUpdate::Keep,
            edit,
        }
    }

    pub const fn redraw(mut self) -> Self {
        self.redraw = true;
        self
    }
}

/// The single provider for one focused preview target and native input sequence.
pub trait PreviewProvider {
    fn on_pointer(
        &mut self,
        event: PointerEvent<'_>,
        context: &dyn PreviewContext,
        edits: &mut dyn PreviewEditSink,
    ) -> PreviewResponse;

    fn on_keyboard(
        &mut self,
        _event: KeyboardEvent,
        _context: &dyn PreviewContext,
        _edits: &mut dyn PreviewEditSink,
    ) -> PreviewResponse {
        PreviewResponse::IGNORED
    }

    fn on_cancel(
        &mut self,
        context: &dyn PreviewContext,
        edits: &mut dyn PreviewEditSink,
    ) -> PreviewResponse {
        self.on_pointer(PointerEvent::Cancel, context, edits)
    }

    /// Called only after input changed provider state and before the next display frame.
    fn on_draw(&mut self, canvas: &skia_safe::Canvas, context: &dyn PreviewContext);

    fn on_project_committed(&mut self, _revision: u64) {}

    fn on_base_frame_presented(&mut self, _revision: u64) {}

    fn keeps_frame_until_base(&self) -> bool {
        false
    }

    fn base_frame_exclusion(&self) -> Option<Uuid> {
        None
    }
}

pub const KEYPOINT_RADIUS: f32 = 6.0;
pub const KEYPOINT_HIT_RADIUS: f32 = 11.0;
pub const CONTROL_LINE_WIDTH: f32 = 2.0;
pub const CONTROL_SHADOW_WIDTH: f32 = 5.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BoundsHandle {
    TopLeft,
    Top,
    TopRight,
    Right,
    BottomRight,
    Bottom,
    BottomLeft,
    Left,
}

pub const BOUNDS_HANDLES: [BoundsHandle; 8] = [
    BoundsHandle::TopLeft,
    BoundsHandle::Top,
    BoundsHandle::TopRight,
    BoundsHandle::Right,
    BoundsHandle::BottomRight,
    BoundsHandle::Bottom,
    BoundsHandle::BottomLeft,
    BoundsHandle::Left,
];

pub fn bounds_handle_position(handle: BoundsHandle, bounds: Rect) -> Vec2 {
    let center = bounds.center();
    match handle {
        BoundsHandle::TopLeft => bounds.min,
        BoundsHandle::Top => Vec2::new(center.x, bounds.min.y),
        BoundsHandle::TopRight => Vec2::new(bounds.max.x, bounds.min.y),
        BoundsHandle::Right => Vec2::new(bounds.max.x, center.y),
        BoundsHandle::BottomRight => bounds.max,
        BoundsHandle::Bottom => Vec2::new(center.x, bounds.max.y),
        BoundsHandle::BottomLeft => Vec2::new(bounds.min.x, bounds.max.y),
        BoundsHandle::Left => Vec2::new(bounds.min.x, center.y),
    }
}

pub fn bounds_handle_cursor(handle: BoundsHandle) -> Cursor {
    match handle {
        BoundsHandle::Top | BoundsHandle::Bottom => Cursor::ResizeVertical,
        BoundsHandle::Left | BoundsHandle::Right => Cursor::ResizeHorizontal,
        BoundsHandle::TopLeft | BoundsHandle::BottomRight => Cursor::ResizeDiagonalDown,
        BoundsHandle::TopRight | BoundsHandle::BottomLeft => Cursor::ResizeDiagonalUp,
    }
}

pub fn draw_keypoint(canvas: &PreviewCanvas, position: Vec2, color: Color) {
    drawing::circle(canvas, position, KEYPOINT_RADIUS, Paint::fill(color));
}

pub fn draw_control_line(canvas: &PreviewCanvas, start: Vec2, end: Vec2, color: Color) {
    drawing::line(
        canvas,
        start,
        end,
        Stroke::new(Color::new(0.0, 0.0, 0.0, 0.67), CONTROL_SHADOW_WIDTH),
    );
    drawing::line(canvas, start, end, Stroke::new(color, CONTROL_LINE_WIDTH));
}

pub fn draw_control_rect(canvas: &PreviewCanvas, transform: Mat3, bounds: Rect, color: Color) {
    let points = [
        bounds.min,
        Vec2::new(bounds.max.x, bounds.min.y),
        bounds.max,
        Vec2::new(bounds.min.x, bounds.max.y),
    ]
    .map(|point| transform.transform_point2(point));
    drawing::polyline(
        canvas,
        &points,
        true,
        Paint::stroke(Stroke::new(
            Color::new(0.0, 0.0, 0.0, 0.67),
            CONTROL_SHADOW_WIDTH,
        )),
    );
    drawing::polyline(
        canvas,
        &points,
        true,
        Paint::stroke(Stroke::new(color, CONTROL_LINE_WIDTH)),
    );
}

pub fn draw_keypoints(canvas: &PreviewCanvas, positions: &[Vec2], color: Color) {
    for &position in positions {
        draw_keypoint(canvas, position, color);
    }
}

pub fn hit_keypoint(pointer: Vec2, position: Vec2) -> bool {
    pointer.distance_squared(position) <= KEYPOINT_HIT_RADIUS * KEYPOINT_HIT_RADIUS
}

pub mod canvas;
pub mod math;
