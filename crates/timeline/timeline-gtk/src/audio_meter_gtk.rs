use std::{cell::RefCell, rc::Rc};

use gtk::{glib, prelude::*};
use shrimply_skia_adw_core::{audio_meter::DEFAULT_WIDTH, canvas::UVec2};
use shrimply_skia_gl::GlAudioMeter;

pub fn new(peaks: impl Fn() -> [f32; 2] + 'static) -> gtk::GLArea {
    let area = gtk::GLArea::builder()
        .auto_render(false)
        .has_depth_buffer(false)
        .has_stencil_buffer(false)
        .hexpand(true)
        .vexpand(true)
        .width_request(DEFAULT_WIDTH)
        .build();
    let meter = Rc::new(RefCell::new(GlAudioMeter::default()));
    let render_meter = meter.clone();
    area.connect_render(move |area, _| {
        area.make_current();
        assert!(
            area.error().is_none(),
            "audio meter OpenGL context failed: {:?}",
            area.error()
        );
        let scale = area.scale_factor().max(1) as u32;
        let size = UVec2::new(area.width().max(1) as u32, area.height().max(1) as u32) * scale;
        render_meter
            .borrow_mut()
            .render(peaks(), size, scale as f32)
            .expect("render Skia audio meter");
        glib::Propagation::Stop
    });
    area.add_tick_callback(|area, _| {
        area.queue_render();
        glib::ControlFlow::Continue
    });
    area.connect_unrealize(move |area| {
        area.make_current();
        meter.borrow_mut().destroy();
    });
    area
}
