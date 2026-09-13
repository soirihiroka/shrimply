mod backend;
mod frame_graph;
#[cfg(target_os = "linux")]
mod pointer_lock;

pub mod file_picker;
pub mod project_open;

pub use shrimply_cross_ui_core::desktop_open;

pub mod i18n {
    pub use shrimply_i18n_qt::{init_system_locale, text, text_args};
}

pub fn init() {
    backend::qobject::force_component_opengl();
    backend::qobject::register_drag_input();
    cxx_qt::init_crate!(shrimply_components_qt);
}

#[macro_export]
macro_rules! tr {
    ($key:expr) => {
        $crate::i18n::text($key)
    };
}
