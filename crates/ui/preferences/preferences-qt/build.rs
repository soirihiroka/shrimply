use cxx_qt_build::{CxxQtBuilder, QmlModule};

fn main() {
    CxxQtBuilder::new_qml_module(
        QmlModule::new("dev.shrimply.preferences").qml_file("qml/PreferencesWindow.qml"),
    )
    .qt_module("Quick")
    .build();
}
