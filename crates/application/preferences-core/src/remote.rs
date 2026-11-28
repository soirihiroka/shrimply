//! Shared preference widgets and commands for native document-based hosts.
use std::{collections::HashMap, sync::mpsc};

use serde_json::{Value, json};
use shrimply_components_core::presentation::normalize_control;
use shrimply_i18n::text;
use shrimply_math_core::{Fraction, fraction_is_finite, fraction_new, fraction_scaled_integer};

use super::{PreferenceId as Id, PreferenceValue as StoredValue, *};

type ServerResult = Result<ComputeServerPresentation, String>;

pub struct PreferencesUi {
    sender: mpsc::Sender<(String, u64, ServerResult)>,
    receiver: mpsc::Receiver<(String, u64, ServerResult)>,
    servers: HashMap<String, Option<ServerResult>>,
    requests: HashMap<String, u64>,
    next_request: u64,
}

impl Default for PreferencesUi {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            sender,
            receiver,
            servers: HashMap::new(),
            requests: HashMap::new(),
            next_request: 0,
        }
    }
}

fn editor(label: &str, fields: Vec<Value>, command: Value) -> Value {
    json!({"widget":"editor","label":text(label),"enabled":true,"editable":true,"editor":{"title":text(label),"fields":fields,"command":command,"confirm_label":text("Apply"),"cancel_label":text("Cancel")}})
}

fn text_field(label: &str, value: &str) -> Value {
    json!({"label":text(label),"value":value,"keyboard":"text","multiline":false})
}

pub fn page(page: PreferencePage, controls: Vec<Value>) -> Value {
    let summary_controls =
        if controls.len() == 1 && controls[0]["label"] == text(page.title()).as_ref() {
            controls[0]["controls"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or(&[])
        } else {
            &controls
        };
    let summary = summary_controls
        .iter()
        .take(3)
        .filter_map(|group| group["label"].as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let mut document = json!({"widget":"group","label":text(page.title()),"icon":page.icon(),"color":page.color(),"summary":summary,"controls":controls});
    normalize_control(&mut document, &format!("preferences:{}", page.id()));
    document
}

fn preference(store: &SharedPreferences, id: Id) -> Value {
    let presentation = id.presentation();
    let command = json!({"action":"set","key":id.key()});
    let mut control = match value(store, id) {
        StoredValue::Boolean(value) => {
            let mut command = command;
            command["value"] = json!(!value);
            json!({"widget":"toggle","label":text(presentation.label),"selected":value,"command":command})
        }
        StoredValue::Integer(value) if !presentation.choices.is_empty() => {
            let selected = usize::try_from(value)
                .ok()
                .and_then(|index| presentation.choices.get(index))
                .map_or_else(String::new, |label| text(label).into_owned());
            json!({"widget":"choice","choice_style":"row","label":text(presentation.label),"display":selected,"choices":presentation.choices.iter().enumerate().map(|(index,label)| {
                let mut command = command.clone();
                command["value"] = json!(index);
                json!({"label":text(label),"command":command})
            }).collect::<Vec<_>>()})
        }
        StoredValue::Integer(value) => {
            let range = integer_range(id).expect("numeric preferences have ranges");
            let formatted = fraction_new(value, range.scale).to_string();
            let mut widget = editor(
                presentation.label,
                vec![
                    json!({"value":formatted,"minimum":fraction_new(range.minimum,range.scale).to_string(),"maximum":fraction_new(range.maximum,range.scale).to_string(),"step":fraction_new(range.step,range.scale).to_string(),"keyboard":"decimal","multiline":false}),
                ],
                command,
            );
            widget["display"] = json!(formatted);
            widget
        }
        StoredValue::Text(value) => {
            let mut widget = editor(presentation.label, vec![text_field("", &value)], command);
            widget["display"] = json!(value);
            widget
        }
        StoredValue::Color(color) => {
            let fields = [("R",color.r),("G",color.g),("B",color.b),("A",color.a)].into_iter().map(|(label,value)|json!({"label":label,"value":value.to_string(),"minimum":"0","maximum":u8::MAX.to_string(),"step":"1","keyboard":"decimal","multiline":false})).collect();
            let mut widget = editor(presentation.label, fields, command);
            widget["display"] = json!(format!(
                "#{:02X}{:02X}{:02X}{:02X}",
                color.r, color.g, color.b, color.a
            ));
            widget
        }
        StoredValue::FontFamily(family) => {
            json!({"widget":"font","label":text(presentation.label),"display":family.name(),"command":{"action":"font_open"}})
        }
    };
    control["subtitle"] = json!(text(presentation.subtitle));
    control
}

impl PreferencesUi {
    /// Network calls run away from the editor/render thread. Hosts refresh this
    /// schema while the dialog is visible; no host interprets server responses.
    pub fn snapshot(&mut self, store: &SharedPreferences) -> Value {
        while let Ok((url, request, status)) = self.receiver.try_recv() {
            if self.requests.get(&url) == Some(&request) {
                self.servers.insert(url, Some(status));
            }
        }
        let selected = snapshot(store).compute_server_url;
        if !self.servers.contains_key(&selected) {
            self.request_server(selected, None);
        }
        let mut groups: Vec<(&str, Vec<Value>)> = Vec::new();
        for id in Id::ALL {
            let presentation = id.presentation();
            if !presentation.available || presentation.page.is_none() {
                continue;
            }
            let index = groups
                .iter()
                .position(|(group, _)| *group == presentation.group)
                .unwrap_or_else(|| {
                    groups.push((presentation.group, Vec::new()));
                    groups.len() - 1
                });
            groups[index].1.push(preference(store, id));
        }
        let mut groups = groups.into_iter().collect::<HashMap<_, _>>();
        for group in self.compute_servers(store) {
            groups.insert(group.0, group.1);
        }
        let controls = PreferencePage::ALL.into_iter().filter_map(|id| {
            let controls = id.groups().iter().filter_map(|label| groups.remove(label).map(|controls| json!({"widget":"group","label":text(label),"controls":controls}))).collect::<Vec<_>>();
            (!controls.is_empty()).then(|| page(id, controls))
        }).collect::<Vec<_>>();
        json!({"title":text(SETTINGS_TITLE),"wide_width":SETTINGS_WIDE_WIDTH,"sidebar_width":SETTINGS_SIDEBAR_WIDTH,"back_label":text("Back"),"close_label":text("Close"),"controls":controls})
    }

    fn compute_servers(&self, store: &SharedPreferences) -> Vec<(&'static str, Vec<Value>)> {
        let settings = snapshot(store);
        let url = &settings.compute_server_url;
        let status = self.servers.get(url);
        let server = status
            .and_then(Option::as_ref)
            .and_then(|result| result.as_ref().ok());
        let mut add_server = editor(
            "Add Server",
            vec![text_field("URL", "")],
            json!({"action":"server_add"}),
        );
        add_server["icon"] = json!("list-add-symbolic");
        add_server["standalone"] = json!(true);
        let mut endpoints = vec![
            add_server,
            json!({"widget":"choice","choice_style":"row","label":text("Server"),"display":url,"choices":settings.compute_server_urls.iter().map(|url|json!({"label":url,"command":{"action":"server_select","url":url}})).collect::<Vec<_>>()}),
            editor(
                "Edit Server",
                vec![text_field("URL", url)],
                json!({"action":"server_edit","url":url}),
            ),
            json!({"widget":"button","label":text("Remove Server"),"enabled":settings.compute_server_urls.len()>1,"command":{"action":"server_remove","url":url}}),
            json!({"widget":"button","label":text("Check Connection"),"enabled":!matches!(status,Some(None)),"command":{"action":"server_check","url":url}}),
        ];
        match status {
            Some(None) => endpoints.push(json!({"widget":"loading","label":text("Connecting…")})),
            Some(Some(Err(error))) => {
                endpoints.push(json!({"widget":"text","label":text("Status"),"display":error}))
            }
            _ => {}
        }
        let row = |label: &str, display: &str| json!({"widget":"text","label":text(label),"display":display,"editable":false});
        let selected = vec![
            row(
                "Version",
                server.map_or("—", |server| server.version.as_str()),
            ),
            row(
                "Protocol",
                server.map_or("—", |server| server.protocol.as_str()),
            ),
        ];
        let mut compute = vec![
            row(
                "PyTorch",
                server.map_or("—", |server| server.torch.as_str()),
            ),
            row("CUDA", server.map_or("—", |server| server.cuda.as_str())),
        ];
        compute.push(json!({"widget":"choice","choice_style":"row","label":text("Device"),"display":server.and_then(|server|server.selected_device.and_then(|index|server.devices.get(index))).map_or("—",|device|device.label.as_str()),"enabled":server.is_some_and(|server|!server.devices.is_empty()),"choices":server.map(|server|server.devices.iter().map(|device|json!({"label":device.label,"command":{"action":"server_device","url":url,"device":device.id}})).collect::<Vec<_>>()).unwrap_or_default()}));
        compute.extend([
            row("Jobs", server.map_or("—", |server| server.jobs.as_str())),
            row(
                "Reserved memory",
                server.map_or("—", |server| server.reservations.as_str()),
            ),
            row(
                "Workers",
                server.map_or("—", |server| server.workers.as_str()),
            ),
        ]);
        vec![
            ("Inference Servers", endpoints),
            ("Selected Server", selected),
            ("Compute", compute),
            (
                "Features",
                vec![row(
                    "Available",
                    server.map_or("—", |server| server.features.as_str()),
                )],
            ),
        ]
    }

    fn request_server(&mut self, url: String, device: Option<String>) {
        self.next_request = self
            .next_request
            .checked_add(1)
            .expect("Preference server request overflow");
        let request = self.next_request;
        self.requests.insert(url.clone(), request);
        self.servers.insert(url.clone(), None);
        let sender = self.sender.clone();
        std::thread::spawn(move || {
            let status = match device {
                Some(device) => select_compute_device(&url, &device),
                None => compute_server_status(&url),
            }
            .map(|status| present_compute_server(&status));
            let _ = sender.send((url, request, status));
        });
    }

    pub fn apply(&mut self, store: &SharedPreferences, request: &Value) -> Result<Value, String> {
        let action = request["action"]
            .as_str()
            .ok_or("Preference action is required")?;
        match action {
            "set" => set_preference(store, request)?,
            "server_add" => {
                add_compute_server(store, editor_values(request, 1)?[0]).map_err(str::to_owned)?
            }
            "server_edit" => {
                let url = request["url"].as_str().ok_or("Server URL is required")?;
                edit_compute_server(store, url, editor_values(request, 1)?[0])
                    .map_err(str::to_owned)?;
                self.servers.remove(url);
                self.requests.remove(url);
            }
            "server_remove" | "server_select" | "server_check" | "server_device" => {
                let url = request["url"]
                    .as_str()
                    .filter(|url| {
                        snapshot(store)
                            .compute_server_urls
                            .iter()
                            .any(|item| item == url)
                    })
                    .ok_or("Server selection is no longer available")?;
                match action {
                    "server_remove" => {
                        remove_compute_server(store, url);
                        self.servers.remove(url);
                        self.requests.remove(url);
                    }
                    "server_select" => {
                        if !select_compute_server(store, url) {
                            return Err("Server selection is no longer available".into());
                        }
                    }
                    "server_check" | "server_device" => {
                        if self.servers.get(url).is_some_and(Option::is_none) {
                            return Err("Server request is already running".into());
                        }
                        let device = if action == "server_device" {
                            let device = request["device"]
                                .as_str()
                                .ok_or("Compute device is required")?;
                            let available = self
                                .servers
                                .get(url)
                                .and_then(Option::as_ref)
                                .and_then(|result| result.as_ref().ok())
                                .is_some_and(|status| {
                                    status.devices.iter().any(|item| item.id == device)
                                });
                            if !available {
                                return Err("Compute device is no longer available".into());
                            }
                            Some(device.to_owned())
                        } else {
                            None
                        };
                        self.request_server(url.to_owned(), device);
                    }
                    _ => unreachable!("server preference action dispatcher"),
                }
            }
            _ => return Err("Unknown preference action".into()),
        }
        Ok(self.snapshot(store))
    }
}

fn editor_values(request: &Value, count: usize) -> Result<Vec<&str>, String> {
    request["values"]
        .as_array()
        .filter(|values| values.len() == count)
        .ok_or("Preference editor values changed")?
        .iter()
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| "Preference editor values must be text".into())
        })
        .collect()
}

fn set_preference(store: &SharedPreferences, request: &Value) -> Result<(), String> {
    let id = request["key"]
        .as_str()
        .and_then(Id::from_key)
        .filter(|id| id.presentation().available)
        .ok_or("Preference is unavailable")?;
    let value = match value(store, id) {
        StoredValue::Boolean(_) => StoredValue::Boolean(
            request["value"]
                .as_bool()
                .ok_or("Preference toggle requires a boolean")?,
        ),
        StoredValue::Integer(_) => {
            let range = integer_range(id).expect("numeric preference range");
            let value = if id.presentation().choices.is_empty() {
                let value: Fraction = editor_values(request, 1)?[0]
                    .parse()
                    .map_err(|_| "Invalid preference number")?;
                if !fraction_is_finite(value) {
                    return Err("Preference number must be finite".into());
                }
                fraction_scaled_integer(value, range.scale)
            } else {
                request["value"]
                    .as_i64()
                    .ok_or("Preference choice is required")?
            };
            if value < range.minimum || value > range.maximum {
                return Err("Preference number is outside its range".into());
            }
            StoredValue::Integer(value)
        }
        StoredValue::Text(_) => StoredValue::Text(editor_values(request, 1)?[0].to_owned()),
        StoredValue::Color(_) => {
            let values = editor_values(request, 4)?
                .into_iter()
                .map(|value| {
                    value
                        .parse::<u8>()
                        .map_err(|_| "Color channels must be integers from 0 to 255")
                })
                .collect::<Result<Vec<_>, _>>()?;
            StoredValue::Color(Color::new(values[0], values[1], values[2], values[3]))
        }
        StoredValue::FontFamily(_) => {
            let values = editor_values(request, 2)?;
            let family = decode_font_family(&format!("{}\t{}", values[0], values[1]))
                .ok_or("Font source or family is invalid")?;
            StoredValue::FontFamily(family)
        }
    };
    set_value(store, id, value).map_err(str::to_owned)
}
