use cxx_qt_lib::QString;
use shrimply_preferences_core::{self as preferences, PreferenceId, PreferenceValue};
use std::path::{Path, PathBuf};

pub use shrimply_preferences_core::IntegerRange;
pub use shrimply_preferences_core::ServerStatus;

pub struct Connector {
    preferences: preferences::SharedPreferences,
}

impl Connector {
    pub fn new(preferences: preferences::SharedPreferences) -> Self {
        Self { preferences }
    }

    pub fn value(&self, key: &QString) -> QString {
        let id =
            PreferenceId::from_key(&key.to_string()).expect("Qt requested an unknown preference");
        QString::from(match preferences::value(&self.preferences, id) {
            PreferenceValue::Boolean(value) => value.to_string(),
            PreferenceValue::Integer(value) => value.to_string(),
            PreferenceValue::Text(value) => value,
            PreferenceValue::FontFamily(value) => value.name().to_string(),
            PreferenceValue::Color(value) => format!(
                "#{:02x}{:02x}{:02x}{:02x}",
                value.a, value.r, value.g, value.b
            ),
        })
    }

    pub fn integer_range(&self, key: &QString) -> IntegerRange {
        let id = PreferenceId::from_key(&key.to_string())
            .expect("Qt requested an unknown preference range");
        preferences::integer_range(id).expect("Qt requested a non-numeric preference range")
    }

    pub fn set_value(&self, key: &QString, value: &QString) -> Result<(), &'static str> {
        let id = PreferenceId::from_key(&key.to_string()).ok_or("Unknown preference")?;
        let value = value.to_string();
        let value = match preferences::value(&self.preferences, id) {
            PreferenceValue::Boolean(_) => {
                PreferenceValue::Boolean(value.parse().map_err(|_| "Preference must be a boolean")?)
            }
            PreferenceValue::Integer(_) => PreferenceValue::Integer(
                value.parse().map_err(|_| "Preference must be an integer")?,
            ),
            PreferenceValue::Text(_) => PreferenceValue::Text(value),
            PreferenceValue::FontFamily(_) => {
                PreferenceValue::FontFamily(shrimply_preferences_core::FontFamily::Local {
                    name: value,
                })
            }
            PreferenceValue::Color(_) => PreferenceValue::Color(parse_color(&value)?),
        };
        preferences::set_value(&self.preferences, id, value)
    }

    pub fn validate_blender_binary(path: &Path) -> Result<PathBuf, String> {
        preferences::validate_blender_binary(path)
    }

    pub fn apply_blender_binary(&self, path: PathBuf) {
        preferences::apply_blender_binary(&self.preferences, Some(path));
    }

    pub fn clear_blender_binary(&self) {
        preferences::apply_blender_binary(&self.preferences, None);
    }

    pub fn compute_servers(&self) -> (Vec<String>, usize) {
        preferences::compute_servers(&self.preferences)
    }

    pub fn add_compute_server(&self, value: &str) -> Result<(), &'static str> {
        preferences::add_compute_server(&self.preferences, value)
    }

    pub fn edit_compute_server(&self, index: usize, value: &str) -> Result<(), &'static str> {
        let previous = preferences::snapshot(&self.preferences)
            .compute_server_urls
            .get(index)
            .cloned()
            .ok_or("Server selection is no longer available")?;
        preferences::edit_compute_server(&self.preferences, &previous, value)
    }

    pub fn remove_compute_server(&self, index: usize) {
        if let Some(url) = preferences::snapshot(&self.preferences)
            .compute_server_urls
            .get(index)
            .cloned()
        {
            preferences::remove_compute_server(&self.preferences, &url);
        }
    }

    pub fn select_compute_server(&self, index: usize) {
        if let Some(url) = preferences::snapshot(&self.preferences)
            .compute_server_urls
            .get(index)
            .cloned()
        {
            preferences::select_compute_server(&self.preferences, &url);
        }
    }

    pub fn selected_compute_server(&self) -> String {
        preferences::snapshot(&self.preferences).compute_server_url
    }

    pub fn compute_server_status(url: &str) -> Result<ServerStatus, String> {
        preferences::compute_server_status(url)
    }

    pub fn select_compute_device(url: &str, device: &str) -> Result<ServerStatus, String> {
        preferences::select_compute_device(url, device)
    }
}

fn parse_color(value: &str) -> Result<shrimply_math_color::Color<u8>, &'static str> {
    let digits = value.trim().trim_start_matches('#');
    let value = u32::from_str_radix(digits, 16).map_err(|_| "Invalid color")?;
    match digits.len() {
        8 => Ok(shrimply_math_color::Color::new(
            ((value >> 16) & 0xff) as u8,
            ((value >> 8) & 0xff) as u8,
            (value & 0xff) as u8,
            (value >> 24) as u8,
        )),
        6 => Ok(shrimply_math_color::Color::from_rgb_u32(value)),
        _ => Err("Color must be #RRGGBB or #AARRGGBB"),
    }
}
