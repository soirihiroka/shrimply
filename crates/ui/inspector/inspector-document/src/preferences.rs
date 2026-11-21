//! Native preferences reuse the GTK font browser and trust operations.
mod trust;
use serde_json::{Value, json};
use shrimply_preferences_core::{self as preferences, PreferencePage, SharedPreferences};

#[derive(Default)]
pub struct Preferences {
    preferences: preferences::remote::PreferencesUi,
    fonts: crate::font_picker::Picker,
    trust: trust::Sources,
}

impl Preferences {
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
        let pages = document["controls"]
            .as_array_mut()
            .expect("preference pages");
        pages.push(self.trust.page());
        pages.sort_by_key(|page| {
            PreferencePage::ALL
                .iter()
                .position(|id| page["id"] == format!("preferences:{}", id.id()))
                .expect("known preference page")
        });
        if let Some(picker) = self.fonts.snapshot(
            &[preferences::snapshot(store).default_text_font_family],
            "preferences",
        ) {
            document["font_picker"] = picker;
        }
        self.trust.decorate(&mut document);
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
