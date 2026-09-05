pub mod gl_loader;
mod renderer;

pub use renderer::TimelineRenderer;

use shrimply_skia_adw_core::{audio_meter::AudioMeter, canvas::UVec2};
use std::time::Instant;

#[derive(Default)]
pub struct GlAudioMeter {
    renderer: TimelineRenderer,
    meter: AudioMeter,
}

impl GlAudioMeter {
    pub fn render(&mut self, peaks: [f32; 2], size: UVec2, scale: f32) -> Result<(), String> {
        self.meter.update(peaks, Instant::now());
        let painter =
            self.renderer
                .begin_frame(size, scale, shrimply_cross_ui_theme::current().view_bg)?;
        self.meter.draw(
            painter.canvas(),
            size.x as f32 / scale,
            size.y as f32 / scale,
        );
        self.renderer.end_frame()
    }

    pub fn destroy(&mut self) {
        self.renderer.destroy();
    }
}
