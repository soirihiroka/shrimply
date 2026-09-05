use super::*;
use crate::items::ItemEdge;

#[derive(Clone, Copy)]
pub struct TimelineViewState {
    pub scroll_seconds: f64,
    pub scroll_y: f64,
    pub seconds_per_pixel: f64,
    pub drag_start_x: f64,
    pub drag_start_y: f64,
    pub drag_start_scroll_seconds: f64,
    pub drag_start_scroll_y: f64,
    pub drag_start_seconds_per_pixel: f64,
    pub drag_moved: bool,
    pub drag_mode: DragMode,
    pub selection: Option<TimelineSelection>,
    pub initialized: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum DragMode {
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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum TimelineCursor {
    #[default]
    Default,
    ResizeStart,
    ResizeEnd,
    ResizeHorizontal,
    Crosshair,
}

#[derive(Clone)]
pub struct TransitionDrag {
    pub key: crate::project::ItemAddress,
    pub side: TransitionSide,
    pub target_duration: Time,
    pub target_timeline_duration: Time,
    pub remove: bool,
}

#[derive(Clone)]
pub struct ClipTransitionDrag {
    pub outgoing: crate::project::ItemAddress,
    pub incoming: crate::project::ItemAddress,
    pub cut: Time,
    pub target_cut: Time,
    pub original_duration: Option<Time>,
    pub target_duration: Option<Time>,
    pub handle: Option<ItemEdge>,
    pub center_resize: bool,
}

pub struct TimelineScrollEvent {
    pub delta: Vec2,
    pub ctrl: bool,
    pub pointer: Option<Vec2>,
}

#[derive(Clone, Copy)]
pub struct TimelineOverscroll {
    pub edge: TimelineOverscrollEdge,
    pub started_at: Instant,
    pub distance: f64,
}

pub use shrimply_skia_adw_core::Edge as TimelineOverscrollEdge;

#[derive(Clone, Copy)]
pub struct TimelineSelection {
    pub start: Time,
    pub end: Time,
    pub start_y: f64,
    pub end_y: f64,
    pub add_to_selection: bool,
    pub ignore_grouping: bool,
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
    pub fn restore_zoom(&mut self, zoom: Option<Time>) {
        if let Some(zoom) = zoom.filter(|zoom| *zoom > Time::ZERO) {
            let seconds_per_pixel = zoom.as_secs_f64();
            if seconds_per_pixel.is_finite() {
                self.seconds_per_pixel = seconds_per_pixel;
                self.drag_start_seconds_per_pixel = seconds_per_pixel;
                self.initialized = true;
            }
        }
    }

    pub fn begin_pan(&mut self, point: glam::DVec2) {
        self.drag_start_x = point.x;
        self.drag_start_y = point.y;
        self.drag_start_scroll_seconds = self.scroll_seconds;
        self.drag_start_scroll_y = self.scroll_y;
        self.drag_mode = DragMode::MiddlePan;
        self.drag_moved = false;
    }

    pub fn pan_to(&mut self, point: glam::DVec2) {
        self.scroll_seconds =
            self.drag_start_scroll_seconds - (point.x - self.drag_start_x) * self.seconds_per_pixel;
        self.scroll_y = self.drag_start_scroll_y - (point.y - self.drag_start_y);
    }
    pub fn initialize(
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

    pub fn clamp(
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

    pub fn clamp_y(&mut self, track_content_height: f64, height: f64) {
        let max_scroll_y = max_scroll_y(track_content_height, height);
        self.scroll_y = self.scroll_y.clamp(0.0, max_scroll_y);
    }

    pub fn keep_time_visible(
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

    pub fn center_time(
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
