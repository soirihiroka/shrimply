use cxx_qt_build::{CxxQtBuilder, QmlModule};

fn main() {
    unsafe {
        CxxQtBuilder::new_qml_module(
            QmlModule::new("dev.shrimply.editor")
                .qml_file("qml/Main.qml")
                .qml_file("qml/AboutWindow.qml")
                .qml_file("qml/PreferencesWindow.qml"),
        )
        .files(["src/backend.rs"])
        .qrc("qml/assets.qrc")
        .cpp_files([
            "../../preview/preview-qt/include/gpu_surface.h",
            "../../preview/preview-qt/src/gpu_surface.cpp",
        ])
        .qt_module("Quick")
        .qt_module("OpenGL")
        .qt_module("Widgets")
        .cc_builder(|build| {
            build.include("../../preview/preview-qt/include");
        })
        .build();
    }
}
