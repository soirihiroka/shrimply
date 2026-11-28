use serde_json::{Value, json};
use shrimply_i18n::text;
use shrimply_preferences_core::{self as preferences, SharedPreferences};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};

#[derive(Default)]
pub(super) struct Settings {
    blender_picker: bool,
    blender: Option<Receiver<Result<PathBuf, String>>>,
    cli: Option<Receiver<Result<Option<String>, String>>>,
    cli_status: Option<Result<Option<String>, String>>,
    error: Option<String>,
}

impl Settings {
    pub(super) fn pending(&self) -> bool {
        self.blender.is_some()
    }

    pub(super) fn decorate(&mut self, store: &SharedPreferences, document: &mut Value) {
        if let Some(result) = self.blender.as_ref().and_then(poll) {
            self.blender = None;
            match result {
                Ok(path) => preferences::apply_blender_binary(store, Some(path)),
                Err(error) => self.error = Some(error),
            }
        }
        if let Some(result) = self.cli.as_ref().and_then(poll) {
            self.cli = None;
            self.cli_status = Some(result);
        }
        if self.cli_status.is_none() && self.cli.is_none() {
            self.check_cli(false);
        }
        if self.blender_picker {
            document["source_picker"] = json!({
                "folder":false,"title":text("Choose Blender Binary"),
                "complete":{"action":"desktop_blender_set"},"cancel":{"action":"desktop_blender_cancel"}
            });
        }
        let Some(groups) = document["controls"]
            .as_array_mut()
            .and_then(|pages| {
                pages
                    .iter_mut()
                    .find(|page| page["id"] == "preferences:external")
            })
            .and_then(|page| page["controls"].as_array_mut())
        else {
            return;
        };
        let binary = preferences::snapshot(store).blender_binary;
        if let Some(group) = groups
            .iter_mut()
            .find(|group| group["label"] == text("Blender").as_ref())
        {
            group["controls"] = json!([
                {"widget":"text","label":text("Blender Binary"),"display":binary.as_ref().map(|path| path.display().to_string()).unwrap_or_else(||text("Not configured").into_owned())},
                {"widget":"button","label":text("Choose…"),"enabled":self.blender.is_none(),"command":{"action":"desktop_blender_pick"}},
                {"widget":"button","label":text("Clear"),"enabled":binary.is_some() && self.blender.is_none(),"command":{"action":"desktop_blender_clear"}}
            ]);
            if self.blender.is_some() {
                group["controls"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"widget":"loading","label":text("Checking Blender…")}));
            }
            if let Some(error) = &self.error {
                group["controls"].as_array_mut().unwrap().push(json!({"widget":"text","label":text("Could not configure Blender"),"display":error}));
            }
        }
        let installed = self
            .cli_status
            .as_ref()
            .and_then(|result| result.as_ref().ok())
            .and_then(Option::as_deref);
        let label = if installed.is_some() {
            "Overwrite"
        } else {
            "Install"
        };
        let status = match &self.cli_status {
            Some(Err(error)) => error,
            _ => installed.unwrap_or("shrimply"),
        };
        let mut group = json!({"widget":"group","label":text("Command Line"),"controls":[
            {"widget":"text","label":text("Command Line Tool"),"display":status},
            {"widget":"button","label":text(label),"enabled":self.cli.is_none(),"command":{"action":"desktop_cli_install"}}
        ]});
        shrimply_components_core::presentation::normalize_control(
            &mut group,
            "settings:command-line",
        );
        groups.push(group);
    }

    pub(super) fn apply(
        &mut self,
        store: &SharedPreferences,
        request: &Value,
    ) -> Result<(), String> {
        match request["action"].as_str() {
            Some("desktop_cli_install") if self.cli.is_none() => self.check_cli(true),
            Some("desktop_blender_pick") if self.blender.is_none() => self.blender_picker = true,
            Some("desktop_blender_cancel") => self.blender_picker = false,
            Some("desktop_blender_clear") if self.blender.is_none() => {
                self.error = None;
                preferences::apply_blender_binary(store, None);
            }
            Some("desktop_blender_set") if self.blender_picker && self.blender.is_none() => {
                self.blender_picker = false;
                self.error = None;
                let path = PathBuf::from(
                    request["path"]
                        .as_str()
                        .ok_or("Blender binary is required")?,
                );
                let (send, receive) = mpsc::channel();
                std::thread::spawn(move || {
                    let _ = send.send(preferences::validate_blender_binary(&path));
                });
                self.blender = Some(receive);
            }
            Some("desktop_font") => {
                let name = request["name"]
                    .as_str()
                    .filter(|name| !name.is_empty())
                    .ok_or("Font family is required")?;
                preferences::set_default_text_font_family(
                    store,
                    preferences::FontFamily::Local { name: name.into() },
                );
            }
            _ => return Err("Settings action is unavailable".into()),
        }
        Ok(())
    }

    fn check_cli(&mut self, install: bool) {
        let (send, receive) = mpsc::channel();
        std::thread::spawn(move || {
            let result = if install {
                preferences::cli::install().map(Some)
            } else {
                preferences::cli::status()
            };
            let _ = send.send(result);
        });
        self.cli = Some(receive);
    }
}

fn poll<T>(result: &Receiver<Result<T, String>>) -> Option<Result<T, String>> {
    match result.try_recv() {
        Ok(result) => Some(result),
        Err(mpsc::TryRecvError::Empty) => None,
        Err(mpsc::TryRecvError::Disconnected) => Some(Err("Settings task stopped".into())),
    }
}
