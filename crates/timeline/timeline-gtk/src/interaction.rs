use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, mpsc, mpsc::TryRecvError};
use std::thread;
use std::time::Duration;

use adw::prelude::*;
use gtk::glib;
use gtk::{gdk, gio};
use shrimply_math_core::Fraction;

use crate::desktop_open;
use crate::export;
use crate::player_state::{self, ProjectChange, SharedPlayerState};
use crate::preferences::store as preferences_store;
use crate::project::{
    AUDIO_TRACK_GAIN_MAX_DB, AUDIO_TRACK_GAIN_MIN_DB, CanvasSize, CaptionItem, CaptionTrack,
    Project, Time, TransitionSide, fraction_as_f64, playback_speed_or_default,
};
use crate::selection_state::{self, SharedSelectionState};
use crate::transcription::{SAMPLE_RATE, TranscribedSegment, prepare_transcription_chunks};

mod context_actions;
mod controllers;
mod cursor;
mod edit_actions;
mod keyboard;
mod media_import;
mod pointer;
mod transcription;

use context_actions::{
    add_menu_action, copy_selected_timeline_items, cut_selected_timeline_items,
    show_timeline_item_context_menu,
};
pub(super) use cursor::timeline_cursor;
pub(super) use media_import::{
    ask_remux_then_import_at, import_path_at, open_track_import_dialog, show_error_dialog,
};
pub(crate) use pointer::select_item_in_context;
pub(super) use pointer::{content_y, set_timeline_selection};
use pointer::{modifiers_from_state, push_modifiers, select_track};
use transcription::{
    add_caption_item_context_actions, selected_audio_project, show_transcribe_dialog,
};

use super::caption_tts;
use super::context_menu;
use super::items::{
    ItemKey, NewItemTarget, TimelineClipboard, TrackKind, delete_track_gap,
    expand_grouped_selection, fold_items, group_item_addresses, hit_item_at, item_group_id,
    paste_items, selected_item_addresses, ungroup_item_addresses,
};
use super::renderer::{Vec2, vec2};
use super::silence;
use super::timeline_operation::{SequenceTimeline, TimelineOperationContext};
use super::{
    RULER_HEIGHT, SCROLL_PIXELS_PER_STEP, TRACK_HEIGHT, TimelineCursor, TimelineModifiers,
    TimelineRuntime, TimelineScrollEvent, TrackAddMenuRequest, TrackKey, TrackLabelAction,
    WAVEFORM_POLL_INTERVAL, import, selected_timeline_items, selected_timeline_tracks, timeline_x,
    track_label_action_at, x_to_time,
};

pub(crate) use context_actions::folded_items::move_item_out_of_sequence_core;
pub(crate) use context_actions::folded_tracks::create_folded_track_core;
pub(crate) use context_actions::{create_track_core, item_file_path};
pub(super) use controllers::{add_input_controllers, start_timeline_animation_tick};
pub(super) use edit_actions::paste_timeline_clipboard;
use edit_actions::{
    append_selected_item_modifiers, delete_selected_addressed_items, delete_selected_gap,
    delete_selected_tracks, delete_tracks, fold_selected_timeline_items,
    group_selected_timeline_items, replace_selected_item_properties,
    ungroup_selected_timeline_items,
};
pub(crate) use edit_actions::{
    delete_selected_addressed_items_core, delete_selected_tracks_now_core,
    fold_selected_timeline_items_core, group_selected_timeline_items_core,
    paste_selected_item_properties_core, paste_timeline_clipboard_core, selected_track_clip_count,
    ungroup_selected_timeline_items_core,
};
