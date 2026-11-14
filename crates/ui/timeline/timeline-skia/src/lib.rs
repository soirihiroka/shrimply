use shrimply_preferences_core::{self as preferences, PreferencesSnapshot, SharedPreferences};

mod menu;
pub use menu::*;
pub mod audio_selection;
pub mod caption_speech;
pub mod import;
pub mod recording;
pub mod silence;
mod track_add;
pub mod transcription;
pub mod video_selection;
pub use track_add::*;

pub use shrimply_timeline_edit::{TrackAddAction, TrackAddMenuEntry, track_add_menu};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum CursorTool {
    #[default]
    Pointer,
    Cut,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum DragCollisionMode {
    #[default]
    Overwrite,
    Block,
    NewTrack,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ToolState {
    pub magnet: bool,
    pub beat_grid: bool,
    pub cursor: CursorTool,
    pub drag_collision: DragCollisionMode,
}

impl ToolState {
    pub fn from_preferences(preferences: &PreferencesSnapshot) -> Self {
        Self {
            magnet: preferences.timeline_magnet == "true",
            beat_grid: preferences.timeline_beat_grid == "true",
            cursor: if preferences.timeline_cursor == "cut" {
                CursorTool::Cut
            } else {
                CursorTool::Pointer
            },
            drag_collision: match preferences.timeline_drag_collision_mode.as_str() {
                "block" => DragCollisionMode::Block,
                "new_track" => DragCollisionMode::NewTrack,
                _ => DragCollisionMode::Overwrite,
            },
        }
    }
}

#[derive(Clone)]
pub struct TimelineTools {
    preferences: SharedPreferences,
}

impl TimelineTools {
    pub fn new(preferences: SharedPreferences) -> Self {
        Self { preferences }
    }

    pub fn state(&self) -> ToolState {
        ToolState::from_preferences(&preferences::snapshot(&self.preferences))
    }

    pub fn set_magnet(&self, enabled: bool) {
        preferences::set_timeline_magnet(&self.preferences, enabled);
    }

    pub fn set_beat_grid(&self, enabled: bool) {
        preferences::set_timeline_beat_grid(&self.preferences, enabled);
    }

    pub fn set_cursor(&self, cursor: CursorTool) {
        preferences::set_timeline_cursor(
            &self.preferences,
            match cursor {
                CursorTool::Pointer => "pointer",
                CursorTool::Cut => "cut",
            },
        );
    }

    pub fn set_drag_collision(&self, mode: DragCollisionMode) {
        preferences::set_timeline_drag_collision_mode(
            &self.preferences,
            match mode {
                DragCollisionMode::Overwrite => "overwrite",
                DragCollisionMode::Block => "block",
                DragCollisionMode::NewTrack => "new_track",
            },
        );
    }
}

/// Ordered GTK sidebar tools, shared by native shells.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimelineTool {
    Magnet,
    BeatGrid,
    Pointer,
    Cut,
    Overwrite,
    Block,
    NewTrack,
}

pub struct ToolDescription {
    pub id: &'static str,
    pub icon: &'static str,
    pub label: &'static str,
    pub section: u8,
}

impl TimelineTool {
    pub const ALL: [Self; 7] = [
        Self::Magnet,
        Self::BeatGrid,
        Self::Pointer,
        Self::Cut,
        Self::Overwrite,
        Self::Block,
        Self::NewTrack,
    ];

    pub fn description(self) -> ToolDescription {
        let (id, icon, label, section) = match self {
            Self::Magnet => ("magnet", "magnet-tilted-symbolic", "Magnet", 0),
            Self::BeatGrid => ("beat-grid", "metronome-symbolic", "Beat Grid", 0),
            Self::Pointer => ("pointer", "pointer-left-symbolic", "Pointer", 1),
            Self::Cut => ("cut", "cut-symbolic", "Cut", 1),
            Self::Overwrite => ("overwrite", "track-insert-symbolic", "Overwrite/Insert", 2),
            Self::Block => ("block", "track-block-symbolic", "Block", 2),
            Self::NewTrack => ("new-track", "track-move-above-symbolic", "New Track", 2),
        };
        ToolDescription {
            id,
            icon,
            label,
            section,
        }
    }

    pub fn active(self, state: ToolState) -> bool {
        match self {
            Self::Magnet => state.magnet,
            Self::BeatGrid => state.beat_grid,
            Self::Pointer => state.cursor == CursorTool::Pointer,
            Self::Cut => state.cursor == CursorTool::Cut,
            Self::Overwrite => state.drag_collision == DragCollisionMode::Overwrite,
            Self::Block => state.drag_collision == DragCollisionMode::Block,
            Self::NewTrack => state.drag_collision == DragCollisionMode::NewTrack,
        }
    }

    pub fn activate(self, tools: &TimelineTools) {
        match self {
            Self::Magnet => tools.set_magnet(!tools.state().magnet),
            Self::BeatGrid => tools.set_beat_grid(!tools.state().beat_grid),
            Self::Pointer => tools.set_cursor(CursorTool::Pointer),
            Self::Cut => tools.set_cursor(CursorTool::Cut),
            Self::Overwrite => tools.set_drag_collision(DragCollisionMode::Overwrite),
            Self::Block => tools.set_drag_collision(DragCollisionMode::Block),
            Self::NewTrack => tools.set_drag_collision(DragCollisionMode::NewTrack),
        }
    }
}

use hashbrown::HashMap;
pub use shrimply_audio_engine as audio;
pub use shrimply_components_skia::{canvas as renderer, cursor, skia_font, skia_system_font};
pub use shrimply_math_color::Color;
pub use shrimply_math_core::Fraction;
pub use shrimply_project_document::{caption, project, time_format, timeline_search};
pub use shrimply_timeline_edit::{edit, selection_state};
use std::time::Instant;
pub mod math;
pub mod scene;
pub mod selection;
use audio::beat::BeatMap;
use audio::waveform::{self, WaveformMap};
use project::{
    AudioItem, CaptionItem, CommentItem, Project, RepeatStrategy, Time, TransitionSide,
    VideoItemContent, media_item_natural_end_position, media_natural_end_interval, media_real_span,
    scaled_time_delta, video_natural_end_interval,
};
use renderer::{FontId, Rect, Stroke, StrokeKind, TimelinePainter, Vec2, vec2};
pub use shrimply_cross_ui_theme as theme;
use shrimply_playback_performance as playback_performance;
use shrimply_timeline_edit::TrackGap;
pub use shrimply_timeline_edit::TrackKey;
pub mod beat_grid;
pub mod dragging;
pub mod draw_state;
pub mod drawing;
pub mod drop_area;
pub mod external_content;
pub mod folded_sequence;
pub mod geometry;
mod import_placement;
pub mod items;
pub mod metrics;
mod performance;
pub use shrimply_timeline_paint_skia::ruler;
pub mod timeline_operation;
pub mod track_controls;
pub mod view;
use draw_state::*;
use drawing::{row_screen_y, row_y};
use geometry::*;
use items::{
    DragIndicator, DragPreviewStatus, DraggedGroup, ItemKey, ResizeDrag, TrackKind,
    fitted_transition_durations, is_item_dragged, is_item_selected, resize_item_times,
    row_for_track, target_item_times, target_track_index, transition_durations,
};
use metrics::*;
use track_controls::*;
use view::*;

pub mod import_queue;

pub mod transitions;

pub mod caption_creation;
pub mod cutting;
pub mod snapping;
