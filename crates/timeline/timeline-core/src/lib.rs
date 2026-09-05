use shrimply_state::preferences::{self, PreferencesSnapshot, SharedPreferences};

mod menu;
pub use menu::*;
pub mod audio_selection;
pub mod import;
mod track_add;
pub mod video_selection;
pub use track_add::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrackAddAction {
    Import,
    Text,
    Shape,
    Paint,
    Background,
    Scene3d,
    VideoGeneration,
    TextToSpeech,
    AudioGenerator,
}

impl TrackAddAction {
    pub const fn label(self, kind: shrimply_timeline::TrackKind) -> &'static str {
        match self {
            Self::Import if matches!(kind, shrimply_timeline::TrackKind::Caption) => {
                "Import Captions…"
            }
            Self::Import => "Import Media…",
            Self::Text => "Text",
            Self::Shape => "Shape",
            Self::Paint => "Paint",
            Self::Background => "Background",
            Self::Scene3d => "3D Scene",
            Self::VideoGeneration => "Video Generation",
            Self::TextToSpeech => "Text to Speech",
            Self::AudioGenerator => "Audio Generator",
        }
    }

    pub const fn icon(self) -> &'static str {
        match self {
            Self::Import => "document-open-symbolic",
            Self::Text => "draw-text-symbolic",
            Self::Shape => "shapes-large-symbolic",
            Self::Paint => "applications-graphics-symbolic",
            Self::Background => "preferences-desktop-wallpaper-symbolic",
            Self::Scene3d => "3d-object-symbolic",
            Self::VideoGeneration => "video-generation-symbolic",
            Self::TextToSpeech => "font-x-generic-symbolic",
            Self::AudioGenerator => "sound-symbolic",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrackAddMenuEntry {
    Action(TrackAddAction),
    Separator,
}

pub const fn track_add_menu(kind: shrimply_timeline::TrackKind) -> &'static [TrackAddMenuEntry] {
    use TrackAddAction::*;
    use TrackAddMenuEntry::*;
    match kind {
        shrimply_timeline::TrackKind::Caption => &[Action(Import)],
        shrimply_timeline::TrackKind::Video => &[
            Action(Import),
            Separator,
            Action(Text),
            Action(Shape),
            Action(Paint),
            Action(Background),
            Action(Scene3d),
            Action(VideoGeneration),
        ],
        shrimply_timeline::TrackKind::Audio => &[
            Action(Import),
            Separator,
            Action(TextToSpeech),
            Action(AudioGenerator),
        ],
    }
}

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

use hashbrown::HashMap;
pub use shrimply_audio as audio;
pub use shrimply_math_color::Color;
pub use shrimply_math_core::Fraction;
pub use shrimply_project::{caption, project, time_format, timeline_search};
pub use shrimply_skia_adw_core::{canvas as renderer, cursor, skia_font, skia_system_font};
pub use shrimply_timeline::{edit, selection_state};
use std::time::Instant;
pub mod math;
pub mod scene;
pub mod selection;
use audio::beat::BeatMap;
use audio::waveform::{self, WaveformMap};
use project::{
    AudioItem, CaptionItem, Project, RepeatStrategy, Time, TransitionSide, VideoItemContent,
    generated_item_keyframe_span, generated_item_natural_end_position, generated_item_natural_span,
    media_item_natural_end_position, media_natural_end_interval, media_real_span,
    scaled_time_delta, video_natural_end_interval,
};
use renderer::{Align2, FontId, Rect, Stroke, StrokeKind, TimelinePainter, Vec2, vec2};
pub use shrimply_cross_ui_theme as theme;
use shrimply_playback_performance as playback_performance;
use shrimply_timeline::TrackGap;
pub use shrimply_timeline::TrackKey;
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
pub mod ruler;
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
