#[cfg(unix)]
use std::path::Path;
use std::process::{Command, Output};

/// The path currently providing the command, including an existing user launcher.
pub fn status() -> Result<Option<String>, String> {
    let path = run("status")?;
    Ok((!path.is_empty()).then_some(path))
}

/// Install the running distribution as the user's terminal command.
pub fn install() -> Result<String, String> {
    run("install")
}

fn run(action: &str) -> Result<String, String> {
    #[cfg(unix)]
    let mut command = {
        let (launcher, native_path) = if action == "install" {
            launcher_command()?
        } else {
            (String::new(), String::new())
        };
        let mut command = host_command("/bin/sh");
        command.args([
            "-c",
            include_str!("cli-unix.sh"),
            "shrimply-cli",
            action,
            &launcher,
            &native_path,
        ]);
        command
    };
    #[cfg(windows)]
    let mut command = {
        use std::os::windows::process::CommandExt;
        use windows::Win32::System::Threading::CREATE_NO_WINDOW;
        let launcher = std::env::current_exe()
            .map_err(|error| format!("Locate Shrimply: {error}"))?
            .with_file_name("shrimply.exe");
        let mut command = Command::new("powershell.exe");
        command
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                include_str!("cli-windows.ps1"),
            ])
            .env("SHRIMPLY_CLI_ACTION", action)
            .env("SHRIMPLY_CLI_LAUNCHER", launcher)
            .creation_flags(CREATE_NO_WINDOW.0);
        command
    };
    output(
        command
            .output()
            .map_err(|error| format!("Configure CLI: {error}"))?,
    )
}

fn output(output: Output) -> Result<String, String> {
    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(if error.is_empty() {
            format!("CLI setup exited with {}", output.status)
        } else {
            error
        });
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_owned())
        .map_err(|error| format!("Read CLI setup result: {error}"))
}

#[cfg(unix)]
fn host_command(program: &str) -> Command {
    if std::env::var_os("FLATPAK_ID").is_some() {
        let mut command = Command::new("flatpak-spawn");
        command.args(["--host", "--directory=/", program]);
        command
    } else {
        Command::new(program)
    }
}

#[cfg(unix)]
fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(unix)]
fn launcher_command() -> Result<(String, String), String> {
    if let Some(id) = std::env::var_os("FLATPAK_ID") {
        let metadata = std::fs::read_to_string("/.flatpak-info")
            .map_err(|error| format!("Read Flatpak installation: {error}"))?;
        let mut section = "";
        let mut values = std::collections::HashMap::new();
        for line in metadata.lines().map(str::trim) {
            if line.starts_with('[') {
                section = line;
            } else if section == "[Instance]"
                && let Some((key, value)) = line.split_once('=')
            {
                values.insert(key, value);
            }
        }
        let arch = values
            .get("arch")
            .ok_or("Flatpak architecture is missing")?;
        let branch = values.get("branch").ok_or("Flatpak branch is missing")?;
        let app_path = values
            .get("original-app-path")
            .or_else(|| values.get("app-path"))
            .ok_or("Flatpak application path is missing")?;
        let deployment = Path::new(app_path)
            .parent()
            .and_then(Path::parent)
            .ok_or("Flatpak application path is invalid")?;
        let reference = format!(
            "{}/{arch}/{branch}",
            id.to_str().ok_or("Flatpak app ID is not UTF-8")?
        );
        let installations = output(
            host_command("flatpak")
                .args(["list", "--app", "--columns=installation:full"])
                .output()
                .map_err(|error| format!("List Flatpak installations: {error}"))?,
        )?;
        for installation in installations.lines().map(str::trim) {
            let scope = match installation {
                "user" => "--user".to_owned(),
                "system" => "--system".to_owned(),
                name => format!("--installation={name}"),
            };
            let location = host_command("flatpak")
                .args(["info", &scope, "--show-location", &reference])
                .output()
                .map_err(|error| format!("Locate Flatpak installation: {error}"))?;
            if let Ok(location) = output(location)
                && Path::new(&location).parent() == Some(deployment)
            {
                return Ok((
                    format!(
                        "shrimply_cli_filesystem=\"$PWD:rw\"\n[ \"$PWD\" != / ] || shrimply_cli_filesystem=host\nexec flatpak run {} --command=shrimply --cwd=\"$PWD\" --filesystem=/tmp --filesystem=\"$shrimply_cli_filesystem\" {} \"$@\"",
                        quote(&scope),
                        quote(&reference),
                    ),
                    String::new(),
                ));
            }
        }
        return Err("The running Flatpak installation could not be found on the host".into());
    }
    let launcher = std::env::current_exe()
        .map_err(|error| format!("Locate Shrimply: {error}"))?
        .with_file_name("shrimply");
    #[cfg(target_os = "macos")]
    if launcher
        .components()
        .any(|component| component.as_os_str() == "AppTranslocation")
    {
        return Err(
            "Move Shrimply.app to Applications and reopen it before installing the CLI".into(),
        );
    }
    if !launcher.is_file() {
        return Err(format!(
            "Shrimply launcher not found: {}",
            launcher.display()
        ));
    }
    let path = launcher.to_str().ok_or("Shrimply path is not UTF-8")?;
    Ok((format!("exec {} \"$@\"", quote(path)), path.to_owned()))
}
