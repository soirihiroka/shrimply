#[cxx_qt::bridge]
mod ffi {
    unsafe extern "C++" {
        include!("file_dialog.h");

        include!("cxx-qt-lib/qcoreapplication.h");
        type QGuiApplication = cxx_qt_lib::QGuiApplication;
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qurl.h");
        type QUrl = cxx_qt_lib::QUrl;

        #[namespace = "shrimply"]
        fn new_widget_application() -> UniquePtr<QGuiApplication>;
        #[namespace = "shrimply"]
        fn apply_windows_system_backdrop();
        #[namespace = "shrimply"]
        fn open_file_dialog(initial_url: &QUrl, title: &QString, filter: &QString) -> QUrl;
        #[namespace = "shrimply"]
        fn save_file_dialog(
            suggested_url: &QUrl,
            title: &QString,
            filter: &QString,
            default_suffix: &QString,
        ) -> QUrl;
    }
}

pub use ffi::{
    apply_windows_system_backdrop, new_widget_application, open_file_dialog, save_file_dialog,
};
