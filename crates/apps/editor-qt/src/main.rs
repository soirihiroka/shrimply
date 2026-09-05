mod backend;

use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QString, QUrl};
use std::process::ExitCode;

fn main() -> ExitCode {
    shrimply_support::crash::install();
    shrimply_support::diagnostics::init();
    shrimply_i18n_qt::init_system_locale();
    shrimply_qt_components::init();
    shrimply_export_qt::init();
    shrimply_inspector_qt::init();
    let mut paths = std::env::args_os().skip(1);
    if paths.next().is_none() || paths.next().is_some() {
        eprintln!(
            "usage: shrimply-editor-qt PROJECT.shrimp|PROJECT.json|TIMELINE.otio|PROJECT.kdenlive"
        );
        return ExitCode::FAILURE;
    }
    backend::qobject::force_opengl();
    QGuiApplication::set_desktop_file_name(&QString::from("dev.shrimply.Shrimply.Qt"));
    let mut app = shrimply_qt_helpers::new_widget_application();
    let Some(mut app) = app.as_mut() else {
        eprintln!("could not create Qt application");
        return ExitCode::FAILURE;
    };
    app.as_mut()
        .set_application_name(&QString::from("shrimply-editor-qt"));
    app.as_mut()
        .set_application_display_name(&QString::from("Shrimply"));
    backend::qobject::configure_icons();
    backend::qobject::register_gpu_surfaces();

    let mut engine = QQmlApplicationEngine::new();
    let Some(mut engine) = engine.as_mut() else {
        eprintln!("could not create QML engine");
        return ExitCode::FAILURE;
    };
    let failed = engine.as_mut().on_object_creation_failed(|_, url| {
        eprintln!("could not load Qt editor UI: {url}");
        std::process::exit(1);
    });
    engine
        .as_mut()
        .load(&QUrl::from("qrc:/qt/qml/dev/shrimply/editor/qml/Main.qml"));
    let status = app.exec();
    drop(failed);

    let save_result = shrimply_project::project::shutdown_history();
    shrimply_project::project::clear_project_file_locks();
    if let Err(error) = save_result {
        tracing::error!(%error, "could not save project during shutdown");
        return ExitCode::FAILURE;
    }
    ExitCode::from(status.clamp(0, 255) as u8)
}
