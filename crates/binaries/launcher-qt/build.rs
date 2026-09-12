use cxx_qt_build::{CxxQtBuilder, QmlModule};

fn main() {
    CxxQtBuilder::new_qml_module(QmlModule::new("dev.shrimply.launcher").qml_file("qml/Main.qml"))
        .files(["src/backend.rs"])
        .qt_module("Widgets")
        .build();
}
