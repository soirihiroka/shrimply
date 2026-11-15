pub const CONTROL_ROW_LABEL_COLUMNS: usize = 13;
pub const CONTROL_ROW_GAP: i32 = 6;

/// Shared panel timing; GTK uses AdwTimedAnimation with its ease-out-cubic default.
pub const PANEL_ANIMATION_DURATION_MS: u32 = 250;

const COMPACT_CONTROLS_WIDTH: f32 = 600.0;
const COMPACT_CONTROLS_HEIGHT: f32 = 480.0;
pub const MIN_PREVIEW_WIDTH: f32 = 360.0;
const INSPECTOR_DIVIDER_WIDTH: f32 = 12.0;
pub const TRANSPORT_ROW_MIN_WIDTH: f32 = 520.0;

pub fn inspector_min_width(inspector_width: f32) -> f32 {
    inspector_width + MIN_PREVIEW_WIDTH + INSPECTOR_DIVIDER_WIDTH
}

pub fn inspector_overlay(width: f32, inspector_width: f32) -> bool {
    width < inspector_min_width(inspector_width)
}

pub fn transport_stacked(width: f32) -> bool {
    width < TRANSPORT_ROW_MIN_WIDTH
}

pub fn compact_controls(width: f32, height: f32) -> bool {
    width < COMPACT_CONTROLS_WIDTH || height < COMPACT_CONTROLS_HEIGHT
}

pub fn menu_uses_bottom_sheet(input_source: &str) -> bool {
    match input_source {
        "touch" | "pen" => true,
        "pointer" | "keyboard" => false,
        _ => panic!("unknown menu input source: {input_source}"),
    }
}
