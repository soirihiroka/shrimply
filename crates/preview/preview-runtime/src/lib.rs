use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub use shrimply_audio as audio;
pub use shrimply_math_color::Color;
pub use shrimply_project::{caption, project, time_format};
pub use shrimply_skia_gl::gl_loader;
pub use shrimply_state::player_state;
pub use shrimply_video_cuda as video;

use player_state::SharedPlayerState;
use preferences::store as preferences_store;
use project::{Project, Time};
use shrimply_playback_performance as playback_performance;
use video::compositor::{
    self, CompositeAccuracy, RenderResourceConfig, VideoCommand, VideoCommandSender, VideoEvent,
};

pub use shrimply_preview_interaction_core::captions;
mod cuda_gl;
pub use shrimply_preview_interaction_core::{geometry, guides};
mod media;
pub mod provider;
pub mod renderer;

pub use media::{PreviewMedia, PreviewMediaUpdate, StepDirection};

pub mod preferences {
    pub use shrimply_state::preferences as store;
}

pub mod timeline {
    pub use shrimply_skia_adw_core::canvas as renderer;
}

pub fn background_color(window_color: Color, fullscreen: bool) -> Color {
    if fullscreen {
        Color::BLACK
    } else {
        window_color
    }
}

pub fn playback_time_label(position: Time, duration: Time) -> String {
    format!(
        "{} / {}",
        time_format::playback_time(position),
        time_format::playback_time(duration)
    )
}

pub use shrimply_preview_core::playback::{playback_speed_label, rendered_frame_rate_label};
pub use shrimply_preview_interaction_core::controller;
