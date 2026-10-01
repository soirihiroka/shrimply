use cxx_qt_build::{CxxQtBuilder, QmlModule};

fn main() {
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        println!("cargo:rustc-link-arg-bin=shrimply-qt=/STACK:8388608");
    }

    CxxQtBuilder::new_qml_module(QmlModule::new("dev.shrimply.launcher").qml_file("qml/Main.qml"))
        .files(["src/backend.rs"])
        .qrc("../../ui/application/application-qt/icons.qrc")
        .qt_module("Widgets")
        .build();
}
