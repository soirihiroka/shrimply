use super::{SharedPreferences, notify_listeners, read_string_or_default, write_string};
use rusqlite::Connection;
use serde_json::{Value, json};

const KEY: &str = "editor_layout";
const DEFAULT_INSPECTOR_WIDTH: f32 = shrimply_components_core::layout::INSPECTOR_MIN_WIDTH;
const MAX_INSPECTOR_WIDTH: f32 = 4096.0;
const MIN_TIMELINE_FRACTION: f32 = 0.05;
const MAX_TIMELINE_FRACTION: f32 = 0.95;

/// Desired layout, independent of temporary viewport constraints and fullscreen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EditorLayout {
    pub inspector_visible: bool,
    pub timeline_visible: bool,
    pub inspector_width: f32,
    /// Share of pane content height, excluding the native divider.
    pub timeline_fraction: f32,
    pub timeline_tools_visible: bool,
    pub timeline_track_controls_visible: bool,
    pub timeline_audio_monitor_visible: bool,
}

impl Default for EditorLayout {
    fn default() -> Self {
        Self {
            inspector_visible: true,
            timeline_visible: true,
            inspector_width: DEFAULT_INSPECTOR_WIDTH,
            timeline_fraction: 0.4,
            timeline_tools_visible: true,
            timeline_track_controls_visible: true,
            timeline_audio_monitor_visible: true,
        }
    }
}

impl EditorLayout {
    pub fn json(self) -> Value {
        json!({
            "inspector_visible": self.inspector_visible,
            "timeline_visible": self.timeline_visible,
            "inspector_width": self.inspector_width,
            "timeline_fraction": self.timeline_fraction,
            "timeline_tools_visible": self.timeline_tools_visible,
            "timeline_track_controls_visible": self.timeline_track_controls_visible,
            "timeline_audio_monitor_visible": self.timeline_audio_monitor_visible,
        })
    }

    fn apply(&mut self, patch: &Value) -> Result<(), String> {
        for (key, value) in patch
            .as_object()
            .ok_or("editor layout requires an object")?
        {
            let flag = match key.as_str() {
                "inspector_visible" => Some(&mut self.inspector_visible),
                "timeline_visible" => Some(&mut self.timeline_visible),
                "timeline_tools_visible" => Some(&mut self.timeline_tools_visible),
                "timeline_track_controls_visible" => {
                    Some(&mut self.timeline_track_controls_visible)
                }
                "timeline_audio_monitor_visible" => Some(&mut self.timeline_audio_monitor_visible),
                "inspector_width" | "timeline_fraction" => None,
                _ => return Err(format!("unknown editor layout preference: {key}")),
            };
            if let Some(flag) = flag {
                *flag = value
                    .as_bool()
                    .ok_or_else(|| format!("{key} requires a boolean"))?;
            } else {
                let number = value
                    .as_f64()
                    .ok_or_else(|| format!("{key} requires a number"))?
                    as f32;
                match key.as_str() {
                    "inspector_width" => self.inspector_width = number,
                    "timeline_fraction" => self.timeline_fraction = number,
                    _ => unreachable!(),
                }
            }
        }
        self.validate()
    }

    fn validate(&mut self) -> Result<(), String> {
        if !self.inspector_width.is_finite() || !self.timeline_fraction.is_finite() {
            return Err("editor layout dimensions must be finite".into());
        }
        self.inspector_width = self
            .inspector_width
            .clamp(DEFAULT_INSPECTOR_WIDTH, MAX_INSPECTOR_WIDTH);
        self.timeline_fraction = self
            .timeline_fraction
            .clamp(MIN_TIMELINE_FRACTION, MAX_TIMELINE_FRACTION);
        Ok(())
    }
}

pub(super) fn load(conn: Option<&Connection>) -> EditorLayout {
    let mut layout = EditorLayout::default();
    if let Some(conn) = conn {
        let saved = read_string_or_default(conn, KEY, "{}", |text| {
            serde_json::from_str::<Value>(text).is_ok_and(|value| {
                let mut layout = EditorLayout::default();
                layout.apply(&value).is_ok()
            })
        });
        layout
            .apply(&serde_json::from_str(&saved).expect("validated editor layout preference"))
            .expect("validated editor layout fields");
    }
    layout
}

/// Persist once at a user toggle or the end of native divider tracking.
pub fn update_editor_layout(
    store: &SharedPreferences,
    update: impl FnOnce(&mut EditorLayout),
) -> Result<EditorLayout, String> {
    let mut state = store.borrow_mut();
    let mut layout = state.editor_layout;
    update(&mut layout);
    layout.validate()?;
    if layout == state.editor_layout {
        return Ok(layout);
    }
    if let Some(conn) = &state.conn {
        write_string(conn, KEY, &layout.json().to_string()).map_err(|error| error.to_string())?;
    }
    state.editor_layout = layout;
    notify_listeners(state);
    Ok(layout)
}

/// Native remote adapters pass only the shared preference fields they changed.
pub fn set_editor_layout(store: &SharedPreferences, patch: &Value) -> Result<EditorLayout, String> {
    let mut layout = store.borrow().editor_layout;
    layout.apply(patch)?;
    update_editor_layout(store, |current| *current = layout)
}
