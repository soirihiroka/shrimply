use glam::Vec2;
use shrimply_preview_provider_skia::{
    Cursor, CursorUpdate, PointerButton, PointerEvent, PreviewResponse, PreviewViewport, Rect, math,
};

#[derive(Clone, Copy)]
pub struct Navigation {
    zoom: f32,
    center: Vec2,
    drag: Option<Drag>,
}

#[derive(Clone, Copy)]
struct Drag {
    origin: Vec2,
    previous: Vec2,
    moved: bool,
}

impl Default for Navigation {
    fn default() -> Self {
        Self {
            zoom: math::MIN_PREVIEW_ZOOM,
            center: Vec2::splat(0.5),
            drag: None,
        }
    }
}

impl Navigation {
    pub fn pointer(
        &mut self,
        fit: PreviewViewport,
        bounds: Rect,
        event: PointerEvent<'_>,
        enabled: bool,
        busy: bool,
    ) -> Option<PreviewResponse> {
        if !enabled {
            self.reset();
            return None;
        }
        match event {
            PointerEvent::Scroll { input, delta } if delta.y != 0.0 => {
                if busy || self.active() {
                    return Some(PreviewResponse {
                        handled: true,
                        ..PreviewResponse::IGNORED
                    });
                }
                self.scroll(fit, bounds, input.sample.position, delta.y);
            }
            PointerEvent::Begin(input) if input.button == PointerButton::Middle => {
                if busy {
                    return Some(PreviewResponse::IGNORED);
                }
                if !self.active() {
                    self.begin(input.sample.position);
                }
            }
            PointerEvent::Hover(input) if self.active() => {
                self.pan(fit, bounds, input.sample.position);
            }
            PointerEvent::Samples { input, samples }
                if self.active() && input.button == PointerButton::Middle =>
            {
                for sample in samples {
                    self.pan(fit, bounds, sample.position);
                }
                self.pan(fit, bounds, input.sample.position);
            }
            PointerEvent::End(input) if self.active() && input.button == PointerButton::Middle => {
                self.end(fit, bounds, input.sample.position)
            }
            PointerEvent::Cancel if self.active() => self.cancel(),
            _ if self.active() => {}
            _ => return None,
        }
        Some(PreviewResponse {
            handled: true,
            redraw: true,
            cursor: if self.active() {
                CursorUpdate::Set(Cursor::Grabbing)
            } else {
                CursorUpdate::Clear
            },
            ..PreviewResponse::IGNORED
        })
    }

    pub fn viewport(&self, fit: PreviewViewport, bounds: Rect) -> PreviewViewport {
        PreviewViewport::new(
            fit.canvas_size,
            math::zoomed_preview_rect(fit.content_rect, bounds, self.zoom, self.center),
        )
    }

    pub fn clip_rect(&self, bounds: Rect, surface: Vec2) -> Rect {
        if self.zoom > math::MIN_PREVIEW_ZOOM {
            bounds
        } else {
            Rect::from_min_size(Vec2::ZERO, surface)
        }
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn active(&self) -> bool {
        self.drag.is_some()
    }

    fn scroll(&mut self, fit: PreviewViewport, bounds: Rect, position: Vec2, steps: f32) {
        if self.active() || !steps.is_finite() || steps == 0.0 {
            return;
        }
        let before = self.viewport(fit, bounds).content_rect;
        let zoom = math::preview_zoom(self.zoom, steps);
        if zoom == self.zoom {
            return;
        }
        let rect = math::cursor_zoom_rect(before, self.zoom, zoom, position);
        self.zoom = zoom;
        if zoom == math::MIN_PREVIEW_ZOOM {
            self.reset();
        } else {
            self.center = math::preview_center(rect, bounds);
            self.center = math::preview_center(self.viewport(fit, bounds).content_rect, bounds);
        }
    }

    fn begin(&mut self, position: Vec2) {
        self.drag = Some(Drag {
            origin: position,
            previous: position,
            moved: false,
        });
    }

    fn pan(&mut self, fit: PreviewViewport, bounds: Rect, position: Vec2) {
        let Some(mut drag) = self.drag else {
            return;
        };
        drag.moved |= math::preview_dragged(drag.origin, position);
        if drag.moved && self.zoom > math::MIN_PREVIEW_ZOOM {
            let rect = self.viewport(fit, bounds).content_rect;
            let delta = position - drag.previous;
            self.center = math::preview_center(rect.translated(delta), bounds);
            self.center = math::preview_center(self.viewport(fit, bounds).content_rect, bounds);
        }
        if drag.moved {
            drag.previous = position;
        }
        self.drag = Some(drag);
    }

    fn end(&mut self, fit: PreviewViewport, bounds: Rect, position: Vec2) {
        self.pan(fit, bounds, position);
        if self.drag.take().is_some_and(|drag| !drag.moved) {
            self.reset();
        }
    }

    pub fn cancel(&mut self) {
        self.drag = None;
    }
}
