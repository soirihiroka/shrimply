use cxx_qt_build::CxxQtBuilder;

fn main() {
    if cfg!(windows) {
        println!("cargo:rustc-link-lib=dwmapi");
    }
    unsafe {
        CxxQtBuilder::new()
            .file("src/lib.rs")
            .cpp_file("src/file_dialog.cpp")
            .qt_module("Gui")
            .qt_module("QuickControls2")
            .qt_module("Widgets")
            .cc_builder(|build| {
                build.include("include");
            })
            .build();
    }
}
