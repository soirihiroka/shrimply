use serde_json::{Value, json};
use shrimply_i18n::text;
use shrimply_preferences_core::{self as preferences, SharedPreferences};
use shrimply_project_service::{self as service, ProjectClient};
use std::sync::mpsc::{self, Receiver, TryRecvError};

type SignIn = Result<(Option<String>, String), String>;

#[derive(Default)]
pub(super) struct Settings {
    sign_in: Option<Receiver<SignIn>>,
    error: Option<String>,
}

impl Settings {
    pub(super) fn pending(&self) -> bool {
        self.sign_in.is_some()
    }

    pub fn decorate(&mut self, store: &SharedPreferences, document: &mut Value) {
        if let Some(receiver) = &self.sign_in {
            let result = match receiver.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => {
                    Some(Err("Project server sign-in stopped".into()))
                }
            };
            if let Some(result) = result {
                self.sign_in = None;
                self.error = result
                    .and_then(|(previous, url)| {
                        service::update_server(store, previous.as_deref(), Some(&url))
                    })
                    .err();
            }
        }
        let Some(servers) = document["controls"].as_array_mut().and_then(|pages| {
            pages
                .iter_mut()
                .find(|page| page["id"] == "preferences:servers")
        }) else {
            return;
        };
        let mut rows = vec![editor("Add Project Server", None, self.sign_in.is_none())];
        for url in preferences::snapshot(store).project_server_urls {
            rows.push(editor("Sign In", Some(&url), self.sign_in.is_none()));
            rows.push(json!({
                "widget":"button","label":text("Remove Project Server"),"subtitle":url,
                "enabled":self.sign_in.is_none(),
                "command":{"action":"project_server_remove","url":url}
            }));
        }
        if self.sign_in.is_some() {
            rows.push(json!({"widget":"loading","label":text("Signing In…")}));
        }
        if let Some(error) = &self.error {
            rows.push(json!({"widget":"text","label":text("Could not sign in"),"display":error}));
        }
        let mut group = json!({"widget":"group","label":text("Project Servers"),"controls":rows});
        shrimply_components_core::presentation::normalize_control(
            &mut group,
            "settings:project-servers",
        );
        servers["controls"]
            .as_array_mut()
            .expect("Server settings groups")
            .insert(0, group);
    }

    pub fn apply(&mut self, store: &SharedPreferences, request: &Value) -> Result<(), String> {
        if self.sign_in.is_some() {
            return Err("Project server sign-in is already running".into());
        }
        self.error = None;
        let previous = request["url"].as_str().map(str::to_owned);
        match request["action"].as_str() {
            Some("project_server_remove") => service::update_server(
                store,
                Some(
                    previous
                        .as_deref()
                        .ok_or("Project server URL is required")?,
                ),
                None,
            ),
            Some("project_server_sign_in") => {
                let values = request["values"]
                    .as_array()
                    .filter(|values| values.len() == 3)
                    .ok_or("Enter the server URL, username, and password")?;
                let field = |index: usize| values[index].as_str().unwrap_or_default();
                let url = preferences::normalize_project_server_url(field(0))?;
                let username = field(1).to_owned();
                let password = field(2).to_owned();
                let (sender, receiver) = mpsc::channel();
                std::thread::spawn(move || {
                    let result =
                        ProjectClient::sign_in(&url, &username, &password).map(|_| (previous, url));
                    let _ = sender.send(result);
                });
                self.sign_in = Some(receiver);
                Ok(())
            }
            _ => Err("Unknown project server action".into()),
        }
    }
}

fn editor(label: &str, url: Option<&str>, enabled: bool) -> Value {
    json!({
        "widget":"editor","label":text(label),"display":url.unwrap_or_default(),"enabled":enabled,
        "editable":true,"editor":{"title":text("Sign In to Project Server"),"fields":[
            {"label":text("Server URL"),"value":url.unwrap_or_default(),"keyboard":"text","multiline":false},
            {"label":text("Username"),"value":"","keyboard":"text","multiline":false},
            {"label":text("Password"),"value":"","keyboard":"password","multiline":false}
        ],"command":{"action":"project_server_sign_in","url":url},"confirm_label":text("Sign In"),"cancel_label":text("Cancel")}
    })
}
