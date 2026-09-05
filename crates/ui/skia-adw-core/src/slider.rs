use std::time::Instant;

use skia_safe::Canvas;

use super::{Axis, Rect, Scrollbar, ScrollbarMetrics, ScrollbarState, Vec2};

const SURFACE_SCROLL_FACTOR: f64 = 2.5;
const WHEEL_SCROLL_EXPONENT: f64 = 2.0 / 3.0;
const WHEEL_SCROLL_MAX_PAGE_FRACTION: f64 = 0.5;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    None,
    Drag { target: Option<f64> },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Begin {
    None,
    Drag,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScrollInput {
    Wheel,
    Surface,
}

#[derive(Clone, Copy)]
pub struct Frame {
    pub scrollbar: Option<Scrollbar>,
    pub animating: bool,
}

#[derive(Clone, Copy)]
pub struct Event {
    pub handled: bool,
    pub animating: bool,
}

#[derive(Clone, Copy, Default)]
pub struct Lifecycle {
    resize: ResizeState,
    scroll: ScrollState,
    drag: Option<DragState>,
}

#[derive(Clone, Copy, Default)]
pub struct ResizeState {
    expansion: f32,
    animation: Option<ResizeAnimation>,
}

#[derive(Clone, Copy, Default)]
pub struct ScrollState {
    animation: Option<ValueAnimation>,
}

#[derive(Clone, Copy)]
struct ResizeAnimation {
    source: f32,
    target: f32,
    started_at: Instant,
}

#[derive(Clone, Copy)]
struct ValueAnimation {
    source: f64,
    target: f64,
    started_at: Instant,
}

#[derive(Clone, Copy)]
struct DragState {
    source: f64,
    max_value: f64,
    max_travel: f64,
}

impl Lifecycle {
    pub fn hit_test(&self, scrollbar: Scrollbar, pointer: Vec2) -> bool {
        hovered(self.interaction_scrollbar(scrollbar, false), Some(pointer))
    }

    pub fn frame(&mut self, scrollbar: Option<Scrollbar>, pointer: Option<Vec2>) -> Frame {
        let (scrollbar, resizing) =
            update_resize(&mut self.resize, scrollbar, pointer, self.drag.is_some());
        Frame {
            scrollbar,
            animating: resizing,
        }
    }

    pub fn begin(
        &mut self,
        scrollbar: Scrollbar,
        pointer: Vec2,
        mut on_value: impl FnMut(f64),
    ) -> Begin {
        let mut scrollbar = self.interaction_scrollbar(scrollbar, false);
        let animated_value = animation_value(&mut self.scroll).map(|(value, _)| value);
        if let Some(value) = animated_value {
            scrollbar.value = value;
        }
        match action(scrollbar, pointer) {
            Action::None => Begin::None,
            Action::Drag { target } => {
                cancel(&mut self.scroll);
                let Some(metrics) = super::scrollbar_metrics(scrollbar) else {
                    return Begin::None;
                };
                let (track_length, thumb_length) = match scrollbar.axis {
                    Axis::Horizontal => (metrics.track.width(), metrics.thumb.width()),
                    Axis::Vertical => (metrics.track.height(), metrics.thumb.height()),
                };
                let max_travel = f64::from(track_length - thumb_length).max(0.0);
                if max_travel <= f64::EPSILON || metrics.max_value <= f64::EPSILON {
                    return Begin::None;
                }
                let source = target
                    .unwrap_or(scrollbar.value)
                    .clamp(0.0, metrics.max_value);
                if target.is_some() || animated_value.is_some() {
                    on_value(source);
                }
                self.drag = Some(DragState {
                    source,
                    max_value: metrics.max_value,
                    max_travel,
                });
                Begin::Drag
            }
        }
    }

    pub fn scroll_pages_at(
        &mut self,
        scrollbar: Scrollbar,
        pointer: Option<Vec2>,
        pages: f64,
        on_value: impl FnMut(f64),
    ) -> Event {
        let scrollbar = self.interaction_scrollbar(scrollbar, false);
        if !hovered(scrollbar, pointer) {
            return Event {
                handled: false,
                animating: false,
            };
        }

        Event {
            handled: true,
            animating: animate_by_pages(&mut self.scroll, scrollbar, pages, on_value),
        }
    }

    pub fn scroll_units_at(
        &mut self,
        scrollbar: Scrollbar,
        pointer: Option<Vec2>,
        units: f64,
        on_value: impl FnMut(f64),
    ) -> Event {
        let scrollbar = self.interaction_scrollbar(scrollbar, false);
        if !hovered(scrollbar, pointer) {
            return Event {
                handled: false,
                animating: false,
            };
        }

        Event {
            handled: true,
            animating: animate_by_units(&mut self.scroll, scrollbar, units, on_value),
        }
    }

    pub fn scroll_at(
        &mut self,
        scrollbar: Scrollbar,
        pointer: Option<Vec2>,
        delta: f64,
        input: ScrollInput,
        mut on_value: impl FnMut(f64),
    ) -> Event {
        let units = match input {
            ScrollInput::Wheel => {
                let page_size = scrollbar.viewport_length;
                delta
                    * page_size
                        .powf(WHEEL_SCROLL_EXPONENT)
                        .min(page_size * WHEEL_SCROLL_MAX_PAGE_FRACTION)
            }
            ScrollInput::Surface => delta * SURFACE_SCROLL_FACTOR,
        };
        let scrollbar = self.interaction_scrollbar(scrollbar, false);
        if hovered(scrollbar, pointer) {
            return Event {
                handled: true,
                animating: animate_by_units(&mut self.scroll, scrollbar, units, on_value),
            };
        }

        cancel(&mut self.scroll);
        let Some(value) = scroll_by_units(scrollbar, units) else {
            return Event {
                handled: false,
                animating: false,
            };
        };
        on_value(value);
        Event {
            handled: true,
            animating: false,
        }
    }

    pub fn drag_by(
        &mut self,
        _scrollbar: Scrollbar,
        delta: f64,
        mut on_value: impl FnMut(f64),
    ) -> bool {
        let Some(drag) = self.drag else {
            return false;
        };
        let target =
            (drag.source + delta * drag.max_value / drag.max_travel).clamp(0.0, drag.max_value);
        on_value(target);
        true
    }

    pub fn apply_scroll(&mut self, on_value: impl FnMut(f64)) -> bool {
        apply_animation(&mut self.scroll, on_value)
    }

    pub fn end_drag(&mut self) {
        self.drag = None;
    }

    pub fn cancel_scroll(&mut self) {
        cancel(&mut self.scroll);
    }

    pub fn animating(&self) -> bool {
        self.drag.is_some() || resize_animating(self.resize) || scroll_animating(self.scroll)
    }

    pub fn visual_animating(&self) -> bool {
        resize_animating(self.resize) || scroll_animating(self.scroll)
    }

    fn interaction_scrollbar(&self, mut scrollbar: Scrollbar, active: bool) -> Scrollbar {
        scrollbar.state = ScrollbarState {
            expansion: self.resize.expansion.max(1.0),
            active,
        };
        scrollbar
    }
}

pub fn idle_state() -> ScrollbarState {
    ScrollbarState {
        expansion: 0.0,
        active: false,
    }
}

pub fn update_resize(
    state: &mut ResizeState,
    scrollbar: Option<Scrollbar>,
    pointer: Option<Vec2>,
    active: bool,
) -> (Option<Scrollbar>, bool) {
    let target = if active || scrollbar.is_some_and(|scrollbar| hovered(scrollbar, pointer)) {
        1.0
    } else {
        0.0
    };
    let animating = update_resize_expansion(state, target);
    (
        scrollbar.map(|mut scrollbar| {
            scrollbar.state = ScrollbarState {
                expansion: state.expansion,
                active,
            };
            scrollbar
        }),
        animating,
    )
}

pub fn resize_animating(state: ResizeState) -> bool {
    state.animation.is_some()
}

pub fn cancel(state: &mut ScrollState) {
    state.animation = None;
}

pub fn animate_to(
    state: &mut ScrollState,
    current_value: f64,
    target: f64,
    mut on_value: impl FnMut(f64),
) -> bool {
    let source = animation_value(state)
        .map(|(value, _)| value)
        .unwrap_or(current_value);
    if (source - target).abs() <= f64::EPSILON {
        state.animation = None;
        on_value(target);
        return false;
    }

    on_value(source);
    state.animation = Some(ValueAnimation {
        source,
        target,
        started_at: Instant::now(),
    });
    true
}

pub fn animate_by_pages(
    state: &mut ScrollState,
    scrollbar: Scrollbar,
    pages: f64,
    on_value: impl FnMut(f64),
) -> bool {
    let mut scrollbar = animation_base_scrollbar(state, scrollbar);
    let Some(target) = scroll_by_pages(scrollbar, pages) else {
        return false;
    };
    let current_value = scrollbar.value;
    scrollbar.value = target;
    animate_to(state, current_value, scrollbar.value, on_value)
}

pub fn animate_by_units(
    state: &mut ScrollState,
    scrollbar: Scrollbar,
    units: f64,
    on_value: impl FnMut(f64),
) -> bool {
    let mut scrollbar = animation_base_scrollbar(state, scrollbar);
    let Some(target) = scroll_by_units(scrollbar, units) else {
        return false;
    };
    let current_value = scrollbar.value;
    scrollbar.value = target;
    animate_to(state, current_value, scrollbar.value, on_value)
}

pub fn apply_animation(state: &mut ScrollState, mut on_value: impl FnMut(f64)) -> bool {
    let Some((value, active)) = animation_value(state) else {
        return false;
    };
    on_value(value);
    active
}

pub fn scroll_animating(state: ScrollState) -> bool {
    state.animation.is_some()
}

pub fn draw(canvas: &Canvas, scrollbar: Scrollbar) -> Option<ScrollbarMetrics> {
    super::draw_scrollbar(canvas, scrollbar)
}

pub fn hovered(scrollbar: Scrollbar, pointer: Option<Vec2>) -> bool {
    pointer.is_some_and(|pointer| hit_rect(scrollbar).contains(pointer))
}

pub fn action(scrollbar: Scrollbar, pointer: Vec2) -> Action {
    if !hit_rect(scrollbar).contains(pointer) {
        return Action::None;
    }

    let Some(metrics) = super::scrollbar_metrics(scrollbar) else {
        return Action::None;
    };
    if metrics.thumb.contains(pointer) {
        return Action::Drag { target: None };
    }

    let target = match scrollbar.axis {
        Axis::Horizontal
            if pointer.x >= metrics.track.left() && pointer.x <= metrics.track.right() =>
        {
            let max_travel = f64::from(metrics.track.width() - metrics.thumb.width()).max(0.0);
            (max_travel > f64::EPSILON).then(|| {
                let track_offset = f64::from(pointer.x - metrics.track.left())
                    - f64::from(metrics.thumb.width()) / 2.0;
                track_offset / max_travel * metrics.max_value
            })
        }
        Axis::Vertical
            if pointer.y >= metrics.track.top() && pointer.y <= metrics.track.bottom() =>
        {
            let max_travel = f64::from(metrics.track.height() - metrics.thumb.height()).max(0.0);
            (max_travel > f64::EPSILON).then(|| {
                let track_offset = f64::from(pointer.y - metrics.track.top())
                    - f64::from(metrics.thumb.height()) / 2.0;
                track_offset / max_travel * metrics.max_value
            })
        }
        _ => None,
    };

    target
        .map(|target| Action::Drag {
            target: Some(target.clamp(0.0, metrics.max_value)),
        })
        .unwrap_or(Action::None)
}

pub fn scroll_by_pages(scrollbar: Scrollbar, pages: f64) -> Option<f64> {
    let metrics = super::scrollbar_metrics(scrollbar)?;
    Some((scrollbar.value + pages * scrollbar.viewport_length).clamp(0.0, metrics.max_value))
}

pub fn scroll_by_units(scrollbar: Scrollbar, units: f64) -> Option<f64> {
    let metrics = super::scrollbar_metrics(scrollbar)?;
    Some((scrollbar.value + units).clamp(0.0, metrics.max_value))
}

pub fn drag_target(scrollbar: Scrollbar, source: f64, delta: f64) -> Option<f64> {
    let metrics = super::scrollbar_metrics(scrollbar)?;
    let (track_length, thumb_length) = match scrollbar.axis {
        Axis::Horizontal => (metrics.track.width(), metrics.thumb.width()),
        Axis::Vertical => (metrics.track.height(), metrics.thumb.height()),
    };
    let max_travel = f64::from(track_length - thumb_length).max(0.0);
    if max_travel <= f64::EPSILON || metrics.max_value <= f64::EPSILON {
        return None;
    }

    Some((source + delta * metrics.max_value / max_travel).clamp(0.0, metrics.max_value))
}

fn update_resize_expansion(state: &mut ResizeState, target: f32) -> bool {
    let target = target.clamp(0.0, 1.0);
    if state
        .animation
        .is_none_or(|animation| (animation.target - target).abs() > f32::EPSILON)
    {
        if (state.expansion - target).abs() <= f32::EPSILON {
            state.expansion = target;
            state.animation = None;
            return false;
        }
        state.animation = Some(ResizeAnimation {
            source: state.expansion,
            target,
            started_at: Instant::now(),
        });
    }

    let Some(current) = state.animation else {
        return false;
    };
    let (value, active) = super::animate_scrollbar_expansion(
        current.source,
        current.target,
        current.started_at.elapsed(),
    );
    state.expansion = value;
    if !active {
        state.animation = None;
    }
    active
}

fn animation_base_scrollbar(state: &ScrollState, mut scrollbar: Scrollbar) -> Scrollbar {
    if let Some(animation) = state.animation {
        scrollbar.value = animation.target;
    }
    scrollbar
}

fn animation_value(state: &mut ScrollState) -> Option<(f64, bool)> {
    let animation = state.animation?;
    let (value, active) = super::animate_scroll(super::ScrollAnimation {
        source: animation.source,
        target: animation.target,
        elapsed: animation.started_at.elapsed(),
    });
    if !active {
        state.animation = None;
    }
    Some((value, active))
}

fn hit_rect(scrollbar: Scrollbar) -> Rect {
    let hit_size = super::SCROLLBAR_HOVER_EDGE_MARGIN + super::SCROLLBAR_THICKNESS;
    match scrollbar.axis {
        Axis::Horizontal => Rect::from_xywh(
            scrollbar.bounds.left(),
            scrollbar.bounds.top() + (scrollbar.bounds.height() - hit_size).max(0.0),
            scrollbar.bounds.width(),
            scrollbar.bounds.height().min(hit_size),
        ),
        Axis::Vertical => Rect::from_xywh(
            scrollbar.bounds.left() + (scrollbar.bounds.width() - hit_size).max(0.0),
            scrollbar.bounds.top(),
            scrollbar.bounds.width().min(hit_size),
            scrollbar.bounds.height(),
        ),
    }
}
