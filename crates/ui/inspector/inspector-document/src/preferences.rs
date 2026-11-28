//! Native preferences reuse the GTK font browser and trust operations.
#[cfg(not(target_os = "android"))]
mod desktop;
mod project_servers;
mod trust;
use serde_json::{Value, json};
use shrimply_preferences_core::{self as preferences, SharedPreferences};

#[derive(Default)]
pub struct Preferences {
    preferences: preferences::remote::PreferencesUi,
    fonts: crate::font_picker::Picker,
    trust: trust::Sources,
    servers: project_servers::Settings,
    #[cfg(not(target_os = "android"))]
    desktop: desktop::Settings,
}

impl Preferences {
    pub fn has_pending_changes(&self) -> bool {
        #[cfg(not(target_os = "android"))]
        if self.desktop.pending() {
            return true;
        }
        self.servers.pending()
    }

    fn poll_fonts(&mut self, store: &SharedPreferences) {
        for family in self.fonts.poll().1 {
            preferences::set_default_text_font_family(store, family);
        }
    }

    pub fn snapshot(&mut self, store: &SharedPreferences) -> Value {
        self.poll_fonts(store);
        let document = self.preferences.snapshot(store);
        self.document(store, document)
    }

    fn document(&mut self, store: &SharedPreferences, mut document: Value) -> Value {
        self.servers.decorate(store, &mut document);
        let trust = document["controls"]
            .as_array_mut()
            .expect("preference pages")
            .iter_mut()
            .find(|page| page["id"] == "preferences:trust")
            .expect("Trust settings page");
        *trust = self.trust.page();
        if let Some(picker) = self.fonts.snapshot(
            &[preferences::snapshot(store).default_text_font_family],
            "preferences",
        ) {
            document["font_picker"] = picker;
        }
        self.trust.decorate(&mut document);
        #[cfg(not(target_os = "android"))]
        self.desktop.decorate(store, &mut document);
        document["pending"] = json!(self.has_pending_changes());
        document["close_command"] = json!({"action":"preferences_close"});
        document
    }

    pub fn apply(&mut self, store: &SharedPreferences, request: &Value) -> Result<Value, String> {
        self.poll_fonts(store);
        let action = request["action"]
            .as_str()
            .ok_or("Preference action is required")?;
        match action {
            "preferences_close" => {
                self.fonts.close();
                self.trust = Default::default();
            }
            action if action.starts_with("project_server_") => {
                self.servers.apply(store, request)?;
            }
            #[cfg(not(target_os = "android"))]
            action if action.starts_with("desktop_") => {
                self.desktop.apply(store, request)?;
            }
            action if action.starts_with("font_") => {
                self.fonts.apply(request)?;
            }
            action if action.starts_with("trust_") || action == "dialog_close" => {
                let reveal = self.trust.apply(request)?;
                let mut document = self.snapshot(store);
                if let Some(path) = reveal {
                    document["show_in_files"] = json!(path);
                }
                return Ok(document);
            }
            _ => {
                let document = self.preferences.apply(store, request)?;
                return Ok(self.document(store, document));
            }
        }
        Ok(self.snapshot(store))
    }
}
