use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, mpsc::TryRecvError};
use std::thread;
use std::time::Duration;

use adw::prelude::*;
use gtk::glib;
use gtk::{gdk, gio};

use crate::desktop_open;
use crate::export;
use crate::player_state::SharedPlayerState;
use crate::preferences::store as preferences_store;
use crate::project::{Project, Time};
use crate::selection_state::SharedSelectionState;

mod context_actions;
mod controllers;
mod cursor;
mod keyboard;
mod media_import;
mod pointer;
mod transcription;

use context_actions::{add_menu_action, show_timeline_item_context_menu};
pub(super) use cursor::timeline_cursor;
pub(super) use media_import::{open_track_import_dialog, show_error_dialog};
use pointer::{modifiers_from_state, push_modifiers};
use transcription::{
    add_caption_item_context_actions, selected_audio_project, show_transcribe_dialog,
};

use super::caption_tts;
use super::context_menu;
use super::renderer::{Vec2, vec2};
use super::silence;
use super::{
    SCROLL_PIXELS_PER_STEP, TimelineCursor, TimelineModifiers, TimelineRuntime,
    TimelineScrollEvent, TimelineScrollInput, TrackKey, WAVEFORM_POLL_INTERVAL,
    selected_timeline_items, selected_timeline_tracks,
};

pub(super) use controllers::{add_input_controllers, start_timeline_animation_tick};
