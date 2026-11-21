//! Synchronize native audio with the same transport events on every UI host.
use crate as player_state;
use shrimply_project_document::Project;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

pub fn connect_audio_playback(
    project: &Rc<RefCell<Project>>,
    player_state: &player_state::SharedPlayerState,
    audio_player: &Rc<shrimply_audio_engine::AudioPlayer>,
) {
    let project = Rc::downgrade(project);
    let media_player = Rc::downgrade(audio_player);
    let audio_alive = media_player.clone();
    let media_state = Rc::downgrade(player_state);
    let last_snapshot = player_state::snapshot(player_state);
    let last_position = Rc::new(Cell::new(last_snapshot.position));
    let last_playing = Rc::new(Cell::new(last_snapshot.playing));
    let last_playback_speed = Rc::new(Cell::new(last_snapshot.playback_speed));
    player_state::connect_while_alive_named(
        player_state,
        "editor audio sync",
        move || audio_alive.strong_count() > 0,
        move |event| {
            let (Some(project), Some(media_player)) = (project.upgrade(), media_player.upgrade())
            else {
                return;
            };
            let Some(media_state) = media_state.upgrade() else {
                return;
            };
            let snapshot = player_state::snapshot(&media_state);
            let position_changed = last_position.get() != snapshot.position;
            let playing_changed = last_playing.get() != snapshot.playing;
            let playback_speed_changed = last_playback_speed.get() != snapshot.playback_speed;
            let project_audio_changed = matches!(
                event,
                player_state::PlayerEvent::Project(change) if change.audio
            );
            let natural_playback = matches!(
                event,
                player_state::PlayerEvent::State(player_state::StateChange {
                    position: Some(player_state::PositionChange::Playback),
                    ..
                })
            );
            if project_audio_changed {
                media_player.set_project(&project.borrow());
            }
            if (position_changed && !natural_playback)
                || playing_changed
                || playback_speed_changed
                || project_audio_changed
            {
                if (position_changed && !natural_playback)
                    || (playing_changed && snapshot.playing)
                    || project_audio_changed
                {
                    media_player.seek(snapshot.position);
                }
                media_player.set_playback_speed(snapshot.playback_speed);
                media_player.set_playing(snapshot.playing);
                if position_changed
                    && !snapshot.playing
                    && matches!(
                        event,
                        player_state::PlayerEvent::State(player_state::StateChange {
                            position: Some(player_state::PositionChange::Seek),
                            ..
                        })
                    )
                {
                    media_player.preview_from(snapshot.position);
                }
            }
            if position_changed || playing_changed || playback_speed_changed {
                last_position.set(snapshot.position);
                last_playing.set(snapshot.playing);
                last_playback_speed.set(snapshot.playback_speed);
            }
        },
    );
}
