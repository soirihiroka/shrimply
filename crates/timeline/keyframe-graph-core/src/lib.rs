use shrimply_skia_adw_core::canvas::{Rect, Stroke, StrokeKind, Vec2, vec2};
use skia_safe::PathBuilder;

use shrimply_interpolation::Interpolation;
use shrimply_math_color::Color;
use shrimply_math_core::{Fraction, Time, fraction_floor_i64};
use shrimply_skia_adw_core::{canvas::TimelinePainter, cursor};
use uuid::Uuid;

mod controller;
pub use controller::{
    FRAME_GRAPH_HEIGHT, FrameGraphAction, FrameGraphComponentAction, FrameGraphComponents,
    FrameGraphKey, FrameGraphKeyMove, FrameGraphModifiers, FrameGraphPointerButton,
    FrameGraphPointerPosition, FrameGraphScrollInput, FrameGraphState, FrameGraphStatus,
    GRAPH_SLIDER_HEIGHT,
};

pub const GRAPH_PAD: f64 = 12.0;
pub const STEP_GRAPH_RANGE: (f64, f64) = (-0.15, 1.15);
pub const CURSOR_LANE_HEIGHT: f64 = 18.0;
const SPEED_CURVE_STEPS: usize = 48;
const CURVE_BREAK_OFFSET: f64 = 1.0 / (SPEED_CURVE_STEPS as f64 * 64.0);

const PLAYHEAD_HANDLE_WIDTH: f64 = 16.0;
const PLAYHEAD_HANDLE_HEIGHT: f64 = 8.0;
const PLAYHEAD_HANDLE_TOP: f64 = 0.0;
const PLAYHEAD_HANDLE_TRIANGLE_HEIGHT: f64 = 5.0;
const FRAME_TICK_MIN_WIDTH: f64 = 8.0;
const GRAPH_GRID_TARGET_PX: f64 = 80.0;
const RULER_TICK_ALPHA: f32 = 0.42;
const SPEED_FILL_ALPHA: f32 = 0.18;
const VIRTUAL_PLAYHEAD_DASH: f64 = 4.0;
const VIRTUAL_PLAYHEAD_GAP: f64 = 4.0;

pub type GraphDomain = (Time, Time);

#[derive(Clone, Copy)]
pub struct KeyframePoint {
    pub time: Time,
    pub value: f64,
}

#[derive(Clone, Copy)]
pub struct RawSegment {
    pub owner_id: Uuid,
    pub start: Time,
    pub end: Time,
    pub start_value: f64,
    pub end_value: f64,
    pub interpolation: Interpolation,
}

#[derive(Clone, Copy)]
pub struct SpeedSegment {
    pub owner_id: Uuid,
    pub start: Time,
    pub end: Time,
    pub value: f64,
    pub interpolation: Interpolation,
}

#[derive(Clone)]
pub enum KeyframeGraph {
    Step {
        points: Vec<KeyframePoint>,
    },
    RawValue {
        points: Vec<KeyframePoint>,
        segments: Vec<RawSegment>,
        static_value: f64,
    },
    Speed {
        segments: Vec<SpeedSegment>,
        keys: Vec<Time>,
        static_value: f64,
    },
}

impl KeyframeGraph {
    pub fn key_times(&self) -> Vec<Time> {
        let mut times: Vec<_> = match self {
            Self::Step { points } => points.iter().map(|point| point.time).collect(),
            Self::RawValue { points, .. } => points.iter().map(|point| point.time).collect(),
            Self::Speed { keys, .. } => keys.clone(),
        };
        times.sort();
        times.dedup_by(|left, right| left.approx_eq(*right));
        times
    }
}

#[derive(Clone, Copy)]
struct GraphFrame<'a> {
    painter: &'a TimelinePainter,
    width: f64,
    height: f64,
    domain: GraphDomain,
    playhead: Time,
    frame_step: Time,
    selected_keys: &'a [Time],
    focused_key: Option<Time>,
    accent_color: Color,
}

pub struct KeyframeGraphDraw<'a> {
    pub painter: &'a TimelinePainter,
    pub width: f64,
    pub height: f64,
    pub content_height: f64,
    pub graph: &'a KeyframeGraph,
    pub domain: GraphDomain,
    pub frame_step: Time,
    pub scrollbar: Option<shrimply_skia_adw_core::Scrollbar>,
    pub overscroll: Option<(shrimply_skia_adw_core::Edge, f64)>,
    pub playhead: Time,
    pub virtual_playhead: Option<Time>,
    pub selected_keys: &'a [Time],
    pub focused_key: Option<Time>,
    pub accent_color: Color,
}

pub fn draw_keyframes(draw: KeyframeGraphDraw<'_>) {
    let KeyframeGraphDraw {
        painter,
        width,
        height,
        content_height,
        graph,
        domain,
        frame_step,
        scrollbar,
        overscroll,
        playhead,
        virtual_playhead,
        selected_keys,
        focused_key,
        accent_color,
    } = draw;
    let keyframe_playhead = virtual_playhead.unwrap_or(playhead);

    painter.rect_filled(
        rect(0.0, 0.0, width, height),
        0,
        shrimply_cross_ui_theme::current().view_bg,
    );

    {
        let graph_painter = painter.with_clip_rect(rect(
            0.0,
            CURSOR_LANE_HEIGHT,
            width,
            (content_height - CURSOR_LANE_HEIGHT).max(0.0),
        ));
        if !matches!(graph, KeyframeGraph::Step { .. }) {
            draw_grid(&graph_painter, width, content_height, domain, frame_step);
        }
        match graph {
            KeyframeGraph::Step { points } => draw_step_values(
                GraphFrame {
                    painter: &graph_painter,
                    width,
                    height: content_height,
                    domain,
                    playhead: keyframe_playhead,
                    frame_step,
                    selected_keys,
                    focused_key,
                    accent_color,
                },
                points,
            ),
            KeyframeGraph::RawValue {
                points,
                segments,
                static_value,
            } => {
                if points.is_empty() {
                    draw_static_value(
                        &graph_painter,
                        width,
                        content_height,
                        *static_value,
                        accent_color,
                    );
                } else {
                    draw_raw_values(
                        GraphFrame {
                            painter: &graph_painter,
                            width,
                            height: content_height,
                            domain,
                            playhead: keyframe_playhead,
                            frame_step,
                            selected_keys,
                            focused_key,
                            accent_color,
                        },
                        points,
                        segments,
                    );
                }
            }
            KeyframeGraph::Speed {
                segments,
                keys,
                static_value,
            } => {
                if segments.is_empty() {
                    draw_static_speed(
                        &graph_painter,
                        width,
                        content_height,
                        *static_value,
                        accent_color,
                    );
                    draw_speed_keys(
                        GraphFrame {
                            painter: &graph_painter,
                            width,
                            height: content_height,
                            domain,
                            playhead: keyframe_playhead,
                            frame_step,
                            selected_keys,
                            focused_key,
                            accent_color,
                        },
                        keys,
                        *static_value,
                    );
                } else {
                    draw_speed_segments(
                        GraphFrame {
                            painter: &graph_painter,
                            width,
                            height: content_height,
                            domain,
                            playhead: keyframe_playhead,
                            frame_step,
                            selected_keys,
                            focused_key,
                            accent_color,
                        },
                        segments,
                        keys,
                    );
                }
            }
        }
    }
    painter.rect_stroke(
        rect(0.5, 0.5, width - 1.0, content_height - 1.0),
        0,
        Stroke::new(1.0, shrimply_cross_ui_theme::current().sidebar_border),
        StrokeKind::Inside,
    );
    draw_cursor_lane(painter, width, domain, frame_step, accent_color);
    if let KeyframeGraph::Step { points } = graph {
        draw_bool_keys(
            GraphFrame {
                painter,
                width,
                height: content_height,
                domain,
                playhead: keyframe_playhead,
                frame_step,
                selected_keys,
                focused_key,
                accent_color,
            },
            points,
        );
    }
    if let Some(virtual_playhead) = virtual_playhead {
        draw_virtual_playhead(
            painter,
            width,
            content_height,
            domain,
            virtual_playhead,
            accent_color,
        );
    }
    draw_playhead(
        painter,
        width,
        content_height,
        domain,
        frame_step,
        playhead,
        accent_color,
    );
    if let Some((edge, distance)) = overscroll {
        draw_graph_overscroll(painter, width, content_height, edge, distance);
    }
    if let Some(scrollbar) = scrollbar {
        shrimply_skia_adw_core::slider::draw(painter.canvas(), scrollbar);
    }
}

fn draw_virtual_playhead(
    painter: &TimelinePainter,
    width: f64,
    height: f64,
    domain: GraphDomain,
    playhead: Time,
    color: Color,
) {
    let x = time_x(playhead, width, domain);
    if x < GRAPH_PAD || x > width - GRAPH_PAD {
        return;
    }
    let stroke = Stroke::new(1.0, color.alpha_multiply(0.75));
    let mut y = CURSOR_LANE_HEIGHT;
    while y < height {
        let end = (y + VIRTUAL_PLAYHEAD_DASH).min(height);
        painter.line_segment(
            [vec2(x as f32, y as f32), vec2(x as f32, end as f32)],
            stroke,
        );
        y += VIRTUAL_PLAYHEAD_DASH + VIRTUAL_PLAYHEAD_GAP;
    }
}

fn draw_step_values(_frame: GraphFrame<'_>, _points: &[KeyframePoint]) {}

fn draw_bool_keys(frame: GraphFrame<'_>, points: &[KeyframePoint]) {
    let graph = shrimply_discrete_keyframe_graph_core::Graph::new(
        points.iter().map(|point| point.time).collect(),
    );
    shrimply_discrete_keyframe_graph_core::draw(shrimply_discrete_keyframe_graph_core::Draw {
        painter: frame.painter,
        width: frame.width,
        content_height: frame.height,
        ruler_height: CURSOR_LANE_HEIGHT,
        graph: &graph,
        domain: frame.domain,
        frame_step: frame.frame_step,
        playhead: frame.playhead,
        selected_keys: frame.selected_keys,
        focused_key: frame.focused_key,
        accent_color: frame.accent_color,
        border_color: shrimply_cross_ui_theme::current().sidebar_border,
        foreground_color: shrimply_cross_ui_theme::current().view_fg,
        shade_color: shrimply_cross_ui_theme::current().sidebar_shade,
    });
}
fn draw_graph_overscroll(
    painter: &TimelinePainter,
    width: f64,
    content_height: f64,
    edge: shrimply_skia_adw_core::Edge,
    distance: f64,
) {
    if width <= 0.0 || content_height <= CURSOR_LANE_HEIGHT {
        return;
    }

    shrimply_skia_adw_core::draw_overshoot(
        painter.canvas(),
        shrimply_skia_adw_core::Rect::from_xywh(
            0.0,
            CURSOR_LANE_HEIGHT as f32,
            width as f32,
            (content_height - CURSOR_LANE_HEIGHT) as f32,
        ),
        edge,
        distance,
        shrimply_cross_ui_theme::current().view_fg,
    );
}

fn draw_cursor_lane(
    painter: &TimelinePainter,
    width: f64,
    domain: GraphDomain,
    frame_step: Time,
    accent_color: Color,
) {
    painter.rect_filled(
        rect(0.0, 0.0, width, CURSOR_LANE_HEIGHT),
        0,
        shrimply_cross_ui_theme::current().sidebar_bg,
    );
    let tick_color = shrimply_cross_ui_theme::current()
        .view_fg
        .alpha_multiply(RULER_TICK_ALPHA);
    for tick in graph_time_ticks(width, domain, frame_step) {
        let x = time_x(tick.time, width, domain);
        if x < GRAPH_PAD || x > width - GRAPH_PAD {
            continue;
        }
        let tick_height = if tick.frame { 5.0 } else { 7.0 };
        painter.rect_filled(
            rect(x, CURSOR_LANE_HEIGHT - tick_height, 1.0, tick_height),
            0,
            tick_color,
        );
    }
    painter.rect_filled(
        rect(0.0, CURSOR_LANE_HEIGHT - 1.0, width, 1.0),
        0,
        accent_color.alpha_multiply(0.55),
    );
}

fn draw_grid(
    painter: &TimelinePainter,
    width: f64,
    height: f64,
    domain: GraphDomain,
    frame_step: Time,
) {
    let stroke = Stroke::new(1.0, shrimply_cross_ui_theme::current().sidebar_shade);
    let graph_top = CURSOR_LANE_HEIGHT;
    let graph_bottom = (height - GRAPH_PAD).max(graph_top);
    for step in 1..4 {
        let y = graph_top + (graph_bottom - graph_top) * step as f64 / 4.0;
        painter.line_segment(
            [
                vec2(GRAPH_PAD as f32, y as f32),
                vec2((width - GRAPH_PAD) as f32, y as f32),
            ],
            stroke,
        );
    }
    for tick in graph_time_ticks(width, domain, frame_step) {
        let x = time_x(tick.time, width, domain);
        if x < GRAPH_PAD || x > width - GRAPH_PAD {
            continue;
        }
        painter.line_segment(
            [
                vec2(x as f32, graph_top as f32),
                vec2(x as f32, graph_bottom as f32),
            ],
            stroke,
        );
    }
}

#[derive(Clone, Copy)]
struct GraphTimeTick {
    time: Time,
    frame: bool,
}

fn graph_time_ticks(width: f64, domain: GraphDomain, frame_step: Time) -> Vec<GraphTimeTick> {
    let start_time = domain.0.max(Time::ZERO);
    let end_time = domain.1.max(start_time);
    let start = start_time.as_secs_f64();
    let end = end_time.as_secs_f64();
    let seconds_per_pixel = graph_seconds_per_pixel(width, domain);
    let frame_step_seconds = frame_step.as_secs_f64();
    if frame_step_seconds.is_finite() && frame_step_seconds > 0.0 {
        let frame_width = frame_step_seconds / seconds_per_pixel;
        if frame_width >= FRAME_TICK_MIN_WIDTH {
            let start_frame = fraction_floor_i64(start_time.seconds / frame_step.seconds)
                .expect("visible frame index must fit i64")
                .max(0) as u64;
            let end_frame = (-fraction_floor_i64(-(end_time.seconds / frame_step.seconds))
                .expect("visible frame index must fit i64"))
            .max(0) as u64;
            return (start_frame..=end_frame)
                .map(|frame| GraphTimeTick {
                    time: Time {
                        seconds: frame_step.seconds * Fraction::from(frame),
                    },
                    frame: true,
                })
                .collect();
        }
    }

    let step = nice_tick_step(seconds_per_pixel * GRAPH_GRID_TARGET_PX);
    let mut tick = (start / step).floor() * step;
    let mut ticks = Vec::new();
    while tick <= end {
        if tick >= 0.0 {
            ticks.push(GraphTimeTick {
                time: Time::from_seconds_f64(tick),
                frame: false,
            });
        }
        tick += step;
    }
    ticks
}

fn graph_plot_width(width: f64) -> f64 {
    (width - GRAPH_PAD * 2.0).max(1.0)
}

fn graph_seconds_per_pixel(width: f64, domain: GraphDomain) -> f64 {
    let visible_seconds = domain
        .1
        .saturating_sub(domain.0)
        .as_secs_f64()
        .max(f64::EPSILON);
    visible_seconds / graph_plot_width(width)
}

fn nice_tick_step(raw: f64) -> f64 {
    let raw = raw.max(0.001);
    let magnitude = 10_f64.powf(raw.log10().floor());
    for multiplier in [1.0, 2.0, 5.0, 10.0] {
        let step = magnitude * multiplier;
        if step >= raw {
            return step;
        }
    }
    magnitude * 10.0
}

fn draw_raw_values(frame: GraphFrame<'_>, points: &[KeyframePoint], segments: &[RawSegment]) {
    let painter = frame.painter;
    let width = frame.width;
    let height = frame.height;
    let domain = frame.domain;
    let range = raw_range(points, segments);
    let mut path = PathBuilder::new();
    if let Some(first) = points.first() {
        let (_, first_y) = raw_point(*first, width, height, domain, range);
        path.move_to((GRAPH_PAD as f32, first_y as f32));
        if let Some(first_point) = points.first() {
            let (first_x, _) = raw_point(*first_point, width, height, domain, range);
            path.line_to((first_x as f32, first_y as f32));
        }
        for segment in segments {
            for progress in curve_sample_progresses(segment.interpolation)
                .into_iter()
                .skip(1)
            {
                let time = Time::from_seconds_f64(
                    segment.start.as_secs_f64()
                        + segment.end.signed_sub(segment.start).as_secs_f64() * progress,
                );
                let eased = segment.interpolation.value(progress);
                let value = segment.start_value + (segment.end_value - segment.start_value) * eased;
                path.line_to((
                    time_x(time, width, domain) as f32,
                    value_y(value, height, range) as f32,
                ));
            }
        }
    }
    if let Some(last) = points.last() {
        let (last_x, last_y) = raw_point(*last, width, height, domain, range);
        path.move_to((last_x as f32, last_y as f32));
        path.line_to(((width - GRAPH_PAD) as f32, last_y as f32));
    }
    painter.path_stroke(&path.snapshot(), Stroke::new(2.0, frame.accent_color));

    for point in points {
        let (px, py) = raw_point(*point, width, height, domain, range);
        draw_keyframe_diamond(
            painter,
            px,
            py,
            key_selected(frame, point.time),
            frame.accent_color,
        );
    }
}

fn draw_speed_segments(frame: GraphFrame<'_>, segments: &[SpeedSegment], keys: &[Time]) {
    let range = speed_range(segments);
    draw_speed_baseline(frame.painter, frame.width, frame.height, range);
    draw_speed_holds(frame, keys, range);
    for segment in segments {
        draw_speed_curve(frame, segment, range);
        let start_speed = segment_speed_at(segment, 0.0).unwrap_or(0.0);
        let end_speed = segment_speed_at(segment, 1.0).unwrap_or(0.0);
        let x0 = time_x(segment.start, frame.width, frame.domain);
        let x1 = time_x(segment.end, frame.width, frame.domain);
        let y0 = value_y(start_speed, frame.height, range);
        let y1 = value_y(end_speed, frame.height, range);
        draw_keyframe_diamond(
            frame.painter,
            x0,
            y0,
            key_selected(frame, segment.start),
            frame.accent_color,
        );
        draw_keyframe_diamond(
            frame.painter,
            x1,
            y1,
            key_selected(frame, segment.end),
            frame.accent_color,
        );
    }
}

fn draw_speed_curve(frame: GraphFrame<'_>, segment: &SpeedSegment, range: (f64, f64)) {
    let painter = frame.painter;
    let width = frame.width;
    let height = frame.height;
    let domain = frame.domain;
    let baseline = value_y(0.0, height, range) as f32;
    let mut points = Vec::new();
    for progress in curve_sample_progresses(segment.interpolation) {
        let time = Time::from_seconds_f64(
            segment.start.as_secs_f64()
                + (segment.end.as_secs_f64() - segment.start.as_secs_f64()) * progress,
        );
        if let Some(speed) = segment_speed_at(segment, progress) {
            points.push(Vec2::new(
                time_x(time, width, domain) as f32,
                value_y(speed, height, range) as f32,
            ));
        } else {
            draw_speed_curve_section(painter, &points, baseline, frame.accent_color);
            points.clear();
        }
    }
    draw_speed_curve_section(painter, &points, baseline, frame.accent_color);
}

fn draw_speed_curve_section(
    painter: &TimelinePainter,
    points: &[Vec2],
    baseline: f32,
    color: Color,
) {
    let Some(first) = points.first() else {
        return;
    };
    let Some(last) = points.last() else {
        return;
    };
    let mut fill = PathBuilder::new();
    fill.move_to((first.x, baseline));
    for point in points {
        fill.line_to((point.x, point.y));
    }
    fill.line_to((last.x, baseline));
    fill.close();
    painter.path_filled(&fill.snapshot(), color.alpha_multiply(SPEED_FILL_ALPHA));

    let mut line = PathBuilder::new();
    line.move_to((first.x, first.y));
    for point in &points[1..] {
        line.line_to((point.x, point.y));
    }
    painter.path_stroke(&line.snapshot(), Stroke::new(2.0, color));
}

fn draw_speed_holds(frame: GraphFrame<'_>, keys: &[Time], range: (f64, f64)) {
    let Some(first) = keys.iter().min() else {
        return;
    };
    let Some(last) = keys.iter().max() else {
        return;
    };
    let painter = frame.painter;
    let width = frame.width;
    let height = frame.height;
    let domain = frame.domain;
    let y = value_y(0.0, height, range);
    let first_x = time_x(*first, width, domain);
    let last_x = time_x(*last, width, domain);
    let stroke = Stroke::new(2.0, frame.accent_color);
    painter.line_segment(
        [
            vec2(GRAPH_PAD as f32, y as f32),
            vec2(first_x as f32, y as f32),
        ],
        stroke,
    );
    painter.line_segment(
        [
            vec2(last_x as f32, y as f32),
            vec2((width - GRAPH_PAD) as f32, y as f32),
        ],
        stroke,
    );
}

fn draw_speed_baseline(painter: &TimelinePainter, width: f64, height: f64, range: (f64, f64)) {
    let y = value_y(0.0, height, range);
    painter.line_segment(
        [
            vec2(GRAPH_PAD as f32, y as f32),
            vec2((width - GRAPH_PAD) as f32, y as f32),
        ],
        Stroke::new(1.0, shrimply_cross_ui_theme::current().sidebar_border),
    );
}

fn draw_static_value(
    painter: &TimelinePainter,
    width: f64,
    height: f64,
    value: f64,
    accent_color: Color,
) {
    let y = value_y(value, height, (value - 1.0, value + 1.0));
    draw_static_line(painter, width, y, accent_color);
}

fn draw_static_speed(
    painter: &TimelinePainter,
    width: f64,
    height: f64,
    value: f64,
    accent_color: Color,
) {
    let y = value_y(value, height, (0.0, value.max(1.0)));
    draw_static_line(painter, width, y, accent_color);
}

fn draw_static_line(painter: &TimelinePainter, width: f64, y: f64, accent_color: Color) {
    painter.line_segment(
        [
            vec2(GRAPH_PAD as f32, y as f32),
            vec2((width - GRAPH_PAD) as f32, y as f32),
        ],
        Stroke::new(2.0, accent_color),
    );
}

fn draw_speed_keys(frame: GraphFrame<'_>, keys: &[Time], value: f64) {
    let painter = frame.painter;
    let width = frame.width;
    let height = frame.height;
    let domain = frame.domain;
    let y = value_y(value, height, (0.0, value.max(1.0)));
    for time in keys {
        draw_keyframe_diamond(
            painter,
            time_x(*time, width, domain),
            y,
            key_selected(frame, *time),
            frame.accent_color,
        );
    }
}

fn key_selected(frame: GraphFrame<'_>, time: Time) -> bool {
    same_frame(time, frame.playhead, frame.frame_step)
        || frame
            .focused_key
            .is_some_and(|focused| focused.approx_eq(time))
        || frame
            .selected_keys
            .iter()
            .any(|selected| selected.approx_eq(time))
}

fn draw_keyframe_diamond(
    painter: &TimelinePainter,
    x: f64,
    y: f64,
    selected: bool,
    accent_color: Color,
) {
    let size = if selected { 5.5 } else { 4.2 };
    let color = if selected {
        Color::YELLOW2
    } else {
        accent_color
    };
    painter.convex_polygon(
        &[
            vec2(x as f32, (y - size) as f32),
            vec2((x + size) as f32, y as f32),
            vec2(x as f32, (y + size) as f32),
            vec2((x - size) as f32, y as f32),
        ],
        color,
        Stroke::none(),
    );
}

fn draw_playhead(
    painter: &TimelinePainter,
    width: f64,
    height: f64,
    domain: GraphDomain,
    frame_step: Time,
    playhead: Time,
    color: Color,
) {
    if domain.1 <= domain.0 {
        return;
    }
    let x = time_x(playhead, width, domain);
    if x < GRAPH_PAD || x > width - GRAPH_PAD {
        return;
    }
    let frame_width = (frame_step.as_secs_f64() / graph_seconds_per_pixel(width, domain)).max(1.0);
    cursor::draw_playhead(
        painter,
        x,
        frame_width,
        height,
        color,
        cursor::PlayheadStyle {
            ruler_height: CURSOR_LANE_HEIGHT,
            frame_y: None,
            handle_width: PLAYHEAD_HANDLE_WIDTH,
            handle_height: PLAYHEAD_HANDLE_HEIGHT,
            handle_top: PLAYHEAD_HANDLE_TOP,
            triangle_height: PLAYHEAD_HANDLE_TRIANGLE_HEIGHT,
        },
    );
}

fn rect(x: f64, y: f64, width: f64, height: f64) -> Rect {
    Rect::from_min_size(
        vec2(x as f32, y as f32),
        vec2(width.max(0.0) as f32, height.max(0.0) as f32),
    )
}

pub fn raw_point(
    point: KeyframePoint,
    width: f64,
    height: f64,
    domain: GraphDomain,
    range: (f64, f64),
) -> (f64, f64) {
    (
        time_x(point.time, width, domain),
        value_y(point.value, height, range),
    )
}

pub fn time_x(time: Time, width: f64, domain: GraphDomain) -> f64 {
    let duration = domain
        .1
        .saturating_sub(domain.0)
        .as_secs_f64()
        .max(f64::EPSILON);
    GRAPH_PAD + (time.as_secs_f64() - domain.0.as_secs_f64()) / duration * (width - GRAPH_PAD * 2.0)
}

pub fn value_y(value: f64, height: f64, (min_value, max_value): (f64, f64)) -> f64 {
    let span = (max_value - min_value).max(1.0);
    height - GRAPH_PAD - (value - min_value) / span * (height - GRAPH_PAD * 2.0)
}

pub fn raw_range(points: &[KeyframePoint], segments: &[RawSegment]) -> (f64, f64) {
    let samples = segments.iter().flat_map(|segment| {
        curve_sample_progresses(segment.interpolation)
            .into_iter()
            .map(move |progress| {
                segment.start_value
                    + (segment.end_value - segment.start_value)
                        * segment.interpolation.value(progress)
            })
    });
    let min_value = points
        .iter()
        .map(|point| point.value)
        .chain(samples.clone())
        .fold(f64::INFINITY, f64::min);
    let max_value = points
        .iter()
        .map(|point| point.value)
        .chain(samples)
        .fold(f64::NEG_INFINITY, f64::max);
    if !min_value.is_finite() || !max_value.is_finite() {
        return (-1.0, 1.0);
    }
    if (max_value - min_value).abs() <= f64::EPSILON {
        let padding = min_value.abs().max(1.0) * 0.5;
        return (min_value - padding, max_value + padding);
    }
    let padding = (max_value - min_value) * 0.08;
    (min_value - padding, max_value + padding)
}

pub fn speed_range(segments: &[SpeedSegment]) -> (f64, f64) {
    let mut minimum = 0.0_f64;
    let mut maximum = 0.0_f64;
    for speed in segments.iter().flat_map(|segment| {
        curve_sample_progresses(segment.interpolation)
            .into_iter()
            .filter_map(move |progress| segment_speed_at(segment, progress))
    }) {
        minimum = minimum.min(speed);
        maximum = maximum.max(speed);
    }
    if (maximum - minimum).abs() <= f64::EPSILON {
        return (-1.0, 1.0);
    }
    let padding = (maximum - minimum) * 0.08;
    (minimum - padding, maximum + padding)
}

pub fn curve_sample_progresses(interpolation: Interpolation) -> Vec<f64> {
    let mut samples: Vec<_> = (0..=SPEED_CURVE_STEPS)
        .map(|step| step as f64 / SPEED_CURVE_STEPS as f64)
        .collect();
    for breakpoint in interpolation.derivative_breakpoints() {
        samples.extend([
            (breakpoint - CURVE_BREAK_OFFSET).max(0.0),
            *breakpoint,
            (breakpoint + CURVE_BREAK_OFFSET).min(1.0),
        ]);
    }
    samples.sort_by(f64::total_cmp);
    samples.dedup();
    samples
}

pub fn segment_speed_at(segment: &SpeedSegment, progress: f64) -> Option<f64> {
    if segment.interpolation == Interpolation::Jump {
        return Some(0.0);
    }
    segment
        .interpolation
        .derivative(progress)
        .map(|derivative| segment.value * derivative)
}

fn same_frame(left: Time, right: Time, frame_step: Time) -> bool {
    left.snapped(frame_step) == right.snapped(frame_step)
}
