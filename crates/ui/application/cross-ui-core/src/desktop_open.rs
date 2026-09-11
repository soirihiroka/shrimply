#[cfg(unix)]
use gio::prelude::DBusProxyExt;
#[cfg(unix)]
use glib::variant::ToVariant;
#[cfg(unix)]
use std::fs::File;
use std::path::{Path, PathBuf};

#[cfg(unix)]
const DEFAULT_DBUS_TIMEOUT: i32 = -1;

pub enum Action {
    Open(PathBuf),
    FocusRevealed(PathBuf),
}

pub fn prepare(path: &Path, activation_token: Option<&str>) -> Result<Action, String> {
    let path = absolute_path(path);
    let metadata = path
        .metadata()
        .map_err(|error| format!("Unable to inspect {}: {error}", path.display()))?;
    if metadata.is_dir() {
        return Ok(Action::Open(path));
    }

    reveal_file(&path, activation_token);
    Ok(Action::FocusRevealed(
        path.parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf(),
    ))
}

fn reveal_file(path: &Path, activation_token: Option<&str>) {
    #[cfg(windows)]
    {
        let _ = activation_token;
        match std::process::Command::new("explorer.exe")
            .arg(format!("/select,{}", path.display()))
            .spawn()
        {
            Ok(_) => tracing::info!(path = %path.display(), "file reveal started"),
            Err(error) => tracing::warn!(path = %path.display(), %error, "file reveal failed"),
        }
    }

    #[cfg(unix)]
    {
        let path_display = path.display();
        let file = match File::open(path) {
            Ok(file) => file,
            Err(error) => {
                tracing::warn!("file reveal fd open failed path={path_display}: {error}");
                return;
            }
        };
        let fd_list = gio::UnixFDList::from_array([file]);
        let options = glib::VariantDict::default();
        if let Some(token) = activation_token {
            options.insert("activation_token", token);
        } else {
            tracing::debug!("file reveal activation token unavailable path={path_display}");
        }
        let parameters = ("", glib::variant::Handle::from(0), options).to_variant();
        let proxy = gio::DBusProxy::for_bus_sync(
            gio::BusType::Session,
            gio::DBusProxyFlags::DO_NOT_LOAD_PROPERTIES
                | gio::DBusProxyFlags::DO_NOT_CONNECT_SIGNALS,
            None,
            "org.freedesktop.portal.Desktop",
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.OpenURI",
            gio::Cancellable::NONE,
        );
        let result = proxy.and_then(|proxy| {
            proxy
                .call_with_unix_fd_list_sync(
                    "OpenDirectory",
                    Some(&parameters),
                    gio::DBusCallFlags::NONE,
                    DEFAULT_DBUS_TIMEOUT,
                    Some(&fd_list),
                    gio::Cancellable::NONE,
                )
                .map(|_| ())
        });
        match result {
            Ok(()) => tracing::info!("file reveal portal complete path={path_display}"),
            Err(error) => tracing::warn!("file reveal portal failed path={path_display}: {error}"),
        }
    }
}

fn absolute_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    }
}
