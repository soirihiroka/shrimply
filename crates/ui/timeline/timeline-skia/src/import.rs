use shrimply_math_core::Time;
pub use shrimply_media_import::*;
use shrimply_player_core::{self as player_state, ProjectChange, SharedPlayerState};
use shrimply_timeline_edit::selection_state::{self, SharedSelectionState};

pub fn finish_track_import(
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    result: Result<(ImportResult, Time), String>,
) -> Result<(), String> {
    let (result, duration) = result?;
    let focused_item = result.selection.first().copied();
    selection_state::set_selected_items(selection_state, result.selection, focused_item);
    player_state::refresh_project(
        player_state,
        ProjectChange {
            duration: Some(duration),
            frame_rate: None,
            audio: result.audio,
            audio_beats: result.audio,
            audio_waveforms: result.audio,
            video: result.video,
            live_preview: false,
            captions: result.captions,
            inspector: true,
        },
    );
    Ok(())
}

pub use crate::import_placement::*;
