#![cfg(any(target_os = "android", target_os = "linux"))]
#![cfg_attr(not(target_os = "android"), allow(dead_code))]

#[cfg(target_os = "android")]
mod bridge;
mod math;
mod vulkan;

use shrimply_math_core::{Fraction, Time};
use shrimply_timeline_paint_skia::{self as paint, Viewport, ruler::TimelineScale};

struct TimelineState {
    tracks: Vec<paint::Track>,
    duration: Time,
    viewport: Viewport,
    playhead: Time,
}

impl TimelineState {
    fn new(stress: bool) -> Self {
        let tracks = paint::fixture::tracks(stress);
        let duration = tracks
            .iter()
            .flat_map(|track| &track.clips)
            .map(|clip| clip.end)
            .max()
            .unwrap_or(Time::ZERO);
        Self {
            tracks,
            duration,
            viewport: Viewport {
                scale: TimelineScale::new(Time::ZERO, Fraction::new(1_u64, 50_u64)),
                scroll_y: 0.0,
            },
            playhead: Time::from_seconds(5),
        }
    }

    fn draw(&self, canvas: &skia_safe::Canvas, width: u32, height: u32, density: f32) {
        canvas.save();
        canvas.scale((density, density));
        paint::draw(
            &paint::canvas::TimelinePainter::new(canvas),
            paint::Timeline {
                tracks: &self.tracks,
                viewport: self.viewport,
                playhead: self.playhead,
                frame_rate: Fraction::from(30),
                width: f64::from(width) / f64::from(density),
                height: f64::from(height) / f64::from(density),
                theme: &shrimply_cross_ui_theme::current(),
            },
        );
        canvas.restore();
    }
}
