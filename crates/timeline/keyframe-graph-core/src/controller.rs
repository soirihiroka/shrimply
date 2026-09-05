use std::{
    ops::{Deref, DerefMut},
    time::Instant,
};

use shrimply_interpolation::Interpolation;
use shrimply_math_color::Color;
use shrimply_math_core::Time;
pub use shrimply_skia_adw_core::slider::ScrollInput as FrameGraphScrollInput;
use shrimply_skia_adw_core::{
    Axis, Edge, Scrollbar,
    canvas::{TimelinePainter, vec2},
    slider,
};
use uuid::Uuid;

use crate::{
    CURSOR_LANE_HEIGHT, GRAPH_PAD, GraphDomain, KeyframeGraph, KeyframeGraphDraw, KeyframePoint,
    RawSegment, STEP_GRAPH_RANGE, draw_keyframes, raw_point, raw_range, segment_speed_at,
    speed_range, time_x, value_y,
};

pub const GRAPH_SLIDER_HEIGHT: f64 = 20.0;
pub const FRAME_GRAPH_HEIGHT: i32 = 132;
const HIT_RADIUS: f64 = 7.0;
const SNAP_RADIUS_PX: f64 = 8.0;
const MINIMUM_VISIBLE_PIXELS: f64 = 10_000.0;
const MINIMUM_SECONDS_PER_PIXEL: f64 = 0.000_001;
const WHEEL_UNITS_PER_STEP: f64 = 120.0;

#[derive(Clone, Copy, Default)]
pub struct FrameGraphModifiers {
    pub control: bool,
    pub shift: bool,
}

#[derive(Clone, Copy)]
pub struct FrameGraphPointerPosition {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum FrameGraphPointerButton {
    Primary,
    Middle,
    Secondary,
}

#[derive(Clone, Copy)]
pub enum FrameGraphKey {
    PreviousFrame,
    NextFrame,
    Start,
    End,
    ZoomIn,
    ZoomOut,
    PreviousKey,
    NextKey,
    Delete,
    Copy,
    Paste,
    TogglePlayback,
}

#[derive(Clone)]
pub enum FrameGraphAction {
    PlayheadChanged(Time),
    KeysChanged(Vec<KeyframePoint>),
    KeysMoved(Vec<FrameGraphKeyMove>),
    KeysDeleted(Vec<Time>),
    KeyAdded(KeyframePoint),
    KeysPasted(Vec<KeyframePoint>),
    CopyRequested(Vec<Time>),
    PasteRequested(Time),
    TogglePlayback,
    EditFinished,
    InterpolationRequested {
        owner_id: Uuid,
        interpolation: Interpolation,
        x: f64,
        y: f64,
    },
    TextInterpolationRequested {
        owner_id: Uuid,
        x: f64,
        y: f64,
    },
}

#[derive(Clone, Copy)]
pub struct FrameGraphKeyMove {
    pub old_time: Time,
    pub time: Time,
    pub value: f64,
}

#[derive(Clone)]
pub struct FrameGraphComponentAction {
    pub component: usize,
    pub action: FrameGraphAction,
}

#[derive(Clone, Copy)]
pub struct FrameGraphStatus {
    pub can_previous: bool,
    pub can_next: bool,
    pub key_at_playhead: bool,
    pub value: f64,
}

#[derive(Clone, Copy)]
struct GraphViewState {
    scroll_seconds: f64,
    seconds_per_pixel: f64,
    minimum_seconds_per_pixel: Option<f64>,
    initialized: bool,
}

impl Default for GraphViewState {
    fn default() -> Self {
        Self {
            scroll_seconds: 0.0,
            seconds_per_pixel: 1.0 / 60.0,
            minimum_seconds_per_pixel: None,
            initialized: false,
        }
    }
}

impl GraphViewState {
    fn initialize(&mut self, range: GraphDomain, width: f64) {
        if self.initialized || width <= 0.0 {
            return;
        }
        let duration = graph_duration_seconds(range);
        self.scroll_seconds = range.0.as_secs_f64();
        self.seconds_per_pixel =
            (duration / graph_plot_width(width)).max(self.minimum_scale(duration));
        self.initialized = true;
    }

    fn clamp(&mut self, range: GraphDomain, width: f64) {
        let duration = graph_duration_seconds(range);
        let plot_width = graph_plot_width(width);
        let minimum = self.minimum_scale(duration);
        let maximum = (duration / plot_width).max(minimum);
        self.seconds_per_pixel = self.seconds_per_pixel.clamp(minimum, maximum);
        let visible = (plot_width * self.seconds_per_pixel).clamp(0.0, duration);
        if visible >= duration || width <= 0.0 {
            self.scroll_seconds = range.0.as_secs_f64();
            self.seconds_per_pixel = maximum;
            return;
        }
        let minimum_scroll = range.0.as_secs_f64();
        let maximum_scroll = (range.1.as_secs_f64() - visible).max(minimum_scroll);
        self.scroll_seconds = self.scroll_seconds.clamp(minimum_scroll, maximum_scroll);
    }

    fn domain(&self, range: GraphDomain, width: f64) -> GraphDomain {
        let visible = graph_plot_width(width) * self.seconds_per_pixel;
        let end = (self.scroll_seconds + visible).min(range.1.as_secs_f64());
        (
            Time::from_seconds_f64(self.scroll_seconds),
            Time::from_seconds_f64(end.max(self.scroll_seconds)),
        )
    }

    fn minimum_scale(&self, duration: f64) -> f64 {
        self.minimum_seconds_per_pixel
            .unwrap_or_else(|| minimum_scale(duration))
    }
}

#[derive(Clone, Copy)]
struct GraphSelectionBox {
    start_x: f64,
    start_y: f64,
    end_x: f64,
    end_y: f64,
    add_to_selection: bool,
}

#[derive(Clone, Copy)]
enum DragTarget {
    Point(Time),
    Cursor,
    SelectBox,
    Scrollbar,
    Pan,
}

#[derive(Clone, Copy)]
struct ActiveDrag {
    target: DragTarget,
    start_x: f64,
    start_scroll_seconds: f64,
}

#[derive(Clone, Copy)]
struct GraphOverscroll {
    edge: Edge,
    started_at: Instant,
    distance: f64,
}

pub struct FrameGraphState {
    graph: KeyframeGraph,
    item_range: GraphDomain,
    frame_step: Time,
    playhead: Time,
    selected_keys: Vec<Time>,
    focused_key: Option<Time>,
    view: GraphViewState,
    pointer: Option<(f64, f64)>,
    selection_box: Option<GraphSelectionBox>,
    active_drag: Option<ActiveDrag>,
    overscroll: Option<GraphOverscroll>,
    scrollbar: slider::Lifecycle,
    clipboard: Vec<KeyframePoint>,
    external_clipboard: bool,
    text_interpolation: bool,
    snap_enabled: bool,
    snap_radius_px: f64,
    accent_color: Color,
    viewport_width: f64,
    pending_step_graph: Option<KeyframeGraph>,
}

pub struct FrameGraphComponents {
    states: Vec<FrameGraphState>,
    active_component: usize,
}

impl FrameGraphComponents {
    pub fn new(states: Vec<FrameGraphState>, active_component: usize) -> Self {
        assert!(
            !states.is_empty(),
            "frame graph needs at least one component"
        );
        assert!(
            active_component < states.len(),
            "frame graph component is out of bounds"
        );
        Self {
            states,
            active_component,
        }
    }

    pub fn single(state: FrameGraphState) -> Self {
        Self::new(vec![state], 0)
    }

    pub fn constant_values(values: &[f64], active_component: usize) -> Self {
        Self::new(
            values
                .iter()
                .copied()
                .map(FrameGraphState::constant)
                .collect(),
            active_component,
        )
    }

    pub fn active_component(&self) -> usize {
        self.active_component
    }

    pub fn activate(&mut self, component: usize) {
        assert!(
            component < self.states.len(),
            "frame graph component is out of bounds"
        );
        self.active_component = component;
    }

    pub fn replace_component_graph(&mut self, component: usize, graph: KeyframeGraph) {
        self.states
            .get_mut(component)
            .expect("frame graph component is out of bounds")
            .replace_graph(graph);
    }

    pub fn set_item_range(&mut self, item_range: GraphDomain) {
        for state in &mut self.states {
            state.set_item_range(item_range);
        }
    }

    pub fn set_frame_step(&mut self, frame_step: Time) {
        for state in &mut self.states {
            state.set_frame_step(frame_step);
        }
    }

    pub fn set_playhead(&mut self, playhead: Time) {
        for state in &mut self.states {
            state.set_playhead(playhead);
        }
    }

    pub fn set_snapping(&mut self, enabled: bool, radius_px: f64) {
        for state in &mut self.states {
            state.set_snapping(enabled, radius_px);
        }
    }

    pub fn set_external_clipboard(&mut self, enabled: bool) {
        for state in &mut self.states {
            state.set_external_clipboard(enabled);
        }
    }

    pub fn set_text_interpolation(&mut self, enabled: bool) {
        for state in &mut self.states {
            state.set_text_interpolation(enabled);
        }
    }

    pub fn active_actions(
        &mut self,
        action: impl FnOnce(&mut FrameGraphState) -> Vec<FrameGraphAction>,
    ) -> Vec<FrameGraphComponentAction> {
        let component = self.active_component;
        action(&mut self.states[component])
            .into_iter()
            .map(|action| FrameGraphComponentAction { component, action })
            .collect()
    }

    pub fn set_component_values(
        &mut self,
        active_component: usize,
        values: &[(usize, f64)],
    ) -> Vec<FrameGraphComponentAction> {
        self.activate(active_component);
        values
            .iter()
            .flat_map(|&(component, value)| {
                let state = self
                    .states
                    .get_mut(component)
                    .expect("frame graph component is out of bounds");
                state
                    .set_value(value)
                    .into_iter()
                    .map(move |action| FrameGraphComponentAction { component, action })
            })
            .collect()
    }

    pub fn reconcile_component_step_moves(
        &mut self,
        component: usize,
        moves: &[(Time, Time, Time)],
    ) {
        self.states
            .get_mut(component)
            .expect("frame graph component is out of bounds")
            .reconcile_step_moves(moves);
    }

    pub fn rollback_component_step_moves(&mut self, component: usize, moves: &[(Time, Time)]) {
        self.states
            .get_mut(component)
            .expect("frame graph component is out of bounds")
            .rollback_step_moves(moves);
    }
}

impl Deref for FrameGraphComponents {
    type Target = FrameGraphState;

    fn deref(&self) -> &Self::Target {
        &self.states[self.active_component]
    }
}

impl DerefMut for FrameGraphComponents {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.states[self.active_component]
    }
}

impl FrameGraphState {
    pub fn new(
        graph: KeyframeGraph,
        item_range: GraphDomain,
        frame_step: Time,
        playhead: Time,
    ) -> Self {
        assert!(
            item_range.1 >= item_range.0,
            "frame graph range is reversed"
        );
        assert!(frame_step > Time::ZERO, "frame graph step must be positive");
        let minimum_seconds_per_pixel = matches!(graph, KeyframeGraph::Step { .. }).then(|| {
            frame_step.as_secs_f64() / shrimply_discrete_keyframe_graph_core::MAX_FRAME_WIDTH
        });
        Self {
            graph,
            item_range,
            frame_step,
            playhead: playhead.clamp(item_range.0, item_range.1),
            selected_keys: Vec::new(),
            focused_key: None,
            view: GraphViewState {
                minimum_seconds_per_pixel,
                ..GraphViewState::default()
            },
            pointer: None,
            selection_box: None,
            active_drag: None,
            overscroll: None,
            scrollbar: slider::Lifecycle::default(),
            clipboard: Vec::new(),
            external_clipboard: false,
            text_interpolation: false,
            snap_enabled: true,
            snap_radius_px: SNAP_RADIUS_PX,
            accent_color: Color::<u8>::new(0x35, 0x84, 0xe4, 0xff).into(),
            viewport_width: 1.0,
            pending_step_graph: None,
        }
    }

    pub fn constant(value: f64) -> Self {
        Self::new(
            KeyframeGraph::RawValue {
                points: Vec::new(),
                segments: Vec::new(),
                static_value: value,
            },
            (Time::ZERO, Time::from_seconds(1)),
            Time::from_fraction(1, 30),
            Time::ZERO,
        )
    }

    pub fn draw(&mut self, painter: &TimelinePainter, width: f64, height: f64) {
        self.viewport_width = width.max(1.0);
        self.apply_scroll_animation(width);
        let content_height = graph_content_height(height);
        let domain = self.domain(width);
        let base_scrollbar = self.scrollbar(width, height);
        let pointer = self.pointer.map(|(x, y)| vec2(x as f32, y as f32));
        let scrollbar = self.scrollbar.frame(base_scrollbar, pointer).scrollbar;
        let overscroll = self.overscroll.and_then(|overscroll| {
            let distance = shrimply_skia_adw_core::overshoot_distance(
                overscroll.distance,
                overscroll.started_at.elapsed(),
            );
            (distance > shrimply_skia_adw_core::OVERSHOOT_VISIBLE_DISTANCE)
                .then_some((overscroll.edge, distance))
        });
        if overscroll.is_none() {
            self.overscroll = None;
        }
        draw_keyframes(KeyframeGraphDraw {
            painter,
            width,
            height,
            content_height,
            graph: &self.graph,
            domain,
            frame_step: self.frame_step,
            scrollbar,
            overscroll,
            playhead: self.playhead,
            virtual_playhead: None,
            selected_keys: &self.selected_keys,
            focused_key: self.focused_key,
            accent_color: self.accent_color,
        });
        if let Some(selection_box) = self.selection_box {
            draw_selection_box(painter, selection_box, content_height);
        }
    }

    pub fn status(&self) -> FrameGraphStatus {
        let times = self.graph.key_times();
        FrameGraphStatus {
            can_previous: previous_key(&times, self.playhead).is_some(),
            can_next: next_key(&times, self.playhead).is_some(),
            key_at_playhead: key_at(&times, self.playhead).is_some(),
            value: self.current_value(),
        }
    }

    pub fn graph(&self) -> &KeyframeGraph {
        &self.graph
    }

    pub fn preferred_height(&self) -> i32 {
        if matches!(self.graph, KeyframeGraph::Step { .. }) {
            shrimply_discrete_keyframe_graph_core::CONTENT_HEIGHT + GRAPH_SLIDER_HEIGHT as i32
        } else {
            FRAME_GRAPH_HEIGHT
        }
    }

    pub fn replace_graph(&mut self, graph: KeyframeGraph) {
        self.view.minimum_seconds_per_pixel =
            matches!(graph, KeyframeGraph::Step { .. }).then(|| {
                self.frame_step.as_secs_f64()
                    / shrimply_discrete_keyframe_graph_core::MAX_FRAME_WIDTH
            });
        self.graph = graph;
        self.pending_step_graph = None;
        self.retain_valid_selection();
    }

    pub fn set_item_range(&mut self, item_range: GraphDomain) {
        assert!(
            item_range.1 >= item_range.0,
            "frame graph range is reversed"
        );
        self.item_range = item_range;
        self.playhead = self.playhead.clamp(item_range.0, item_range.1);
        self.view.clamp(item_range, self.viewport_width);
    }

    pub fn set_frame_step(&mut self, frame_step: Time) {
        assert!(frame_step > Time::ZERO, "frame graph step must be positive");
        self.frame_step = frame_step;
        if matches!(self.graph, KeyframeGraph::Step { .. }) {
            self.view.minimum_seconds_per_pixel = Some(
                frame_step.as_secs_f64() / shrimply_discrete_keyframe_graph_core::MAX_FRAME_WIDTH,
            );
        }
    }

    pub fn set_playhead(&mut self, value: Time) {
        self.playhead = value.clamp(self.item_range.0, self.item_range.1);
    }

    pub fn set_snapping(&mut self, enabled: bool, radius_px: f64) {
        assert!(radius_px >= 0.0, "frame graph snap radius is negative");
        self.snap_enabled = enabled;
        self.snap_radius_px = radius_px;
    }

    pub fn set_external_clipboard(&mut self, enabled: bool) {
        self.external_clipboard = enabled;
    }

    pub fn set_text_interpolation(&mut self, enabled: bool) {
        self.text_interpolation = enabled;
    }

    fn reconcile_step_moves(&mut self, moves: &[(Time, Time, Time)]) {
        let mut graph = self
            .pending_step_graph
            .take()
            .expect("step graph move has no authoritative source");
        let KeyframeGraph::Step { points } = &mut graph else {
            panic!("only step graphs can reconcile canonical step keyframe times");
        };
        let mut moved = Vec::with_capacity(moves.len());
        for (index, &(old_time, _, _)) in moves.iter().enumerate() {
            assert!(
                !moves[..index]
                    .iter()
                    .any(|(previous, _, _)| previous.approx_eq(old_time)),
                "reconciled step keyframe move is duplicated"
            );
            moved.push(
                points
                    .iter()
                    .find(|point| point.time.approx_eq(old_time))
                    .copied()
                    .expect("reconciled step keyframe is missing"),
            );
        }
        points.retain(|point| {
            !moves
                .iter()
                .any(|(old_time, _, _)| point.time.approx_eq(*old_time))
        });
        let mut destinations = Vec::with_capacity(moved.len());
        for (mut point, &(_, _, time)) in moved.into_iter().zip(moves) {
            points.retain(|other| !other.time.approx_eq(time));
            destinations.retain(|other: &KeyframePoint| !other.time.approx_eq(time));
            point.time = time;
            destinations.push(point);
        }
        points.extend(destinations);
        points.sort_by_key(|point| point.time);
        self.graph = graph;
        let reconciled = |source: Time| {
            moves
                .iter()
                .find_map(|(_, raw_time, time)| raw_time.approx_eq(source).then_some(*time))
                .unwrap_or(source)
        };
        for selected in &mut self.selected_keys {
            *selected = reconciled(*selected);
        }
        self.focused_key = self.focused_key.map(reconciled);
        if let Some(ActiveDrag {
            target: DragTarget::Point(active),
            ..
        }) = &mut self.active_drag
        {
            *active = reconciled(*active);
        }
        self.selected_keys.sort();
        self.selected_keys
            .dedup_by(|left, right| left.approx_eq(*right));
    }

    fn rollback_step_moves(&mut self, moves: &[(Time, Time)]) {
        self.graph = self
            .pending_step_graph
            .take()
            .expect("step graph move has no authoritative source");
        let restored = |source: Time| {
            moves
                .iter()
                .find_map(|(time, raw_time)| raw_time.approx_eq(source).then_some(*time))
                .unwrap_or(source)
        };
        for selected in &mut self.selected_keys {
            *selected = restored(*selected);
        }
        self.focused_key = self.focused_key.map(restored);
        if let Some(ActiveDrag {
            target: DragTarget::Point(active),
            ..
        }) = &mut self.active_drag
        {
            *active = restored(*active);
        }
        self.retain_valid_selection();
    }

    pub fn set_value(&mut self, value: f64) -> Vec<FrameGraphAction> {
        if let Some(time) = key_at(&self.graph.key_times(), self.playhead) {
            update_graph_point(&mut self.graph, time, time, value);
            return vec![FrameGraphAction::KeysMoved(vec![FrameGraphKeyMove {
                old_time: time,
                time,
                value,
            }])];
        }
        let point = KeyframePoint {
            time: self.playhead,
            value,
        };
        insert_graph_key(&mut self.graph, point);
        self.selected_keys = vec![point.time];
        self.focused_key = Some(point.time);
        vec![FrameGraphAction::KeyAdded(point)]
    }

    pub fn pointer_moved(&mut self, x: f64, y: f64) {
        self.pointer = Some((x, y));
    }

    pub fn pointer_left(&mut self) {
        self.pointer = None;
    }

    pub fn begin_pointer(
        &mut self,
        button: FrameGraphPointerButton,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        modifiers: FrameGraphModifiers,
    ) -> Vec<FrameGraphAction> {
        self.pointer_moved(x, y);
        let start_scroll_seconds = self.view.scroll_seconds;
        if button == FrameGraphPointerButton::Middle {
            self.scrollbar.cancel_scroll();
            self.active_drag = Some(ActiveDrag {
                target: DragTarget::Pan,
                start_x: x,
                start_scroll_seconds,
            });
            return Vec::new();
        }
        if button == FrameGraphPointerButton::Secondary {
            let content_height = graph_content_height(height);
            return if let Some((owner_id, interpolation)) =
                self.hit_segment(width, content_height, x, y)
            {
                vec![FrameGraphAction::InterpolationRequested {
                    owner_id,
                    interpolation,
                    x,
                    y,
                }]
            } else if self.text_interpolation
                && y > CURSOR_LANE_HEIGHT
                && y < content_height
                && let Some(owner_id) = self.segment_owner_at_x(width, x)
            {
                vec![FrameGraphAction::TextInterpolationRequested { owner_id, x, y }]
            } else {
                Vec::new()
            };
        }

        let domain = self.domain(width);
        if let Some(scrollbar) = self.scrollbar(width, height) {
            let mut scroll_seconds = self.view.scroll_seconds;
            if self
                .scrollbar
                .begin(scrollbar, vec2(x as f32, y as f32), |value| {
                    scroll_seconds = self.item_range.0.as_secs_f64() + value;
                })
                == slider::Begin::Drag
            {
                self.view.scroll_seconds = scroll_seconds;
                self.active_drag = Some(ActiveDrag {
                    target: DragTarget::Scrollbar,
                    start_x: x,
                    start_scroll_seconds,
                });
                return Vec::new();
            }
        }

        let content_height = graph_content_height(height);
        let target = if y <= CURSOR_LANE_HEIGHT {
            Some(DragTarget::Cursor)
        } else {
            hit_graph_point(
                &self.graph,
                domain,
                width,
                content_height,
                self.frame_step,
                x,
                y,
            )
            .map(|point| DragTarget::Point(point.time))
        };
        let mut actions = Vec::new();
        match target {
            Some(DragTarget::Cursor) => actions.extend(self.seek_pointer(x, width)),
            Some(DragTarget::Point(time)) => {
                if modifiers.control || modifiers.shift {
                    add_key_to_selection(&mut self.selected_keys, &mut self.focused_key, time);
                } else if !key_is_selected(&self.selected_keys, time) {
                    set_key_selection(
                        &mut self.selected_keys,
                        &mut self.focused_key,
                        vec![time],
                        Some(time),
                    );
                } else {
                    self.focused_key = Some(time);
                }
            }
            None if y > CURSOR_LANE_HEIGHT && y < content_height => {
                if !modifiers.control {
                    self.selected_keys.clear();
                    self.focused_key = None;
                }
                self.selection_box = Some(GraphSelectionBox {
                    start_x: x,
                    start_y: y,
                    end_x: x,
                    end_y: y,
                    add_to_selection: modifiers.control,
                });
            }
            _ => {}
        }
        self.active_drag = Some(ActiveDrag {
            target: target.unwrap_or(DragTarget::SelectBox),
            start_x: x,
            start_scroll_seconds,
        });
        actions
    }

    pub fn update_pointer(
        &mut self,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    ) -> Vec<FrameGraphAction> {
        self.pointer_moved(x, y);
        let Some(drag) = self.active_drag else {
            return Vec::new();
        };
        match drag.target {
            DragTarget::Scrollbar => {
                let Some(scrollbar) = self.scrollbar(width, height) else {
                    return Vec::new();
                };
                let mut scroll_seconds = self.view.scroll_seconds;
                if self
                    .scrollbar
                    .drag_by(scrollbar, x - drag.start_x, |value| {
                        scroll_seconds = self.item_range.0.as_secs_f64() + value;
                    })
                {
                    self.view.scroll_seconds = scroll_seconds;
                    self.view.clamp(self.item_range, width);
                }
                Vec::new()
            }
            DragTarget::Pan => {
                let target =
                    drag.start_scroll_seconds - self.view.seconds_per_pixel * (x - drag.start_x);
                self.set_scroll(width, target);
                Vec::new()
            }
            DragTarget::Cursor => self.drag_cursor(x, width),
            DragTarget::SelectBox => {
                let Some(mut selection_box) = self.selection_box else {
                    return Vec::new();
                };
                selection_box.end_x = x;
                selection_box.end_y = y;
                let domain = self.domain(width);
                let selected = select_keys_in_box(
                    &self.graph,
                    domain,
                    width,
                    graph_content_height(height),
                    self.frame_step,
                    selection_box,
                    &self.selected_keys,
                );
                let focused = selected.last().copied();
                set_key_selection(
                    &mut self.selected_keys,
                    &mut self.focused_key,
                    selected,
                    focused,
                );
                self.selection_box = Some(selection_box);
                Vec::new()
            }
            DragTarget::Point(focus_time) => {
                let domain = self.domain(width);
                let range = graph_range(&self.graph);
                let point_x = if matches!(self.graph, KeyframeGraph::Step { .. }) {
                    x - shrimply_discrete_keyframe_graph_core::frame_width(
                        width,
                        domain,
                        self.frame_step,
                    ) / 2.0
                } else {
                    x
                };
                let requested_time = clamp_graph_time(
                    snap_keyframe_time(
                        time_at_x(point_x, width, domain),
                        self.item_range,
                        self.snap_enabled,
                        self.snap_radius_px,
                        graph_duration_seconds(domain) / graph_plot_width(width),
                        self.playhead,
                    ),
                    self.item_range,
                );
                let requested_value = graph_edit_value(
                    &self.graph,
                    value_at_y(y, graph_content_height(height), range),
                );
                let authoritative_step_graph =
                    matches!(self.graph, KeyframeGraph::Step { .. }).then(|| self.graph.clone());
                let (updates, selected, focused) = move_selected_graph_points(
                    &mut self.graph,
                    &self.selected_keys,
                    focus_time,
                    requested_time,
                    requested_value,
                    self.item_range,
                );
                self.pending_step_graph = (!updates.is_empty())
                    .then_some(authoritative_step_graph)
                    .flatten();
                self.selected_keys = selected;
                self.focused_key = Some(focused);
                if let Some(active) = self.active_drag.as_mut() {
                    active.target = DragTarget::Point(focused);
                }
                vec![FrameGraphAction::KeysMoved(
                    updates
                        .into_iter()
                        .map(|(old_time, time, value)| FrameGraphKeyMove {
                            old_time,
                            time,
                            value,
                        })
                        .collect(),
                )]
            }
        }
    }

    pub fn end_pointer(&mut self) -> Vec<FrameGraphAction> {
        let edited = matches!(
            self.active_drag,
            Some(ActiveDrag {
                target: DragTarget::Point(_),
                ..
            })
        );
        self.scrollbar.end_drag();
        self.selection_box = None;
        self.active_drag = None;
        self.pending_step_graph = None;
        edited
            .then_some(FrameGraphAction::EditFinished)
            .into_iter()
            .collect()
    }

    pub fn scroll(
        &mut self,
        dx: f64,
        dy: f64,
        pointer: FrameGraphPointerPosition,
        control: bool,
        input: FrameGraphScrollInput,
    ) -> bool {
        let FrameGraphPointerPosition {
            x: pointer_x,
            y: pointer_y,
            width,
            height,
        } = pointer;
        self.pointer_moved(pointer_x, pointer_y);
        let delta = if dx.abs() > f64::EPSILON { dx } else { dy };
        if delta.abs() <= f64::EPSILON {
            return false;
        }
        self.view.initialize(self.item_range, width);
        self.view.clamp(self.item_range, width);
        if !control && self.scroll_should_propagate(width, delta) {
            self.overscroll = None;
            return false;
        }
        if !control && let Some(scrollbar) = self.scrollbar(width, height) {
            let mut scroll_seconds = self.view.scroll_seconds;
            let event = self.scrollbar.scroll_at(
                scrollbar,
                Some(vec2(pointer_x as f32, pointer_y as f32)),
                delta,
                input,
                |value| {
                    scroll_seconds = self.item_range.0.as_secs_f64() + value;
                },
            );
            if event.handled {
                self.view.scroll_seconds = scroll_seconds;
                self.view.clamp(self.item_range, width);
                return true;
            }
        }
        if control {
            self.zoom(delta, pointer_x, width);
        } else {
            let units = match input {
                FrameGraphScrollInput::Wheel => delta * WHEEL_UNITS_PER_STEP,
                FrameGraphScrollInput::Surface => delta,
            };
            let target = self.view.scroll_seconds + self.view.seconds_per_pixel * units;
            self.set_scroll(width, target);
        }
        true
    }

    pub fn key(&mut self, key: FrameGraphKey) -> Vec<FrameGraphAction> {
        match key {
            FrameGraphKey::PreviousFrame => {
                self.set_playhead(self.playhead.saturating_sub(self.frame_step));
                vec![FrameGraphAction::PlayheadChanged(self.playhead)]
            }
            FrameGraphKey::NextFrame => {
                self.set_playhead(self.playhead.saturating_add(self.frame_step));
                vec![FrameGraphAction::PlayheadChanged(self.playhead)]
            }
            FrameGraphKey::Start => {
                self.set_playhead(self.item_range.0);
                vec![FrameGraphAction::PlayheadChanged(self.playhead)]
            }
            FrameGraphKey::End => {
                self.set_playhead(self.item_range.1);
                vec![FrameGraphAction::PlayheadChanged(self.playhead)]
            }
            FrameGraphKey::ZoomIn => {
                self.zoom(-1.0, self.viewport_width / 2.0, self.viewport_width);
                Vec::new()
            }
            FrameGraphKey::ZoomOut => {
                self.zoom(1.0, self.viewport_width / 2.0, self.viewport_width);
                Vec::new()
            }
            FrameGraphKey::PreviousKey => self.previous_key(),
            FrameGraphKey::NextKey => self.next_key(),
            FrameGraphKey::Delete => self.delete_selected(),
            FrameGraphKey::Copy => {
                let selected = self.selected_keys.clone();
                if !self.external_clipboard {
                    self.copy_selected();
                }
                vec![FrameGraphAction::CopyRequested(selected)]
            }
            FrameGraphKey::Paste if self.external_clipboard => {
                vec![FrameGraphAction::PasteRequested(self.playhead)]
            }
            FrameGraphKey::Paste => self.paste(),
            FrameGraphKey::TogglePlayback => vec![FrameGraphAction::TogglePlayback],
        }
    }

    pub fn previous_key(&mut self) -> Vec<FrameGraphAction> {
        let Some(time) = previous_key(&self.graph.key_times(), self.playhead) else {
            return Vec::new();
        };
        self.set_playhead(time);
        vec![FrameGraphAction::PlayheadChanged(time)]
    }

    pub fn next_key(&mut self) -> Vec<FrameGraphAction> {
        let Some(time) = next_key(&self.graph.key_times(), self.playhead) else {
            return Vec::new();
        };
        self.set_playhead(time);
        vec![FrameGraphAction::PlayheadChanged(time)]
    }

    pub fn toggle_key(&mut self) -> Vec<FrameGraphAction> {
        if let Some(time) = key_at(&self.graph.key_times(), self.playhead) {
            delete_graph_key(&mut self.graph, time);
            self.selected_keys
                .retain(|selected| !selected.approx_eq(time));
            self.focused_key = None;
            return vec![FrameGraphAction::KeysDeleted(vec![time])];
        }
        let point = KeyframePoint {
            time: self.playhead,
            value: self.current_value(),
        };
        insert_graph_key(&mut self.graph, point);
        self.selected_keys = vec![point.time];
        self.focused_key = Some(point.time);
        vec![FrameGraphAction::KeyAdded(point)]
    }

    pub fn delete_selected(&mut self) -> Vec<FrameGraphAction> {
        let mut times = self.selected_keys.clone();
        if times.is_empty()
            && let Some(time) = key_at(&self.graph.key_times(), self.playhead)
        {
            times.push(time);
        }
        for time in &times {
            delete_graph_key(&mut self.graph, *time);
        }
        self.selected_keys.clear();
        self.focused_key = None;
        (!times.is_empty())
            .then_some(FrameGraphAction::KeysDeleted(times))
            .into_iter()
            .collect()
    }

    pub fn set_interpolation(&mut self, owner_id: Uuid, interpolation: Interpolation) {
        match &mut self.graph {
            KeyframeGraph::RawValue { segments, .. } => {
                if let Some(segment) = segments
                    .iter_mut()
                    .find(|segment| segment.owner_id == owner_id)
                {
                    segment.interpolation = interpolation;
                }
            }
            KeyframeGraph::Speed { segments, .. } => {
                if let Some(segment) = segments
                    .iter_mut()
                    .find(|segment| segment.owner_id == owner_id)
                {
                    segment.interpolation = interpolation;
                }
            }
            KeyframeGraph::Step { .. } => {}
        }
    }

    pub fn is_animating(&self) -> bool {
        self.overscroll.is_some() || self.scrollbar.visual_animating()
    }

    fn current_value(&self) -> f64 {
        if let KeyframeGraph::Step { points } = &self.graph {
            return points
                .iter()
                .rev()
                .find(|point| point.time <= self.playhead)
                .or_else(|| points.first())
                .map_or(0.0, |point| point.value);
        }
        self.focused_key
            .and_then(|time| graph_key_point(&self.graph, time))
            .or_else(|| {
                graph_key_points(&self.graph)
                    .into_iter()
                    .min_by_key(|point| point.time.abs_diff(self.playhead))
            })
            .map_or(0.0, |point| point.value)
    }

    fn domain(&mut self, width: f64) -> GraphDomain {
        self.view.initialize(self.item_range, width);
        self.view.clamp(self.item_range, width);
        self.view.domain(self.item_range, width)
    }

    fn seek_pointer(&mut self, x: f64, width: f64) -> Vec<FrameGraphAction> {
        let time = clamp_graph_time(time_at_x(x, width, self.domain(width)), self.item_range)
            .snapped(self.frame_step);
        self.set_playhead(time);
        vec![FrameGraphAction::PlayheadChanged(self.playhead)]
    }

    fn drag_cursor(&mut self, x: f64, width: f64) -> Vec<FrameGraphAction> {
        let left = GRAPH_PAD;
        let right = (width - GRAPH_PAD).max(left);
        let target = if x < left {
            self.view.scroll_seconds - self.view.seconds_per_pixel * (left - x)
        } else if x > right {
            self.view.scroll_seconds + self.view.seconds_per_pixel * (x - right)
        } else {
            self.view.scroll_seconds
        };
        self.set_scroll(width, target);
        self.seek_pointer(x.clamp(left, right), width)
    }

    fn zoom(&mut self, delta: f64, pointer_x: f64, width: f64) {
        self.view.initialize(self.item_range, width);
        self.view.clamp(self.item_range, width);
        let domain = self.view.domain(self.item_range, width);
        let pointer_time = time_at_x(pointer_x, width, domain);
        let scale = if delta < 0.0 { 0.8 } else { 1.25 };
        let pointer_plot_x = (pointer_x - GRAPH_PAD).clamp(0.0, graph_plot_width(width));
        self.view.seconds_per_pixel *= scale;
        self.view.scroll_seconds =
            pointer_time.as_secs_f64() - pointer_plot_x * self.view.seconds_per_pixel;
        self.view.clamp(self.item_range, width);
        self.overscroll = None;
    }

    fn set_scroll(&mut self, width: f64, target: f64) {
        self.view.initialize(self.item_range, width);
        if !self.can_scroll(width) {
            self.view.clamp(self.item_range, width);
            self.overscroll = None;
            return;
        }
        let visible = graph_plot_width(width) * self.view.seconds_per_pixel;
        let minimum = self.item_range.0.as_secs_f64();
        let maximum = (self.item_range.1.as_secs_f64() - visible).max(minimum);
        let edge = if target < minimum {
            Some((Edge::Left, minimum - target))
        } else if target > maximum {
            Some((Edge::Right, target - maximum))
        } else {
            None
        };
        self.view.scroll_seconds = target.clamp(minimum, maximum);
        self.overscroll = edge.map(|(edge, distance)| GraphOverscroll {
            edge,
            started_at: Instant::now(),
            distance: (distance / self.view.seconds_per_pixel)
                .clamp(1.0, shrimply_skia_adw_core::OVERSHOOT_MAX_DISTANCE),
        });
    }

    fn can_scroll(&self, width: f64) -> bool {
        graph_plot_width(width) * self.view.seconds_per_pixel
            < graph_duration_seconds(self.item_range)
    }

    fn scroll_should_propagate(&self, width: f64, delta: f64) -> bool {
        if !self.can_scroll(width) {
            return true;
        }
        let visible = graph_plot_width(width) * self.view.seconds_per_pixel;
        let maximum = self.item_range.1.as_secs_f64() - visible;
        let tolerance = self.view.seconds_per_pixel / 2.0;
        (delta < 0.0 && self.view.scroll_seconds <= self.item_range.0.as_secs_f64() + tolerance)
            || (delta > 0.0 && self.view.scroll_seconds >= maximum - tolerance)
    }

    fn scrollbar(&self, width: f64, height: f64) -> Option<Scrollbar> {
        if !self.can_scroll(width) {
            return None;
        }
        let visible = graph_plot_width(width) * self.view.seconds_per_pixel;
        let duration = graph_duration_seconds(self.item_range);
        Some(Scrollbar {
            axis: Axis::Horizontal,
            bounds: shrimply_skia_adw_core::Rect::from_xywh(
                0.0,
                graph_content_height(height) as f32,
                width.max(0.0) as f32,
                GRAPH_SLIDER_HEIGHT as f32,
            ),
            content_length: duration.max(visible),
            viewport_length: visible,
            value: (self.view.scroll_seconds - self.item_range.0.as_secs_f64()).max(0.0),
            color: Color::LIGHT1,
            outline_color: Color::<f32>::from_rgb8_alpha(0x00, 0x00, 0x0c, 0.95),
            state: slider::idle_state(),
        })
    }

    fn apply_scroll_animation(&mut self, width: f64) {
        let mut scroll_seconds = self.view.scroll_seconds;
        if self.scrollbar.apply_scroll(|value| {
            scroll_seconds = self.item_range.0.as_secs_f64() + value;
        }) {
            self.view.scroll_seconds = scroll_seconds;
            self.view.clamp(self.item_range, width);
        }
    }

    fn copy_selected(&mut self) {
        let points = graph_key_points(&self.graph);
        let mut copied: Vec<_> = points
            .into_iter()
            .filter(|point| key_is_selected(&self.selected_keys, point.time))
            .collect();
        copied.sort_by_key(|point| point.time);
        let Some(origin) = copied.first().map(|point| point.time) else {
            self.clipboard.clear();
            return;
        };
        for point in &mut copied {
            point.time = Time {
                seconds: point.time.seconds - origin.seconds,
            };
        }
        self.clipboard = copied;
    }

    fn paste(&mut self) -> Vec<FrameGraphAction> {
        if self.clipboard.is_empty() {
            return Vec::new();
        }
        let mut added = Vec::new();
        for copied in self.clipboard.clone() {
            let point = KeyframePoint {
                time: Time {
                    seconds: self.playhead.seconds + copied.time.seconds,
                }
                .clamp(self.item_range.0, self.item_range.1),
                value: copied.value,
            };
            insert_graph_key(&mut self.graph, point);
            added.push(point);
        }
        self.selected_keys = added.iter().map(|point| point.time).collect();
        self.focused_key = self.selected_keys.first().copied();
        vec![FrameGraphAction::KeysPasted(added)]
    }

    fn retain_valid_selection(&mut self) {
        let times = self.graph.key_times();
        self.selected_keys
            .retain(|selected| times.iter().any(|time| time.approx_eq(*selected)));
        self.focused_key = self.focused_key.filter(|focused| {
            self.selected_keys
                .iter()
                .any(|time| time.approx_eq(*focused))
        });
    }

    fn hit_segment(
        &mut self,
        width: f64,
        height: f64,
        x: f64,
        y: f64,
    ) -> Option<(Uuid, Interpolation)> {
        let domain = self.domain(width);
        let mut closest = None;
        match &self.graph {
            KeyframeGraph::Step { .. } => return None,
            KeyframeGraph::RawValue {
                points, segments, ..
            } => {
                let range = raw_range(points, segments);
                for segment in segments {
                    let mut previous = None;
                    for progress in crate::curve_sample_progresses(segment.interpolation) {
                        let time = Time::from_seconds_f64(
                            segment.start.as_secs_f64()
                                + (segment.end.as_secs_f64() - segment.start.as_secs_f64())
                                    * progress,
                        );
                        let value = segment.start_value
                            + (segment.end_value - segment.start_value)
                                * segment.interpolation.value(progress);
                        let point = glam::DVec2::new(
                            time_x(time, width, domain),
                            value_y(value, height, range),
                        );
                        if let Some(start) = previous {
                            let distance = shrimply_math_geometry::distance_to_dsegment(
                                glam::DVec2::new(x, y),
                                start,
                                point,
                            );
                            if closest
                                .as_ref()
                                .is_none_or(|(current, _, _)| distance < *current)
                            {
                                closest = Some((distance, segment.owner_id, segment.interpolation));
                            }
                        }
                        previous = Some(point);
                    }
                }
            }
            KeyframeGraph::Speed { segments, .. } => {
                let range = speed_range(segments);
                for segment in segments {
                    let mut previous = None;
                    for progress in crate::curve_sample_progresses(segment.interpolation) {
                        let Some(speed) = segment_speed_at(segment, progress) else {
                            previous = None;
                            continue;
                        };
                        let time = Time::from_seconds_f64(
                            segment.start.as_secs_f64()
                                + (segment.end.as_secs_f64() - segment.start.as_secs_f64())
                                    * progress,
                        );
                        let point = glam::DVec2::new(
                            time_x(time, width, domain),
                            value_y(speed, height, range),
                        );
                        if let Some(start) = previous {
                            let distance = shrimply_math_geometry::distance_to_dsegment(
                                glam::DVec2::new(x, y),
                                start,
                                point,
                            );
                            if closest
                                .as_ref()
                                .is_none_or(|(current, _, _)| distance < *current)
                            {
                                closest = Some((distance, segment.owner_id, segment.interpolation));
                            }
                        }
                        previous = Some(point);
                    }
                }
            }
        }
        closest
            .filter(|(distance, _, _)| *distance <= HIT_RADIUS)
            .map(|(_, owner_id, interpolation)| (owner_id, interpolation))
    }

    fn segment_owner_at_x(&mut self, width: f64, x: f64) -> Option<Uuid> {
        let domain = self.domain(width);
        let contains = |start: Time, end: Time| {
            let start = time_x(start, width, domain);
            let end = time_x(end, width, domain);
            x > start.min(end) && x < start.max(end)
        };
        match &self.graph {
            KeyframeGraph::Step { .. } => None,
            KeyframeGraph::RawValue { segments, .. } => segments
                .iter()
                .find(|segment| contains(segment.start, segment.end))
                .map(|segment| segment.owner_id),
            KeyframeGraph::Speed { segments, .. } => segments
                .iter()
                .find(|segment| contains(segment.start, segment.end))
                .map(|segment| segment.owner_id),
        }
    }
}

fn graph_content_height(height: f64) -> f64 {
    (height - GRAPH_SLIDER_HEIGHT).max(1.0)
}

fn graph_plot_width(width: f64) -> f64 {
    (width - GRAPH_PAD * 2.0).max(1.0)
}

fn graph_duration_seconds((start, end): GraphDomain) -> f64 {
    (end.as_secs_f64() - start.as_secs_f64()).max(0.001)
}

fn minimum_scale(duration: f64) -> f64 {
    (duration / MINIMUM_VISIBLE_PIXELS).max(MINIMUM_SECONDS_PER_PIXEL)
}

fn time_at_x(x: f64, width: f64, domain: GraphDomain) -> Time {
    let progress = ((x - GRAPH_PAD) / graph_plot_width(width)).clamp(0.0, 1.0);
    Time::from_seconds_f64(
        domain.0.as_secs_f64() + (domain.1.as_secs_f64() - domain.0.as_secs_f64()) * progress,
    )
}

fn value_at_y(y: f64, height: f64, (minimum, maximum): (f64, f64)) -> f64 {
    let span = (maximum - minimum).max(1.0);
    minimum + (height - GRAPH_PAD - y) / (height - GRAPH_PAD * 2.0) * span
}

fn graph_range(graph: &KeyframeGraph) -> (f64, f64) {
    match graph {
        KeyframeGraph::Step { .. } => STEP_GRAPH_RANGE,
        KeyframeGraph::RawValue {
            points, segments, ..
        } => raw_range(points, segments),
        KeyframeGraph::Speed { segments, .. } => speed_range(segments),
    }
}

fn graph_edit_value(graph: &KeyframeGraph, value: f64) -> f64 {
    match graph {
        KeyframeGraph::Step { .. } => value.clamp(0.0, 1.0),
        KeyframeGraph::RawValue { .. } | KeyframeGraph::Speed { .. } => value,
    }
}

fn key_at(times: &[Time], playhead: Time) -> Option<Time> {
    times.iter().copied().find(|time| time.approx_eq(playhead))
}

fn previous_key(times: &[Time], playhead: Time) -> Option<Time> {
    times
        .iter()
        .copied()
        .rev()
        .find(|time| *time < playhead && !time.approx_eq(playhead))
}

fn next_key(times: &[Time], playhead: Time) -> Option<Time> {
    times
        .iter()
        .copied()
        .find(|time| *time > playhead && !time.approx_eq(playhead))
}

fn graph_key_points(graph: &KeyframeGraph) -> Vec<KeyframePoint> {
    match graph {
        KeyframeGraph::Step { points } | KeyframeGraph::RawValue { points, .. } => points.clone(),
        KeyframeGraph::Speed {
            segments,
            keys,
            static_value,
        } if segments.is_empty() => keys
            .iter()
            .map(|time| KeyframePoint {
                time: *time,
                value: *static_value,
            })
            .collect(),
        KeyframeGraph::Speed { segments, .. } => {
            let mut points = Vec::new();
            for segment in segments {
                points.extend([
                    KeyframePoint {
                        time: segment.start,
                        value: segment_speed_at(segment, 0.0).unwrap_or(0.0),
                    },
                    KeyframePoint {
                        time: segment.end,
                        value: segment_speed_at(segment, 1.0).unwrap_or(0.0),
                    },
                ]);
            }
            points.sort_by_key(|point| point.time);
            points.dedup_by_key(|point| point.time);
            points
        }
    }
}

fn graph_key_point(graph: &KeyframeGraph, time: Time) -> Option<KeyframePoint> {
    graph_key_points(graph)
        .into_iter()
        .find(|point| point.time.approx_eq(time))
}

fn hit_graph_point(
    graph: &KeyframeGraph,
    domain: GraphDomain,
    width: f64,
    height: f64,
    frame_step: Time,
    x: f64,
    y: f64,
) -> Option<KeyframePoint> {
    let points = graph_key_points(graph);
    let range = graph_range(graph);
    points
        .into_iter()
        .filter_map(|point| {
            let (point_x, point_y) = if matches!(graph, KeyframeGraph::Step { .. }) {
                (
                    shrimply_discrete_keyframe_graph_core::key_x(
                        point.time, width, domain, frame_step,
                    ),
                    shrimply_discrete_keyframe_graph_core::key_y(height, CURSOR_LANE_HEIGHT),
                )
            } else {
                raw_point(point, width, height, domain, range)
            };
            let distance = (point_x - x).hypot(point_y - y);
            (distance <= HIT_RADIUS).then_some((distance, point))
        })
        .min_by(|left, right| left.0.total_cmp(&right.0))
        .map(|(_, point)| point)
}

fn set_key_selection(
    selected_keys: &mut Vec<Time>,
    focused_key: &mut Option<Time>,
    mut selected: Vec<Time>,
    focused: Option<Time>,
) {
    selected.sort();
    selected.dedup_by(|left, right| left.approx_eq(*right));
    *focused_key = focused.filter(|time| selected.iter().any(|item| item.approx_eq(*time)));
    *selected_keys = selected;
}

fn add_key_to_selection(selected_keys: &mut Vec<Time>, focused_key: &mut Option<Time>, time: Time) {
    if !key_is_selected(selected_keys, time) {
        selected_keys.push(time);
        selected_keys.sort();
    }
    *focused_key = Some(time);
}

fn key_is_selected(selected_keys: &[Time], time: Time) -> bool {
    selected_keys.iter().any(|item| item.approx_eq(time))
}

fn select_keys_in_box(
    graph: &KeyframeGraph,
    domain: GraphDomain,
    width: f64,
    height: f64,
    frame_step: Time,
    selection_box: GraphSelectionBox,
    previous_selection: &[Time],
) -> Vec<Time> {
    let left = selection_box.start_x.min(selection_box.end_x);
    let right = selection_box.start_x.max(selection_box.end_x);
    let top = selection_box.start_y.min(selection_box.end_y);
    let bottom = selection_box.start_y.max(selection_box.end_y);
    let range = graph_range(graph);
    let mut selected = if selection_box.add_to_selection {
        previous_selection.to_vec()
    } else {
        Vec::new()
    };
    for point in graph_key_points(graph) {
        let (x, y) = if matches!(graph, KeyframeGraph::Step { .. }) {
            (
                shrimply_discrete_keyframe_graph_core::key_x(point.time, width, domain, frame_step),
                shrimply_discrete_keyframe_graph_core::key_y(height, CURSOR_LANE_HEIGHT),
            )
        } else {
            raw_point(point, width, height, domain, range)
        };
        if x >= left
            && x <= right
            && y >= top
            && y <= bottom
            && !key_is_selected(&selected, point.time)
        {
            selected.push(point.time);
        }
    }
    selected.sort();
    selected.dedup_by(|left, right| left.approx_eq(*right));
    selected
}

fn move_selected_graph_points(
    graph: &mut KeyframeGraph,
    selected_times: &[Time],
    focus_time: Time,
    requested_time: Time,
    requested_value: f64,
    item_range: GraphDomain,
) -> (Vec<(Time, Time, f64)>, Vec<Time>, Time) {
    let Some(focus_point) = graph_key_point(graph, focus_time) else {
        return (Vec::new(), selected_times.to_vec(), focus_time);
    };
    let selected_times = if key_is_selected(selected_times, focus_time) {
        selected_times.to_vec()
    } else {
        vec![focus_time]
    };
    let requested_delta = requested_time.as_secs_f64() - focus_time.as_secs_f64();
    let minimum = selected_times.iter().min().copied().unwrap_or(focus_time);
    let maximum = selected_times.iter().max().copied().unwrap_or(focus_time);
    let delta = requested_delta.clamp(
        item_range.0.as_secs_f64() - minimum.as_secs_f64(),
        item_range.1.as_secs_f64() - maximum.as_secs_f64(),
    );
    let delta_value = if matches!(graph, KeyframeGraph::RawValue { .. }) {
        requested_value - focus_point.value
    } else {
        0.0
    };
    let mut updates = Vec::new();
    for old_time in &selected_times {
        if let Some(point) = graph_key_point(graph, *old_time) {
            updates.push((
                point.time,
                Time::from_seconds_f64(point.time.as_secs_f64() + delta),
                point.value + delta_value,
            ));
        }
    }
    if delta > 0.0 {
        updates.sort_by_key(|(old_time, _, _)| std::cmp::Reverse(*old_time));
    } else {
        updates.sort_by_key(|(old_time, _, _)| *old_time);
    }
    for (old_time, time, value) in &updates {
        update_graph_point(graph, *old_time, *time, *value);
    }
    let mut selected: Vec<_> = updates.iter().map(|(_, time, _)| *time).collect();
    selected.sort();
    selected.dedup_by(|left, right| left.approx_eq(*right));
    let focused = Time::from_seconds_f64(focus_time.as_secs_f64() + delta);
    (updates, selected, focused)
}

fn update_graph_point(graph: &mut KeyframeGraph, old_time: Time, time: Time, value: f64) {
    match graph {
        KeyframeGraph::Step { points } => {
            if let Some(index) = points
                .iter_mut()
                .position(|point| point.time.approx_eq(old_time))
            {
                let mut point = points.remove(index);
                points.retain(|other| !other.time.approx_eq(time));
                point.time = time;
                point.value = value;
                points.push(point);
            }
            points.sort_by_key(|point| point.time);
        }
        KeyframeGraph::RawValue {
            points, segments, ..
        } => {
            for point in &mut *points {
                if point.time.approx_eq(old_time) {
                    point.time = time;
                    point.value = value;
                }
            }
            for segment in &mut *segments {
                if segment.start.approx_eq(old_time) {
                    segment.start = time;
                    segment.start_value = value;
                }
                if segment.end.approx_eq(old_time) {
                    segment.end = time;
                    segment.end_value = value;
                }
            }
            points.sort_by_key(|point| point.time);
            segments.sort_by_key(|segment| segment.start);
        }
        KeyframeGraph::Speed { segments, keys, .. } => {
            for key in &mut *keys {
                if key.approx_eq(old_time) {
                    *key = time;
                }
            }
            for segment in &mut *segments {
                if segment.start.approx_eq(old_time) {
                    segment.start = time;
                }
                if segment.end.approx_eq(old_time) {
                    segment.end = time;
                }
            }
            keys.sort();
            segments.sort_by_key(|segment| segment.start);
        }
    }
}

fn insert_graph_key(graph: &mut KeyframeGraph, point: KeyframePoint) {
    match graph {
        KeyframeGraph::Step { points } => {
            points.retain(|existing| !existing.time.approx_eq(point.time));
            points.push(point);
            points.sort_by_key(|point| point.time);
        }
        KeyframeGraph::RawValue {
            points, segments, ..
        } => {
            points.retain(|existing| !existing.time.approx_eq(point.time));
            points.push(point);
            points.sort_by_key(|point| point.time);
            *segments = raw_segments_preserving(points, segments);
        }
        KeyframeGraph::Speed { keys, .. } => {
            keys.retain(|time| !time.approx_eq(point.time));
            keys.push(point.time);
            keys.sort();
        }
    }
}

fn delete_graph_key(graph: &mut KeyframeGraph, time: Time) {
    match graph {
        KeyframeGraph::Step { points } => points.retain(|point| !point.time.approx_eq(time)),
        KeyframeGraph::RawValue {
            points, segments, ..
        } => {
            points.retain(|point| !point.time.approx_eq(time));
            *segments = raw_segments_preserving(points, segments);
        }
        KeyframeGraph::Speed { segments, keys, .. } => {
            keys.retain(|key| !key.approx_eq(time));
            segments
                .retain(|segment| !segment.start.approx_eq(time) && !segment.end.approx_eq(time));
        }
    }
}

fn raw_segments_preserving(points: &[KeyframePoint], previous: &[RawSegment]) -> Vec<RawSegment> {
    points
        .windows(2)
        .map(|points| {
            let owner = previous
                .iter()
                .find(|segment| segment.start.approx_eq(points[0].time));
            let interpolation = owner
                .or_else(|| {
                    previous.iter().find(|segment| {
                        segment.start < points[0].time && segment.end > points[0].time
                    })
                })
                .map_or_else(Interpolation::default, |segment| segment.interpolation);
            RawSegment {
                owner_id: owner.map_or_else(Uuid::new_v4, |segment| segment.owner_id),
                start: points[0].time,
                end: points[1].time,
                start_value: points[0].value,
                end_value: points[1].value,
                interpolation,
            }
        })
        .collect()
}

fn snap_keyframe_time(
    time: Time,
    item_range: GraphDomain,
    enabled: bool,
    radius_px: f64,
    seconds_per_pixel: f64,
    cursor: Time,
) -> Time {
    let time = clamp_graph_time(time, item_range);
    if !enabled {
        return time;
    }
    let radius = seconds_per_pixel * radius_px;
    if cursor >= item_range.0
        && cursor <= item_range.1
        && (cursor.as_secs_f64() - time.as_secs_f64()).abs() <= radius
    {
        cursor
    } else {
        time
    }
}

fn clamp_graph_time(time: Time, range: GraphDomain) -> Time {
    time.clamp(range.0, range.1)
}

fn draw_selection_box(
    painter: &TimelinePainter,
    selection_box: GraphSelectionBox,
    content_height: f64,
) {
    use shrimply_skia_adw_core::canvas::{Rect, Stroke, StrokeKind, vec2};

    let left = selection_box.start_x.min(selection_box.end_x);
    let right = selection_box.start_x.max(selection_box.end_x);
    let top = selection_box
        .start_y
        .min(selection_box.end_y)
        .clamp(CURSOR_LANE_HEIGHT, content_height);
    let bottom = selection_box
        .start_y
        .max(selection_box.end_y)
        .clamp(CURSOR_LANE_HEIGHT, content_height);
    if right <= left || bottom <= top {
        return;
    }
    let bounds = Rect::from_min_size(
        vec2(left as f32, top as f32),
        vec2((right - left) as f32, (bottom - top) as f32),
    );
    painter.rect_filled(
        bounds,
        0,
        Color::<f32>::from_rgb8_alpha(0x61, 0xa7, 0xff, 0.18),
    );
    painter.rect_stroke(
        bounds,
        0,
        Stroke::new(1.0, Color::<f32>::from_rgb8_alpha(0x8c, 0xc3, 0xff, 0.78)),
        StrokeKind::Inside,
    );
}
