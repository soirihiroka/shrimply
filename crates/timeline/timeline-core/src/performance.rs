use shrimply_playback_performance as playback_performance;
use shrimply_project::project::Project;
use shrimply_state::player_state::{self, SharedPlayerState};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
};

pub(super) struct State {
    collector: playback_performance::SharedCollector,
    updates: async_channel::Receiver<()>,
    _lifetime: Rc<()>,
    player: std::rc::Weak<RefCell<player_state::PlayerState>>,
}

impl State {
    pub(super) fn new(
        project: Rc<RefCell<Project>>,
        player: SharedPlayerState,
        collector: playback_performance::SharedCollector,
    ) -> Self {
        let initial = player_state::snapshot(&player);
        let last_position = Rc::new(Cell::new(initial.position));
        let last_playing = Rc::new(Cell::new(initial.playing));
        let lifetime = Rc::new(());
        let alive = Rc::downgrade(&lifetime);
        let weak_player = Rc::downgrade(&player);
        let listener_player = weak_player.clone();
        let listener_collector = collector.clone();
        player_state::connect_while_alive_named(
            &player,
            "timeline performance",
            move || alive.strong_count() > 0,
            move |event| {
                let Some(player) = listener_player.upgrade() else {
                    return;
                };
                let snapshot = player_state::snapshot(&player);
                let position_changed = last_position.get() != snapshot.position;
                let playing_changed = last_playing.get() != snapshot.playing;
                let natural_playback = matches!(
                    event,
                    player_state::PlayerEvent::State(player_state::StateChange {
                        position: Some(player_state::PositionChange::Playback),
                        ..
                    })
                );
                if playing_changed {
                    if snapshot.playing {
                        playback_performance::begin_playback(
                            &listener_collector,
                            snapshot.position,
                        );
                    } else {
                        playback_performance::end_playback(&listener_collector, snapshot.position);
                    }
                } else if position_changed && snapshot.playing && !natural_playback {
                    playback_performance::seek_playback(&listener_collector, snapshot.position);
                }
                if matches!(event, player_state::PlayerEvent::Project(change) if change.video) {
                    playback_performance::set_project(
                        &listener_collector,
                        Arc::new(project.borrow().clone()),
                    );
                }
                last_position.set(snapshot.position);
                last_playing.set(snapshot.playing);
            },
        );
        if initial.playing {
            playback_performance::begin_playback(&collector, initial.position);
        }
        Self {
            updates: playback_performance::subscribe(&collector),
            collector,
            _lifetime: lifetime,
            player: weak_player,
        }
    }

    pub(super) fn snapshot(&self) -> Arc<playback_performance::Snapshot> {
        playback_performance::snapshot(&self.collector)
    }

    pub(super) fn updated(&self) -> bool {
        let mut updated = false;
        while self.updates.try_recv().is_ok() {
            updated = true;
        }
        updated
    }
}

impl Drop for State {
    fn drop(&mut self) {
        if let Some(player) = self.player.upgrade() {
            let snapshot = player_state::snapshot(&player);
            if snapshot.playing {
                playback_performance::end_playback(&self.collector, snapshot.position);
            }
        }
    }
}
