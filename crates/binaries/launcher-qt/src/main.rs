mod backend;

use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QString, QUrl};
use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    shrimply_process_reporting::diagnostics::init();
    shrimply_i18n_qt::init_system_locale();

    if let Err(error) = shrimply_cross_ui_core::launcher::check_nvidia_gpu() {
        tracing::error!("{error}");
        let app = shrimply_application_qt::new_widget_application();
        if app.is_null() {
            eprintln!("could not create Qt application");
            return ExitCode::FAILURE;
        }
        backend::qobject::show_startup_error(
            &shrimply_i18n_qt::text(shrimply_cross_ui_core::launcher::UNSUPPORTED_GPU_HEADING),
            &shrimply_i18n_qt::text(shrimply_cross_ui_core::launcher::UNSUPPORTED_GPU_MESSAGE),
        );
        return ExitCode::FAILURE;
    }

    let mut args = std::env::args_os().skip(1);
    if let Some(path) = args.next() {
        if args.next().is_some() {
            eprintln!(
                "usage: shrimply-qt [PROJECT.shrimp|PROJECT.json|TIMELINE.otio|PROJECT.kdenlive]"
            );
            return ExitCode::FAILURE;
        }
        return match shrimply_cross_ui_core::launcher::launch_qt_editor(Path::new(&path)).and_then(
            |mut editor| {
                editor
                    .wait()
                    .map_err(|error| format!("could not wait for editor: {error}"))
            },
        ) {
            Ok(status) if status.success() => ExitCode::SUCCESS,
            Ok(status) => {
                eprintln!("editor exited with {status}");
                ExitCode::FAILURE
            }
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        };
    }

    QGuiApplication::set_desktop_file_name(&QString::from("dev.shrimply.Shrimply.Qt"));
    let mut app = shrimply_application_qt::new_widget_application();
    let Some(mut app) = app.as_mut() else {
        eprintln!("could not create Qt application");
        return ExitCode::FAILURE;
    };
    backend::qobject::set_breeze_icon_fallback();
    app.as_mut()
        .set_application_name(&QString::from("shrimply-qt"));
    app.as_mut()
        .set_application_display_name(&QString::from("Shrimply"));
    let mut engine = QQmlApplicationEngine::new();
    let Some(mut engine) = engine.as_mut() else {
        eprintln!("could not create QML engine");
        return ExitCode::FAILURE;
    };
    let failed = engine.as_mut().on_object_creation_failed(|_, url| {
        eprintln!("could not load Qt launcher UI: {url}");
        std::process::exit(1);
    });
    engine.as_mut().load(&QUrl::from(
        "qrc:/qt/qml/dev/shrimply/launcher/qml/Main.qml",
    ));
    let status = app.exec();
    drop(failed);
    ExitCode::from(status.clamp(0, 255) as u8)
}
