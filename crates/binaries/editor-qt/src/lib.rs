mod backend;

pub use backend::qobject::register_gpu_surfaces;

pub const MAIN_QML: &str = "qrc:/qt/qml/dev/shrimply/editor/qml/Main.qml";

pub fn configure() {
    shrimply_process_reporting::crash::install();
    cxx_qt::init_qml_module!("dev.shrimply.editor");
    shrimply_export_qt::init();
    shrimply_inspector_qt::init();
    shrimply_preferences_qt::init();
    backend::qobject::force_opengl();
}

pub fn save_on_exit() -> Result<(), String> {
    use shrimply_project_document::project;
    let result = project::shutdown_history();
    project::clear_project_file_locks();
    result
}
