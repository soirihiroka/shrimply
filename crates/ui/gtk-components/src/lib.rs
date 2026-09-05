pub use shrimply_skia_adw_core::{skia_font, skia_system_font};
pub use shrimply_skia_gl::gl_loader;
pub mod canvas {
    pub use shrimply_skia_adw_core::canvas::*;
    pub use shrimply_skia_gl::TimelineRenderer;
}
pub mod cursor;
pub mod desktop_open;
pub mod export_feedback;
pub mod file_picker;
pub mod icons;
pub mod i18n {
    pub use shrimply_i18n_gtk::{init_system_locale, text, text_args};
}
pub mod playback_shortcuts;
pub mod project_open;
pub mod project_settings;
pub mod resource_pipeline;
pub mod toast;
pub mod ui;

#[macro_export]
macro_rules! tr {
    ($key:expr) => {
        $crate::i18n::text($key)
    };
}
