use super::*;
use block2::RcBlock;
use objc2::rc::Weak;
use objc2_app_kit::{NSEvent, NSEventMask, NSEventType};
use objc2_foundation::NSMouseInRect;
use shrimply_preview_interaction_core::fullscreen::{CONTROLS_HIDE_DELAY, ControlsMotion};
use std::time::Instant;

#[derive(Default)]
pub(super) struct State {
    restore: Option<Restore>,
    motion: ControlsMotion,
    hide_at: Option<Instant>,
    pointer_in_controls: bool,
    last_playing: bool,
}

struct Restore {
    tools_hidden: bool,
    controls_hidden: bool,
    toolbar_visible: bool,
}

impl Editor {
    pub(super) fn toggle_preview_fullscreen(&self) {
        let fullscreen = !self.ivars().fullscreen_preview.get();
        self.ivars().fullscreen_preview.set(fullscreen);
        self.sync_panels();
        let window = self.ivars().window.get().expect("editor window must exist");
        if window.styleMask().contains(NSWindowStyleMask::FullScreen) != fullscreen {
            window.toggleFullScreen(None);
        }
    }
    pub(super) fn install_fullscreen_events(&self) {
        let editor = Weak::new(self);
        let handler = RcBlock::new(move |event: std::ptr::NonNull<NSEvent>| {
            // AppKit's local event monitor runs on the main thread and supplies
            // a live event. Returning null consumes Escape; other events pass through.
            let consumed = editor
                .load()
                .is_some_and(|editor| editor.fullscreen_event(unsafe { event.as_ref() }));
            if consumed {
                std::ptr::null_mut()
            } else {
                event.as_ptr()
            }
        });
        let monitor = unsafe {
            NSEvent::addLocalMonitorForEventsMatchingMask_handler(
                NSEventMask::MouseMoved
                    | NSEventMask::LeftMouseDragged
                    | NSEventMask::RightMouseDragged
                    | NSEventMask::OtherMouseDragged
                    | NSEventMask::MouseEntered
                    | NSEventMask::MouseExited
                    | NSEventMask::KeyDown,
                &handler,
            )
        }
        .expect("install fullscreen preview event monitor");
        self.ivars()
            .event_monitor
            .set(monitor)
            .expect("fullscreen event monitor already installed");
        self.ivars()
            .window
            .get()
            .expect("window installed")
            .setAcceptsMouseMovedEvents(true);
    }

    pub(super) fn sync_fullscreen_layout(&self) {
        let active = self.ivars().fullscreen_preview.get();
        let layout = self.ivars().layout.get().expect("layout installed");
        let Some(window) = self.ivars().window.get() else {
            return;
        };
        let was_active = self.ivars().fullscreen.borrow().restore.is_some();
        if active == was_active {
            return;
        }
        if active {
            let restore = Restore {
                tools_hidden: layout.preview_tools.isHidden(),
                controls_hidden: layout.playbar.isHidden(),
                toolbar_visible: window.toolbar().is_some_and(|toolbar| toolbar.isVisible()),
            };
            let playing = player_state::snapshot(
                &self
                    .ivars()
                    .session
                    .get()
                    .expect("project loaded")
                    .player_state,
            )
            .playing;
            *self.ivars().fullscreen.borrow_mut() = State {
                restore: Some(restore),
                last_playing: playing,
                ..Default::default()
            };
            layout.preview_tools.removeFromSuperview();
            layout.playbar.removeFromSuperview();
            layout.playbar.setHidden(false);
            layout
                .playbar
                .setTranslatesAutoresizingMaskIntoConstraints(false);
            layout
                .controls_overlay
                .setContentView(Some(&layout.playbar));
            layout.preview_host.addSubview(&layout.controls_overlay);
            for constraint in &layout.overlay_constraints {
                constraint.setActive(true);
            }
            if let Some(toolbar) = window.toolbar() {
                toolbar.setVisible(false);
            }
            layout.fullscreen_button.setImage(Some(&layout::symbol(
                "arrow.down.right.and.arrow.up.left",
                "Restore Preview",
            )));
            layout
                .fullscreen_button
                .setToolTip(Some(ns_string!("Restore Preview")));
            self.show_fullscreen_controls();
        } else {
            let restore = self
                .ivars()
                .fullscreen
                .borrow_mut()
                .restore
                .take()
                .expect("fullscreen restore state");
            *self.ivars().fullscreen.borrow_mut() = State::default();
            for constraint in &layout.overlay_constraints {
                constraint.setActive(false);
            }
            layout.controls_overlay.setContentView(None);
            layout.controls_overlay.removeFromSuperview();
            layout.preview_layout.addArrangedSubview(&layout.playbar);
            layout.playbar.setHidden(restore.controls_hidden);
            layout
                .viewer
                .insertArrangedSubview_atIndex(&layout.preview_tools, 0);
            layout.preview_tools.setHidden(restore.tools_hidden);
            if let Some(toolbar) = window.toolbar() {
                toolbar.setVisible(restore.toolbar_visible);
            }
            layout.fullscreen_button.setImage(Some(&layout::symbol(
                "arrow.up.left.and.arrow.down.right",
                "Fullscreen Preview",
            )));
            layout
                .fullscreen_button
                .setToolTip(Some(ns_string!("Fullscreen Preview")));
            for canvas in &layout.canvases {
                canvas.set_caption_bottom_inset(0.0);
            }
        }
    }

    fn show_fullscreen_controls(&self) {
        {
            let mut state = self.ivars().fullscreen.borrow_mut();
            state.motion.shown();
            state.hide_at = Some(Instant::now() + CONTROLS_HIDE_DELAY);
        }
        self.ivars()
            .layout
            .get()
            .expect("layout installed")
            .controls_overlay
            .setHidden(false);
        self.update_fullscreen_caption_inset();
    }

    fn hide_fullscreen_controls(&self, require_pointer_move: bool) {
        {
            let mut state = self.ivars().fullscreen.borrow_mut();
            state.motion.hidden(require_pointer_move);
            state.hide_at = None;
            state.pointer_in_controls = false;
        }
        let layout = self.ivars().layout.get().expect("layout installed");
        layout.controls_overlay.setHidden(true);
        for canvas in &layout.canvases {
            canvas.set_caption_bottom_inset(0.0);
        }
    }

    fn update_fullscreen_caption_inset(&self) {
        let layout = self.ivars().layout.get().expect("layout installed");
        let inset = if layout.controls_overlay.isHidden() {
            0.0
        } else {
            layout.controls_overlay.frame().size.height as f32
        };
        for canvas in &layout.canvases {
            canvas.set_caption_bottom_inset(inset);
        }
    }

    pub(super) fn tick_fullscreen(&self, playing: bool) {
        let active = self.ivars().fullscreen_preview.get();
        let hide = {
            let mut state = self.ivars().fullscreen.borrow_mut();
            let started = playing && !state.last_playing;
            state.last_playing = playing;
            active
                && (started
                    || state
                        .hide_at
                        .is_some_and(|deadline| Instant::now() >= deadline))
        };
        if hide {
            self.hide_fullscreen_controls(true);
        }
        if active {
            self.update_fullscreen_caption_inset();
        }
    }

    fn fullscreen_event(&self, event: &NSEvent) -> bool {
        if !self.ivars().fullscreen_preview.get() {
            return false;
        }
        let window = self.ivars().window.get().expect("window installed");
        if event
            .window(self.mtm())
            .is_none_or(|event_window| event_window.windowNumber() != window.windowNumber())
        {
            return false;
        }
        if event.r#type() == NSEventType::KeyDown {
            if event
                .charactersIgnoringModifiers()
                .is_some_and(|key| key.to_string() == "\u{1b}")
            {
                self.toggle_preview_fullscreen();
                return true;
            }
            return false;
        }
        let layout = self.ivars().layout.get().expect("layout installed");
        let point = layout
            .preview_host
            .convertPoint_fromView(event.locationInWindow(), None);
        if !NSMouseInRect(
            point,
            layout.preview_host.bounds(),
            layout.preview_host.isFlipped(),
        ) {
            self.hide_fullscreen_controls(false);
            return false;
        }
        let controls_point = layout
            .playbar
            .convertPoint_fromView(event.locationInWindow(), None);
        let in_controls = !layout.controls_overlay.isHidden()
            && NSMouseInRect(
                controls_point,
                layout.playbar.bounds(),
                layout.playbar.isFlipped(),
            );
        let (reveal, enter) = {
            let mut state = self.ivars().fullscreen.borrow_mut();
            let reveal = state
                .motion
                .pointer_motion(glam::Vec2::new(point.x as f32, point.y as f32));
            let enter = in_controls && !state.pointer_in_controls;
            let leave = !in_controls && state.pointer_in_controls;
            let mut controls_reveal = false;
            if enter {
                state.motion.controls_enter(glam::Vec2::new(
                    controls_point.x as f32,
                    controls_point.y as f32,
                ));
            } else if in_controls {
                controls_reveal = state.motion.controls_motion(glam::Vec2::new(
                    controls_point.x as f32,
                    controls_point.y as f32,
                ));
            }
            state.pointer_in_controls = in_controls;
            if enter || leave {
                state.hide_at = Some(Instant::now() + CONTROLS_HIDE_DELAY);
            }
            (reveal || controls_reveal, enter)
        };
        if reveal {
            self.show_fullscreen_controls();
        } else if enter {
            layout.controls_overlay.setHidden(false);
            self.update_fullscreen_caption_inset();
        }
        false
    }
}
