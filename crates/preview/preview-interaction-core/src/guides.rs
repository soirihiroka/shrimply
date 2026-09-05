use glam::{IVec2, Vec2 as GlamVec2};
use shrimply_math_color::Color;
use shrimply_preview_core::PreviewViewport;
use shrimply_project::project::CanvasSize;
use shrimply_project::project::{PreviewGuides, Project};
use shrimply_skia_adw_core::canvas::{Align2, FontId, Rect, Stroke, TimelinePainter, Vec2, vec2};

const RULER_SIZE_PX: f32 = 24.0;
pub const MIN_PADDING_PX: u32 = RULER_SIZE_PX as u32;
const GUIDE_HIT_RADIUS_PX: f32 = 5.0;
const RULER_DIVISIONS: usize = 20;
const MAJOR_TICK_INTERVAL: usize = 5;

pub fn padding_px(preview_padding_px: u32, visible: bool, fullscreen: bool) -> u32 {
    let preview_padding_px = if fullscreen { 0 } else { preview_padding_px };
    if visible {
        preview_padding_px.max(MIN_PADDING_PX)
    } else {
        preview_padding_px
    }
}

pub fn viewport(
    surface: IVec2,
    canvas: CanvasSize,
    preview_padding_px: u32,
    visible: bool,
    fullscreen: bool,
) -> PreviewViewport {
    crate::geometry::preview_viewport(
        surface,
        canvas,
        padding_px(preview_padding_px, visible, fullscreen),
    )
}

#[derive(Clone, Copy)]
pub(super) enum GuideDrag {
    Vertical { index: usize, original: Option<f32> },
    Horizontal { index: usize, original: Option<f32> },
}

#[derive(Clone, Copy, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum GuideCursor {
    #[default]
    Default,
    ResizeHorizontal,
    ResizeVertical,
}

impl GuideCursor {
    pub const fn name(self) -> Option<&'static str> {
        match self {
            Self::Default => None,
            Self::ResizeHorizontal => Some("ew-resize"),
            Self::ResizeVertical => Some("ns-resize"),
        }
    }
}

#[derive(Default)]
pub struct GuideInput {
    drag: Option<GuideDrag>,
    origin: Option<GlamVec2>,
    moved: bool,
    cursor: GuideCursor,
}

impl GuideInput {
    pub fn pointer_move(
        &mut self,
        guides: &mut PreviewGuides,
        viewport: PreviewViewport,
        visible: bool,
        position: GlamVec2,
    ) {
        if let Some(drag) = self.drag {
            self.moved |= self.origin.is_some_and(|origin| origin != position);
            drag.update(guides, viewport, position);
            return;
        }
        self.cursor = hover_cursor(guides, viewport, visible, position);
    }

    pub fn pointer_press(
        &mut self,
        guides: &mut PreviewGuides,
        viewport: PreviewViewport,
        visible: bool,
        position: GlamVec2,
    ) -> bool {
        if !visible || self.drag.is_some() {
            return false;
        }
        let Some(drag) = GuideDrag::begin(guides, viewport, position) else {
            return false;
        };
        self.drag = Some(drag);
        self.origin = Some(position);
        self.moved = false;
        self.cursor = drag.cursor();
        true
    }

    pub fn pointer_release(
        &mut self,
        guides: &mut PreviewGuides,
        viewport: PreviewViewport,
        position: GlamVec2,
    ) -> Option<bool> {
        let drag = self.drag.take()?;
        let changed = drag.finish(guides, viewport, position, self.moved);
        self.reset();
        Some(changed)
    }

    pub fn pointer_cancel(&mut self, guides: &mut PreviewGuides) -> bool {
        let Some(drag) = self.drag.take() else {
            return false;
        };
        drag.cancel(guides);
        self.reset();
        true
    }

    pub fn pointer_leave(&mut self) {
        if self.drag.is_none() {
            self.cursor = GuideCursor::Default;
        }
    }

    pub const fn active(&self) -> bool {
        self.drag.is_some()
    }

    pub const fn cursor(&self) -> GuideCursor {
        self.cursor
    }

    fn reset(&mut self) {
        self.origin = None;
        self.moved = false;
        self.cursor = GuideCursor::Default;
    }
}

impl GuideDrag {
    pub(super) fn begin(
        guides: &mut PreviewGuides,
        viewport: PreviewViewport,
        position: GlamVec2,
    ) -> Option<Self> {
        let drag = match ruler_axis(position) {
            Some(GuideAxis::Vertical) => {
                guides.vertical.push(0.0);
                Self::Vertical {
                    index: guides.vertical.len() - 1,
                    original: None,
                }
            }
            Some(GuideAxis::Horizontal) => {
                guides.horizontal.push(0.0);
                Self::Horizontal {
                    index: guides.horizontal.len() - 1,
                    original: None,
                }
            }
            None => return Self::existing(guides, viewport, position),
        };
        drag.update(guides, viewport, position);
        Some(drag)
    }

    pub(super) fn update(
        self,
        guides: &mut PreviewGuides,
        viewport: PreviewViewport,
        position: GlamVec2,
    ) {
        let canvas = viewport.screen_point_to_canvas(position);
        match self {
            Self::Vertical { index, .. } => {
                guides.vertical[index] = canvas.x.clamp(0.0, viewport.canvas_size.x);
            }
            Self::Horizontal { index, .. } => {
                guides.horizontal[index] = canvas.y.clamp(0.0, viewport.canvas_size.y);
            }
        }
    }

    pub(super) fn cancel(self, guides: &mut PreviewGuides) {
        match self {
            Self::Vertical { index, original } => match original {
                Some(value) => guides.vertical[index] = value,
                None => {
                    guides.vertical.remove(index);
                }
            },
            Self::Horizontal { index, original } => match original {
                Some(value) => guides.horizontal[index] = value,
                None => {
                    guides.horizontal.remove(index);
                }
            },
        }
    }

    fn remove(self, guides: &mut PreviewGuides) {
        match self {
            Self::Vertical { index, .. } => {
                guides.vertical.remove(index);
            }
            Self::Horizontal { index, .. } => {
                guides.horizontal.remove(index);
            }
        }
    }

    pub(super) const fn cursor(self) -> GuideCursor {
        match self {
            Self::Vertical { .. } => GuideCursor::ResizeHorizontal,
            Self::Horizontal { .. } => GuideCursor::ResizeVertical,
        }
    }

    pub(super) fn finish(
        self,
        guides: &mut PreviewGuides,
        viewport: PreviewViewport,
        position: GlamVec2,
        moved: bool,
    ) -> bool {
        if self.returned_to_edge(position) {
            if self.is_new() {
                self.cancel(guides);
                false
            } else {
                self.remove(guides);
                true
            }
        } else if moved {
            self.update(guides, viewport, position);
            true
        } else {
            self.cancel(guides);
            false
        }
    }

    const fn is_new(self) -> bool {
        match self {
            Self::Vertical { original, .. } | Self::Horizontal { original, .. } => {
                original.is_none()
            }
        }
    }

    const fn returned_to_edge(self, position: GlamVec2) -> bool {
        match self {
            Self::Vertical { .. } => position.x < RULER_SIZE_PX,
            Self::Horizontal { .. } => position.y < RULER_SIZE_PX,
        }
    }

    pub(super) fn existing(
        guides: &PreviewGuides,
        viewport: PreviewViewport,
        position: GlamVec2,
    ) -> Option<Self> {
        if position.x < RULER_SIZE_PX || position.y < RULER_SIZE_PX {
            return None;
        }
        let mut closest = None;
        for (index, value) in guides.vertical.iter().copied().enumerate() {
            let screen = viewport.canvas_point_to_screen(GlamVec2::new(value, 0.0)).x;
            let distance = (position.x - screen).abs();
            if distance <= GUIDE_HIT_RADIUS_PX && closest.is_none_or(|(best, _)| distance < best) {
                closest = Some((
                    distance,
                    Self::Vertical {
                        index,
                        original: Some(value),
                    },
                ));
            }
        }
        for (index, value) in guides.horizontal.iter().copied().enumerate() {
            let screen = viewport.canvas_point_to_screen(GlamVec2::new(0.0, value)).y;
            let distance = (position.y - screen).abs();
            if distance <= GUIDE_HIT_RADIUS_PX && closest.is_none_or(|(best, _)| distance < best) {
                closest = Some((
                    distance,
                    Self::Horizontal {
                        index,
                        original: Some(value),
                    },
                ));
            }
        }
        closest.map(|(_, drag)| drag)
    }
}

#[derive(Clone, Copy)]
enum GuideAxis {
    Vertical,
    Horizontal,
}

fn ruler_axis(position: GlamVec2) -> Option<GuideAxis> {
    if position.y < RULER_SIZE_PX && position.x >= RULER_SIZE_PX {
        Some(GuideAxis::Horizontal)
    } else if position.x < RULER_SIZE_PX && position.y >= RULER_SIZE_PX {
        Some(GuideAxis::Vertical)
    } else {
        None
    }
}

pub(super) fn ruler_cursor(position: GlamVec2) -> Option<GuideCursor> {
    match ruler_axis(position)? {
        GuideAxis::Vertical => Some(GuideCursor::ResizeHorizontal),
        GuideAxis::Horizontal => Some(GuideCursor::ResizeVertical),
    }
}

fn hover_cursor(
    guides: &PreviewGuides,
    viewport: PreviewViewport,
    visible: bool,
    position: GlamVec2,
) -> GuideCursor {
    if !visible {
        return GuideCursor::Default;
    }
    if let Some(cursor) = ruler_cursor(position) {
        return cursor;
    }
    GuideDrag::existing(guides, viewport, position).map_or(GuideCursor::Default, GuideDrag::cursor)
}

pub fn draw(
    painter: &TimelinePainter,
    guides: &PreviewGuides,
    viewport: PreviewViewport,
    surface_rect: Rect,
    color: Color,
) {
    for guide in &guides.vertical {
        let x = viewport
            .canvas_point_to_screen(GlamVec2::new(*guide, 0.0))
            .x;
        line(
            painter,
            vec2(x, surface_rect.top() + RULER_SIZE_PX),
            vec2(x, surface_rect.bottom()),
            color,
        );
    }
    for guide in &guides.horizontal {
        let y = viewport
            .canvas_point_to_screen(GlamVec2::new(0.0, *guide))
            .y;
        line(
            painter,
            vec2(surface_rect.left() + RULER_SIZE_PX, y),
            vec2(surface_rect.right(), y),
            color,
        );
    }
    draw_rulers(painter, viewport, surface_rect);
}

fn draw_rulers(painter: &TimelinePainter, viewport: PreviewViewport, surface_rect: Rect) {
    let content_rect = viewport.content_rect;
    let background = shrimply_cross_ui_theme::current().sidebar_bg;
    let foreground = shrimply_cross_ui_theme::current().sidebar_fg;
    painter.rect_filled(
        Rect::from_min_size(
            vec2(surface_rect.left(), surface_rect.top()),
            vec2(surface_rect.width(), RULER_SIZE_PX),
        ),
        0,
        background,
    );
    painter.rect_filled(
        Rect::from_min_size(
            vec2(surface_rect.left(), surface_rect.top() + RULER_SIZE_PX),
            vec2(
                RULER_SIZE_PX,
                (surface_rect.height() - RULER_SIZE_PX).max(0.0),
            ),
        ),
        0,
        background,
    );
    let step_x = viewport.canvas_size.x / RULER_DIVISIONS as f32;
    let first_x = viewport
        .screen_point_to_canvas(GlamVec2::new(surface_rect.left(), content_rect.top()))
        .x
        .div_euclid(step_x) as i32;
    let last_x = (viewport
        .screen_point_to_canvas(GlamVec2::new(surface_rect.right(), content_rect.top()))
        .x
        / step_x)
        .ceil() as i32;
    for index in first_x..=last_x {
        let major = index.rem_euclid(MAJOR_TICK_INTERVAL as i32) == 0;
        let tick = if major { 9.0 } else { 5.0 };
        let value = index as f32 * step_x;
        let screen = viewport.canvas_point_to_screen(GlamVec2::new(value, 0.0));
        if screen.x <= surface_rect.left() + RULER_SIZE_PX {
            continue;
        }
        painter.line_segment(
            [
                vec2(screen.x, RULER_SIZE_PX - tick),
                vec2(screen.x, RULER_SIZE_PX),
            ],
            Stroke::new(1.0, foreground),
        );
        if major {
            painter.text(
                vec2(screen.x + 2.0, 2.0),
                Align2::LEFT_TOP,
                format!("{value:.0}"),
                FontId::proportional(9.0),
                foreground,
            );
        }
    }

    let step_y = viewport.canvas_size.y / RULER_DIVISIONS as f32;
    let first_y = viewport
        .screen_point_to_canvas(GlamVec2::new(content_rect.left(), surface_rect.top()))
        .y
        .div_euclid(step_y) as i32;
    let last_y = (viewport
        .screen_point_to_canvas(GlamVec2::new(content_rect.left(), surface_rect.bottom()))
        .y
        / step_y)
        .ceil() as i32;
    for index in first_y..=last_y {
        let major = index.rem_euclid(MAJOR_TICK_INTERVAL as i32) == 0;
        let tick = if major { 9.0 } else { 5.0 };
        let value = index as f32 * step_y;
        let screen = viewport.canvas_point_to_screen(GlamVec2::new(0.0, value));
        if screen.y <= surface_rect.top() + RULER_SIZE_PX {
            continue;
        }
        painter.line_segment(
            [
                vec2(RULER_SIZE_PX - tick, screen.y),
                vec2(RULER_SIZE_PX, screen.y),
            ],
            Stroke::new(1.0, foreground),
        );
        if major {
            painter.text_rotated(
                vec2(RULER_SIZE_PX - 3.0, screen.y + 2.0),
                Align2::LEFT_TOP,
                format!("{value:.0}"),
                FontId::proportional(9.0),
                foreground,
                90.0,
            );
        }
    }
}

pub fn commit_edit(project: &Project) {
    shrimply_project::project::commit_edit(project, "preview-guide");
}

fn line(painter: &TimelinePainter, start: Vec2, end: Vec2, color: Color) {
    painter.line_segment([start, end], Stroke::new(1.5, color));
}
