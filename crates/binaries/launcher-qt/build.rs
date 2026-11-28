use cxx_qt_build::{CxxQtBuilder, QmlModule};

fn main() {
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        println!("cargo:rustc-link-arg-bin=shrimply=/STACK:8388608");
        // The launcher checks the driver before editor startup. CLI edits do not need it.
        println!("cargo:rustc-link-arg-bin=shrimply=/DELAYLOAD:nvcuda.dll");
        println!("cargo:rustc-link-arg-bin=shrimply=delayimp.lib");
    }

    CxxQtBuilder::new_qml_module(QmlModule::new("dev.shrimply.launcher").qml_file("qml/Main.qml"))
        .files(["src/backend.rs"])
        .qt_module("Widgets")
        .build();
}
