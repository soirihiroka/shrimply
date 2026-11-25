use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

use shrimply_editing_api::protocol::{Presence, ScopeRef};
use uuid::Uuid;

use super::{Project, SharedPlayerState, SharedSelectionState, player_state, selection_state};

const PEER_TTL: Duration = Duration::from_secs(3);

pub(super) fn local_name() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .ok()
        .filter(|name| !name.trim().is_empty() && name.len() <= 128)
        .unwrap_or_else(|| "Shrimply".into())
}

pub(super) fn capture(
    id: Uuid,
    name: &str,
    player: &SharedPlayerState,
    selection: &SharedSelectionState,
    project: &Project,
) -> Presence {
    let player = player_state::snapshot(player);
    Presence {
        id,
        name: name.to_owned(),
        position: player.position,
        playing: player.playing,
        scope: ScopeRef {
            sequence_path: selection_state::active_scope(selection)
                .instance_ids()
                .to_vec(),
        },
        clips: selection_state::selected_item_addresses(selection, project),
        tracks: selection_state::selected_track_addresses(selection, project),
    }
}

#[derive(Default)]
pub(super) struct Peers(BTreeMap<Uuid, (Instant, Presence)>);

impl Peers {
    pub(super) fn update(&mut self, presence: Presence) {
        self.0.insert(presence.id, (Instant::now(), presence));
    }

    pub(super) fn active(&mut self) -> Vec<Presence> {
        let now = Instant::now();
        self.0.retain(|_, (observed, _)| now - *observed < PEER_TTL);
        self.0
            .values()
            .map(|(_, presence)| presence.clone())
            .collect()
    }
}
