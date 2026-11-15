use std::cell::RefCell;
use std::ffi::c_void;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::gdk;
use gtk::gdk::prelude::*;
use gtk::glib;
use gtk::glib::translate::{ToGlibPtr, from_glib};
use gtk::prelude::*;
use shrimply_pointer_lock_wayland::WaylandPointerLock;

const SLOW_POINTER_LOCK_LOG_THRESHOLD: Duration = Duration::from_millis(16);

unsafe extern "C" {
    fn gdk_wayland_display_get_type() -> glib::ffi::GType;
    fn gdk_wayland_display_get_wl_display(display: *mut gdk::ffi::GdkDisplay) -> *mut c_void;
    fn gdk_wayland_surface_get_wl_surface(surface: *mut gdk::ffi::GdkSurface) -> *mut c_void;
    fn gdk_wayland_seat_get_wl_seat(seat: *mut gdk::ffi::GdkSeat) -> *mut c_void;
}

pub struct PointerLock {
    _inner: Rc<RefCell<PointerLockInner>>,
}

impl PointerLock {
    pub fn new(widget: &impl IsA<gtk::Widget>, on_delta: impl Fn(f64) + 'static) -> Option<Self> {
        Self::new_2d(widget, move |delta_x, _| on_delta(delta_x))
    }

    pub fn new_2d(
        widget: &impl IsA<gtk::Widget>,
        on_delta: impl Fn(f64, f64) + 'static,
    ) -> Option<Self> {
        let native = widget.as_ref().native()?;
        let gdk_surface = native.surface()?;
        let gdk_display = gdk_surface.display();
        if !gdk_display
            .type_()
            .is_a(unsafe { from_glib(gdk_wayland_display_get_type()) })
        {
            return None;
        }
        let gdk_seat = gdk_display.default_seat()?;
        let wl_display =
            unsafe { gdk_wayland_display_get_wl_display(gdk_display.to_glib_none().0) };
        let wl_surface =
            unsafe { gdk_wayland_surface_get_wl_surface(gdk_surface.to_glib_none().0) };
        let wl_seat = unsafe { gdk_wayland_seat_get_wl_seat(gdk_seat.to_glib_none().0) };
        let wayland = unsafe { WaylandPointerLock::new(wl_display, wl_surface, wl_seat) }?;

        let inner = Rc::new(RefCell::new(PointerLockInner {
            wayland,
            on_delta: Rc::new(on_delta),
            source: None,
            _gdk_display: gdk_display,
            _gdk_surface: gdk_surface,
            _gdk_seat: gdk_seat,
        }));
        let weak = Rc::downgrade(&inner);
        let source = widget.as_ref().add_tick_callback(move |_, _| {
            let Some(inner) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let mut inner = inner.borrow_mut();
            if let Some((delta_x, delta_y)) = inner.wayland.poll() {
                let started = Instant::now();
                (inner.on_delta)(delta_x, delta_y);
                let elapsed = started.elapsed();
                if elapsed >= SLOW_POINTER_LOCK_LOG_THRESHOLD {
                    tracing::debug!(
                        delta_x,
                        delta_y,
                        on_delta_elapsed_us = elapsed.as_micros(),
                        "pointer_lock: coalesced delta"
                    );
                }
            }
            glib::ControlFlow::Continue
        });
        inner.borrow_mut().source = Some(source);
        Some(Self { _inner: inner })
    }

    pub fn restore_cursor_at(&self, x: f64, y: f64) {
        self._inner.borrow_mut().wayland.restore_cursor_at(x, y);
    }
}

struct PointerLockInner {
    wayland: WaylandPointerLock,
    on_delta: Rc<dyn Fn(f64, f64)>,
    source: Option<gtk::TickCallbackId>,
    _gdk_display: gdk::Display,
    _gdk_surface: gdk::Surface,
    _gdk_seat: gdk::Seat,
}

impl Drop for PointerLockInner {
    fn drop(&mut self) {
        if let Some(source) = self.source.take() {
            source.remove();
        }
    }
}
