#[cxx::bridge]
mod locale {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("inspector_locale.h");
        #[rust_name = "format_local_date_time"]
        fn formatLocalDateTime(seconds: i64) -> QString;
    }
}

pub(crate) fn format_date(seconds: i64) -> Option<String> {
    let value = locale::format_local_date_time(seconds).to_string();
    (!value.is_empty()).then_some(value)
}
