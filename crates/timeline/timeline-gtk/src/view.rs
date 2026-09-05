use super::*;
use crate::items::ItemEdge;

#[derive(Clone, Copy)]
pub(super) struct TimelineViewState {
    pub(super) scroll_seconds: f64,
    pub(super) scroll_y: f64,
    pub(super) seconds_per_pixel: f64,
    pub(super) drag_start_x: f64,
    pub(super) drag_start_y: f64,
    pub(super) drag_start_scroll_seconds: f64,
    pub(super) drag_start_scroll_y: f64,
    pub(super) drag_start_seconds_per_pixel: f64,
    pub(super) drag_moved: bool,
    pub(super) drag_mode: DragMode,
    pub(super) selection: Option<TimelineSelection>,
    pub(super) initialized: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) enum DragMode {
    #[default]
    None,
    Cut,
    Seek,
    Select,
    Item,
    ResizeItem,
    Transition,
    MiddlePan,
    SliderMove,
    VerticalSliderMove,
}

#[derive(Clone)]
pub(super) struct TransitionDrag {
    pub(super) key: crate::project::ItemAddress,
    pub(super) side: TransitionSide,
    pub(super) target_duration: Time,
    pub(super) target_timeline_duration: Time,
    pub(super) remove: bool,
}

#[derive(Clone)]
pub(super) struct ClipTransitionDrag {
    pub(super) outgoing: crate::project::ItemAddress,
    pub(super) incoming: crate::project::ItemAddress,
    pub(super) cut: Time,
    pub(super) target_cut: Time,
    pub(super) original_duration: Option<Time>,
    pub(super) target_duration: Option<Time>,
    pub(super) handle: Option<ItemEdge>,
    pub(super) center_resize: bool,
}

pub(super) struct TimelineScrollEvent {
    pub(super) delta: Vec2,
    pub(super) ctrl: bool,
    pub(super) pointer: Option<Vec2>,
}

#[derive(Clone, Copy)]
pub(super) struct TimelineOverscroll {
    pub(super) edge: TimelineOverscrollEdge,
    pub(super) started_at: Instant,
    pub(super) distance: f64,
}

pub(super) use shrimply_skia_adw_core::Edge as TimelineOverscrollEdge;

#[derive(Clone, Copy)]
pub(super) struct TimelineSelection {
    pub(super) start: Time,
    pub(super) end: Time,
    pub(super) start_y: f64,
    pub(super) end_y: f64,
    pub(super) add_to_selection: bool,
    pub(super) ignore_grouping: bool,
}

impl Default for TimelineViewState {
    fn default() -> Self {
        Self {
            scroll_seconds: 0.0,
            scroll_y: 0.0,
            seconds_per_pixel: 1.0 / 60.0,
            drag_start_x: 0.0,
            drag_start_y: 0.0,
            drag_start_scroll_seconds: 0.0,
            drag_start_scroll_y: 0.0,
            drag_start_seconds_per_pixel: 1.0 / 60.0,
            drag_moved: false,
            drag_mode: DragMode::None,
            selection: None,
            initialized: false,
        }
    }
}

impl TimelineViewState {
    pub(super) fn initialize(
        &mut self,
        duration_seconds: f64,
        timeline_width: f64,
        min_seconds_per_pixel: f64,
    ) {
        if self.initialized
            || timeline_width <= 0.0
            || !duration_seconds.is_finite()
            || duration_seconds <= 0.0
        {
            return;
        }

        self.seconds_per_pixel =
            (duration_seconds / timeline_width).clamp(min_seconds_per_pixel, MAX_SECONDS_PER_PIXEL);
        self.initialized = true;
    }

    pub(super) fn clamp(
        &mut self,
        duration_seconds: f64,
        timeline_width: f64,
        min_seconds_per_pixel: f64,
        track_content_height: f64,
        height: f64,
    ) {
        self.seconds_per_pixel = self
            .seconds_per_pixel
            .clamp(min_seconds_per_pixel, MAX_SECONDS_PER_PIXEL);
        let visible_seconds = timeline_width * self.seconds_per_pixel;
        let max_scroll = duration_seconds.max(visible_seconds) + visible_seconds;
        self.scroll_seconds = self.scroll_seconds.clamp(0.0, max_scroll);
        self.clamp_y(track_content_height, height);
    }

    pub(super) fn clamp_y(&mut self, track_content_height: f64, height: f64) {
        let max_scroll_y = max_scroll_y(track_content_height, height);
        self.scroll_y = self.scroll_y.clamp(0.0, max_scroll_y);
    }

    pub(super) fn keep_time_visible(
        &mut self,
        time: Time,
        duration_seconds: f64,
        timeline_width: f64,
        min_seconds_per_pixel: f64,
        track_content_height: f64,
        height: f64,
    ) {
        if timeline_width <= 0.0 {
            return;
        }

        self.clamp(
            duration_seconds,
            timeline_width,
            min_seconds_per_pixel,
            track_content_height,
            height,
        );
        let visible_seconds = timeline_width * self.seconds_per_pixel;
        if !visible_seconds.is_finite() || visible_seconds <= 0.0 {
            return;
        }

        let margin_pixels = PLAYHEAD_SCROLL_MARGIN_PX
            .max(PLAYHEAD_HANDLE_WIDTH)
            .min(timeline_width / 2.0);
        let margin_seconds = margin_pixels * self.seconds_per_pixel;
        let time_seconds = time.as_secs_f64().max(0.0);
        let left_edge = self.scroll_seconds + margin_seconds;
        let right_edge = self.scroll_seconds + visible_seconds - margin_seconds;

        if time_seconds < left_edge {
            self.scroll_seconds = time_seconds - margin_seconds;
        } else if time_seconds > right_edge {
            self.scroll_seconds = time_seconds - visible_seconds + margin_seconds;
        }

        self.clamp(
            duration_seconds,
            timeline_width,
            min_seconds_per_pixel,
            track_content_height,
            height,
        );
    }

    pub(super) fn center_time(
        &mut self,
        time: Time,
        duration_seconds: f64,
        timeline_width: f64,
        min_seconds_per_pixel: f64,
        track_content_height: f64,
        height: f64,
    ) {
        let visible_seconds = timeline_width * self.seconds_per_pixel;
        if visible_seconds.is_finite() && visible_seconds > 0.0 {
            self.scroll_seconds = time.as_secs_f64() - visible_seconds / 2.0;
        }
        self.clamp(
            duration_seconds,
            timeline_width,
            min_seconds_per_pixel,
            track_content_height,
            height,
        );
    }
}
