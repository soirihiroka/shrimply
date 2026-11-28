mod backend;

use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QString, QUrl};
use std::process::ExitCode;

fn main() -> ExitCode {
    if let Some(status) = shrimply_cli::run_if_requested() {
        return ExitCode::from(status);
    }
    let edit = match shrimply_cross_ui_core::launcher::project_path_argument() {
        Ok(path) => path.is_some(),
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    #[cfg(windows)]
    if edit && std::env::var_os("CUDA_MODULE_LOADING").is_none() {
        // CLI dispatch returned without running commands or starting threads.
        unsafe { std::env::set_var("CUDA_MODULE_LOADING", "EAGER") };
    }
    shrimply_process_reporting::diagnostics::init();
    shrimply_i18n_qt::init_system_locale();
    shrimply_components_qt::init();

    if edit && let Err(error) = shrimply_cross_ui_core::launcher::check_nvidia_gpu() {
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

    shrimply_editor_qt::register_qml();
    if edit {
        shrimply_editor_qt::configure_editor();
    }
    QGuiApplication::set_desktop_file_name(&QString::from("dev.shrimply.Shrimply.Qt"));
    let mut app = shrimply_application_qt::new_widget_application();
    let Some(mut app) = app.as_mut() else {
        eprintln!("could not create Qt application");
        return ExitCode::FAILURE;
    };
    app.as_mut()
        .set_application_name(&QString::from("shrimply"));
    app.as_mut()
        .set_application_display_name(&QString::from("Shrimply"));
    if edit {
        shrimply_editor_qt::register_gpu_surfaces();
    }
    let mut engine = QQmlApplicationEngine::new();
    let Some(mut engine) = engine.as_mut() else {
        eprintln!("could not create QML engine");
        return ExitCode::FAILURE;
    };
    let failed = engine.as_mut().on_object_creation_failed(|_, url| {
        eprintln!("could not load Shrimply UI: {url}");
        std::process::exit(1);
    });
    engine.as_mut().load(&QUrl::from(if edit {
        shrimply_editor_qt::MAIN_QML
    } else {
        "qrc:/qt/qml/dev/shrimply/launcher/qml/Main.qml"
    }));
    shrimply_application_qt::apply_windows_system_backdrop();
    let status = app.exec();
    drop(failed);
    if edit && let Err(error) = shrimply_editor_qt::save_on_exit() {
        tracing::error!(%error, "could not save project during shutdown");
        return ExitCode::FAILURE;
    }
    ExitCode::from(status.clamp(0, 255) as u8)
}
