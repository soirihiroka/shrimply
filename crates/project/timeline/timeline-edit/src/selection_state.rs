use std::cell::RefCell;
use std::rc::Rc;

pub use crate::{
    ItemKey as SelectedItem, TrackGap as SelectedGap, TrackKey as SelectedTrack,
    TrackKind as SelectedItemKind,
};
use shrimply_project_document::project::{
    ItemAddress, Project, SequenceScopeId, Time, TrackAddress,
};

pub type SharedSelectionState = Rc<RefCell<SelectionState>>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrackAddressGap {
    pub track: TrackAddress,
    pub start: Time,
    pub end: Time,
}

#[derive(Clone)]
pub struct SelectionState {
    selected_item_addresses: Vec<ItemAddress>,
    focused_item_address: Option<ItemAddress>,
    // Compatibility for callers that still select root items by vector index.
    // New code must use the address APIs below.
    legacy_selected_items: Vec<SelectedItem>,
    legacy_focused_item: Option<SelectedItem>,
    active_scope: SequenceScopeId,
    focused_transition: Option<shrimply_project_document::project::TransitionSide>,
    selected_track_addresses: Vec<TrackAddress>,
    focused_track_address: Option<TrackAddress>,
    legacy_selected_tracks: Vec<SelectedTrack>,
    legacy_focused_track: Option<SelectedTrack>,
    selected_gap_address: Option<TrackAddressGap>,
    legacy_selected_gap: Option<SelectedGap>,
    listeners: Vec<SelectionListener>,
}

#[derive(Clone)]
struct SelectionListener {
    label: &'static str,
    callback: Rc<dyn Fn()>,
}

/// Selection data for cancelling an interaction; subscriptions remain live.
pub struct SelectionSnapshot(SelectionState);

pub fn snapshot(state: &SharedSelectionState) -> SelectionSnapshot {
    let mut snapshot = state.borrow().clone();
    snapshot.listeners.clear();
    SelectionSnapshot(snapshot)
}

pub fn restore(state: &SharedSelectionState, snapshot: SelectionSnapshot) {
    let listeners = {
        let mut state = state.borrow_mut();
        let listeners = std::mem::take(&mut state.listeners);
        *state = snapshot.0;
        state.listeners = listeners;
        state.listeners.clone()
    };
    notify_listeners(listeners);
}

pub fn new() -> SharedSelectionState {
    Rc::new(RefCell::new(SelectionState {
        selected_item_addresses: Vec::new(),
        focused_item_address: None,
        legacy_selected_items: Vec::new(),
        legacy_focused_item: None,
        active_scope: SequenceScopeId::root(),
        focused_transition: None,
        selected_track_addresses: Vec::new(),
        focused_track_address: None,
        legacy_selected_tracks: Vec::new(),
        legacy_focused_track: None,
        selected_gap_address: None,
        legacy_selected_gap: None,
        listeners: Vec::new(),
    }))
}

pub fn connect_named(
    state: &SharedSelectionState,
    label: &'static str,
    listener: impl Fn() + 'static,
) {
    state.borrow_mut().listeners.push(SelectionListener {
        label,
        callback: Rc::new(listener),
    });
}

pub fn focused_item(state: &SharedSelectionState) -> Option<SelectedItem> {
    state.borrow().legacy_focused_item
}

pub fn selected_items(state: &SharedSelectionState) -> Vec<SelectedItem> {
    state.borrow().legacy_selected_items.clone()
}

pub fn focused_nested_item(state: &SharedSelectionState) -> Option<ItemAddress> {
    state
        .borrow()
        .focused_item_address
        .clone()
        .filter(|item| !item.is_root())
}

pub fn selected_nested_items(state: &SharedSelectionState) -> Vec<ItemAddress> {
    state
        .borrow()
        .selected_item_addresses
        .iter()
        .filter(|item| !item.is_root())
        .cloned()
        .collect()
}

pub fn active_scope(state: &SharedSelectionState) -> SequenceScopeId {
    state.borrow().active_scope.clone()
}

pub fn set_active_scope(state: &SharedSelectionState, scope: SequenceScopeId) {
    let listeners = {
        let mut state = state.borrow_mut();
        if state.active_scope == scope {
            return;
        }
        state.active_scope = scope;
        state.listeners.clone()
    };
    notify_listeners(listeners);
}

pub fn selected_item_addresses(
    state: &SharedSelectionState,
    project: &Project,
) -> Vec<ItemAddress> {
    let state = state.borrow();
    if !state.selected_item_addresses.is_empty() {
        return state.selected_item_addresses.clone();
    }
    state
        .legacy_selected_items
        .iter()
        .filter_map(|key| item_address(project, *key))
        .collect()
}

pub fn focused_item_address(
    state: &SharedSelectionState,
    project: &Project,
) -> Option<ItemAddress> {
    let state = state.borrow();
    if let Some(item) = &state.focused_item_address {
        return Some(item.clone());
    }
    item_address(project, state.legacy_focused_item?)
}

pub fn focused_transition_address(
    state: &SharedSelectionState,
    project: &Project,
) -> Option<(
    ItemAddress,
    shrimply_project_document::project::TransitionSide,
)> {
    let state = state.borrow();
    let item = match &state.focused_item_address {
        Some(item) => item.clone(),
        None => item_address(project, state.legacy_focused_item?)?,
    };
    Some((item, state.focused_transition?))
}

pub fn set_focused_transition(
    state: &SharedSelectionState,
    side: shrimply_project_document::project::TransitionSide,
) {
    let listeners = {
        let mut state = state.borrow_mut();
        if (state.legacy_focused_item.is_none() && state.focused_item_address.is_none())
            || state.focused_transition == Some(side)
        {
            return;
        }
        state.focused_transition = Some(side);
        state.listeners.clone()
    };
    notify_listeners(listeners);
}

pub fn clear_focused_transition(state: &SharedSelectionState) {
    let listeners = {
        let mut state = state.borrow_mut();
        if state.focused_transition.take().is_none() {
            return;
        }
        state.listeners.clone()
    };
    notify_listeners(listeners);
}

pub fn focused_track(state: &SharedSelectionState) -> Option<SelectedTrack> {
    state.borrow().legacy_focused_track
}

pub fn selected_tracks(state: &SharedSelectionState) -> Vec<SelectedTrack> {
    state.borrow().legacy_selected_tracks.clone()
}

pub fn selected_gap(state: &SharedSelectionState) -> Option<SelectedGap> {
    state.borrow().legacy_selected_gap
}

pub fn focused_track_address(
    state: &SharedSelectionState,
    project: &Project,
) -> Option<TrackAddress> {
    let state = state.borrow();
    state
        .focused_track_address
        .clone()
        .or_else(|| track_address(project, state.legacy_focused_track?))
}

pub fn selected_track_addresses(
    state: &SharedSelectionState,
    project: &Project,
) -> Vec<TrackAddress> {
    let state = state.borrow();
    if !state.selected_track_addresses.is_empty() {
        return state.selected_track_addresses.clone();
    }
    state
        .legacy_selected_tracks
        .iter()
        .filter_map(|track| track_address(project, *track))
        .collect()
}

pub fn selected_gap_address(
    state: &SharedSelectionState,
    project: &Project,
) -> Option<TrackAddressGap> {
    let state = state.borrow();
    state.selected_gap_address.clone().or_else(|| {
        let gap = state.legacy_selected_gap?;
        Some(TrackAddressGap {
            track: track_address(project, gap.track)?,
            start: gap.start,
            end: gap.end,
        })
    })
}

pub fn set_selected_items(
    state: &SharedSelectionState,
    selected_items: Vec<SelectedItem>,
    focused_item: Option<SelectedItem>,
) {
    assert!(
        focused_item.is_none_or(|focused_item| selected_items.contains(&focused_item)),
        "focused item must be selected"
    );
    let listeners = {
        let mut state = state.borrow_mut();
        if state.legacy_selected_items == selected_items
            && state.legacy_focused_item == focused_item
            && state.focused_transition.is_none()
            && state.selected_item_addresses.is_empty()
            && state.focused_item_address.is_none()
            && state.selected_track_addresses.is_empty()
            && state.focused_track_address.is_none()
            && state.legacy_selected_tracks.is_empty()
            && state.legacy_focused_track.is_none()
            && state.selected_gap_address.is_none()
            && state.legacy_selected_gap.is_none()
        {
            return;
        }
        if !selected_items.is_empty() {
            state.active_scope = SequenceScopeId::root();
        }
        state.legacy_selected_items = selected_items;
        state.legacy_focused_item = focused_item;
        state.selected_item_addresses.clear();
        state.focused_item_address = None;
        state.focused_transition = None;
        state.selected_track_addresses.clear();
        state.focused_track_address = None;
        state.legacy_selected_tracks.clear();
        state.legacy_focused_track = None;
        state.selected_gap_address = None;
        state.legacy_selected_gap = None;
        state.listeners.clone()
    };

    notify_listeners(listeners);
}

pub fn set_selected_nested_items(
    state: &SharedSelectionState,
    selected_items: Vec<ItemAddress>,
    focused_item: Option<ItemAddress>,
) {
    assert!(
        selected_items.iter().all(|item| !item.is_root()),
        "nested selection cannot contain root items"
    );
    assert!(
        focused_item
            .as_ref()
            .is_none_or(|focused| selected_items.contains(focused)),
        "focused nested item must be selected"
    );
    let listeners = {
        let mut state = state.borrow_mut();
        if state.selected_item_addresses == selected_items
            && state.focused_item_address == focused_item
            && state.legacy_selected_items.is_empty()
            && state.legacy_focused_item.is_none()
            && state.selected_track_addresses.is_empty()
            && state.focused_track_address.is_none()
            && state.legacy_selected_tracks.is_empty()
            && state.legacy_focused_track.is_none()
            && state.selected_gap_address.is_none()
            && state.legacy_selected_gap.is_none()
        {
            return;
        }
        state.legacy_selected_items.clear();
        state.legacy_focused_item = None;
        state.selected_item_addresses = selected_items;
        state.focused_item_address = focused_item;
        state.focused_transition = None;
        state.selected_track_addresses.clear();
        state.focused_track_address = None;
        state.legacy_selected_tracks.clear();
        state.legacy_focused_track = None;
        state.selected_gap_address = None;
        state.legacy_selected_gap = None;
        state.listeners.clone()
    };
    notify_listeners(listeners);
}

pub fn set_selected_item_addresses(
    state: &SharedSelectionState,
    project: &Project,
    selected_items: Vec<ItemAddress>,
    focused_item: Option<ItemAddress>,
) {
    assert!(
        focused_item
            .as_ref()
            .is_none_or(|focused| selected_items.contains(focused)),
        "focused item must be selected"
    );
    let scopes = selected_items
        .iter()
        .map(|item| {
            assert!(project.item(item).is_some(), "selected item must exist");
            project
                .item_scope(item)
                .expect("selected item must have a valid sequence scope")
        })
        .collect::<Vec<_>>();
    assert!(
        scopes
            .first()
            .is_none_or(|first| scopes.iter().all(|scope| scope == first)),
        "item selection cannot span sequence scopes"
    );

    let legacy_selected_items = selected_items
        .iter()
        .map(|item| item_key(project, item))
        .collect::<Option<Vec<_>>>()
        .unwrap_or_default();
    let legacy_focused_item = focused_item
        .as_ref()
        .and_then(|item| item_key(project, item));
    let listeners = {
        let mut state = state.borrow_mut();
        if state.selected_item_addresses == selected_items
            && state.focused_item_address == focused_item
            && state.legacy_selected_items == legacy_selected_items
            && state.legacy_focused_item == legacy_focused_item
            && state.focused_transition.is_none()
            && state.selected_track_addresses.is_empty()
            && state.focused_track_address.is_none()
            && state.legacy_selected_tracks.is_empty()
            && state.legacy_focused_track.is_none()
            && state.selected_gap_address.is_none()
            && state.legacy_selected_gap.is_none()
        {
            return;
        }
        if let Some(scope) = scopes.into_iter().next() {
            state.active_scope = scope;
        }
        state.selected_item_addresses = selected_items;
        state.focused_item_address = focused_item;
        state.legacy_selected_items = legacy_selected_items;
        state.legacy_focused_item = legacy_focused_item;
        state.focused_transition = None;
        state.selected_track_addresses.clear();
        state.focused_track_address = None;
        state.legacy_selected_tracks.clear();
        state.legacy_focused_track = None;
        state.selected_gap_address = None;
        state.legacy_selected_gap = None;
        state.listeners.clone()
    };
    notify_listeners(listeners);
}

pub fn set_selected_tracks(
    state: &SharedSelectionState,
    selected_tracks: Vec<SelectedTrack>,
    focused_track: Option<SelectedTrack>,
) {
    assert!(
        focused_track.is_none_or(|focused_track| selected_tracks.contains(&focused_track)),
        "focused track must be selected"
    );
    let listeners = {
        let mut state = state.borrow_mut();
        if state.legacy_selected_tracks == selected_tracks
            && state.legacy_focused_track == focused_track
            && state.selected_track_addresses.is_empty()
            && state.focused_track_address.is_none()
            && state.selected_item_addresses.is_empty()
            && state.focused_item_address.is_none()
            && state.legacy_selected_items.is_empty()
            && state.legacy_focused_item.is_none()
            && state.selected_gap_address.is_none()
            && state.legacy_selected_gap.is_none()
        {
            return;
        }
        if !selected_tracks.is_empty() {
            state.active_scope = SequenceScopeId::root();
        }
        state.selected_item_addresses.clear();
        state.focused_item_address = None;
        state.legacy_selected_items.clear();
        state.legacy_focused_item = None;
        state.focused_transition = None;
        state.selected_track_addresses.clear();
        state.focused_track_address = None;
        state.legacy_selected_tracks = selected_tracks;
        state.legacy_focused_track = focused_track;
        state.selected_gap_address = None;
        state.legacy_selected_gap = None;
        state.listeners.clone()
    };

    notify_listeners(listeners);
}

pub fn set_selected_track_addresses(
    state: &SharedSelectionState,
    project: &Project,
    selected_tracks: Vec<TrackAddress>,
    focused_track: Option<TrackAddress>,
) {
    assert!(
        focused_track
            .as_ref()
            .is_none_or(|focused| selected_tracks.contains(focused)),
        "focused track must be selected"
    );
    let scopes = selected_tracks
        .iter()
        .map(|track| {
            assert!(project.track(track).is_some(), "selected track must exist");
            project
                .track_scope(track)
                .expect("selected track must have a valid sequence scope")
        })
        .collect::<Vec<_>>();
    assert!(
        scopes
            .first()
            .is_none_or(|first| scopes.iter().all(|scope| scope == first)),
        "track selection cannot span sequence scopes"
    );
    let legacy_selected_tracks = selected_tracks
        .iter()
        .map(|track| track_key(project, track))
        .collect::<Option<Vec<_>>>()
        .unwrap_or_default();
    let legacy_focused_track = focused_track
        .as_ref()
        .and_then(|track| track_key(project, track));
    let listeners = {
        let mut state = state.borrow_mut();
        if state.selected_track_addresses == selected_tracks
            && state.focused_track_address == focused_track
            && state.legacy_selected_tracks == legacy_selected_tracks
            && state.legacy_focused_track == legacy_focused_track
            && state.selected_item_addresses.is_empty()
            && state.focused_item_address.is_none()
            && state.legacy_selected_items.is_empty()
            && state.legacy_focused_item.is_none()
            && state.selected_gap_address.is_none()
            && state.legacy_selected_gap.is_none()
        {
            return;
        }
        if let Some(scope) = scopes.into_iter().next() {
            state.active_scope = scope;
        }
        state.selected_item_addresses.clear();
        state.focused_item_address = None;
        state.legacy_selected_items.clear();
        state.legacy_focused_item = None;
        state.focused_transition = None;
        state.selected_track_addresses = selected_tracks;
        state.focused_track_address = focused_track;
        state.legacy_selected_tracks = legacy_selected_tracks;
        state.legacy_focused_track = legacy_focused_track;
        state.selected_gap_address = None;
        state.legacy_selected_gap = None;
        state.listeners.clone()
    };
    notify_listeners(listeners);
}

pub fn set_selected_gap(state: &SharedSelectionState, selected_gap: Option<SelectedGap>) {
    assert!(
        selected_gap.is_none_or(|gap| gap.start < gap.end),
        "selected gap must have positive duration"
    );
    let listeners = {
        let mut state = state.borrow_mut();
        if state.legacy_selected_gap == selected_gap
            && state.selected_gap_address.is_none()
            && state.selected_item_addresses.is_empty()
            && state.focused_item_address.is_none()
            && state.legacy_selected_items.is_empty()
            && state.legacy_focused_item.is_none()
            && state.focused_transition.is_none()
            && state.selected_track_addresses.is_empty()
            && state.focused_track_address.is_none()
            && state.legacy_selected_tracks.is_empty()
            && state.legacy_focused_track.is_none()
        {
            return;
        }
        if selected_gap.is_some() {
            state.active_scope = SequenceScopeId::root();
        }
        state.selected_item_addresses.clear();
        state.focused_item_address = None;
        state.legacy_selected_items.clear();
        state.legacy_focused_item = None;
        state.focused_transition = None;
        state.selected_track_addresses.clear();
        state.focused_track_address = None;
        state.legacy_selected_tracks.clear();
        state.legacy_focused_track = None;
        state.selected_gap_address = None;
        state.legacy_selected_gap = selected_gap;
        state.listeners.clone()
    };

    notify_listeners(listeners);
}

pub fn set_selected_gap_address(
    state: &SharedSelectionState,
    project: &Project,
    selected_gap: Option<TrackAddressGap>,
) {
    assert!(
        selected_gap.as_ref().is_none_or(|gap| gap.start < gap.end),
        "selected gap must have positive duration"
    );
    let scope = selected_gap.as_ref().map(|gap| {
        assert!(project.track(&gap.track).is_some(), "gap track must exist");
        project
            .track_scope(&gap.track)
            .expect("gap track must have a valid sequence scope")
    });
    let legacy_selected_gap = selected_gap.as_ref().and_then(|gap| {
        Some(SelectedGap {
            track: track_key(project, &gap.track)?,
            start: gap.start,
            end: gap.end,
        })
    });
    let listeners = {
        let mut state = state.borrow_mut();
        if state.selected_gap_address == selected_gap
            && state.legacy_selected_gap == legacy_selected_gap
            && state.selected_item_addresses.is_empty()
            && state.focused_item_address.is_none()
            && state.legacy_selected_items.is_empty()
            && state.legacy_focused_item.is_none()
            && state.focused_transition.is_none()
            && state.selected_track_addresses.is_empty()
            && state.focused_track_address.is_none()
            && state.legacy_selected_tracks.is_empty()
            && state.legacy_focused_track.is_none()
        {
            return;
        }
        if let Some(scope) = scope {
            state.active_scope = scope;
        }
        state.selected_item_addresses.clear();
        state.focused_item_address = None;
        state.legacy_selected_items.clear();
        state.legacy_focused_item = None;
        state.focused_transition = None;
        state.selected_track_addresses.clear();
        state.focused_track_address = None;
        state.legacy_selected_tracks.clear();
        state.legacy_focused_track = None;
        state.selected_gap_address = selected_gap;
        state.legacy_selected_gap = legacy_selected_gap;
        state.listeners.clone()
    };
    notify_listeners(listeners);
}

fn notify_listeners(listeners: Vec<SelectionListener>) {
    for listener in listeners {
        shrimply_process_reporting::crash::set_context(format!(
            "selection listener begin {}",
            listener.label
        ));
        (listener.callback)();
        shrimply_process_reporting::crash::set_context(format!(
            "selection listener end {}",
            listener.label
        ));
    }
}

pub fn focused_caption(state: &SharedSelectionState) -> Option<SelectedItem> {
    focused_kind(state, SelectedItemKind::Caption)
}

pub fn focused_video(state: &SharedSelectionState) -> Option<SelectedItem> {
    focused_kind(state, SelectedItemKind::Video)
}

pub fn focused_video_address(
    state: &SharedSelectionState,
    project: &Project,
) -> Option<ItemAddress> {
    let address = focused_item_address(state, project)?;
    matches!(address, ItemAddress::Video { .. }).then_some(address)
}

pub fn focused_audio(state: &SharedSelectionState) -> Option<SelectedItem> {
    focused_kind(state, SelectedItemKind::Audio)
}

fn focused_kind(state: &SharedSelectionState, kind: SelectedItemKind) -> Option<SelectedItem> {
    let item = state.borrow().legacy_focused_item?;
    (item.kind == kind).then_some(item)
}

pub fn item_address(project: &Project, key: SelectedItem) -> Option<ItemAddress> {
    match key.kind {
        SelectedItemKind::Comment => {
            let track = project.comment_tracks.get(key.track_index)?;
            Some(ItemAddress::Comment {
                track_id: track.id,
                item_id: track.items.get(key.item_index)?.id,
            })
        }
        SelectedItemKind::Caption => {
            let track = project.caption_tracks.get(key.track_index)?;
            Some(ItemAddress::Caption {
                track_id: track.id,
                item_id: track.items.get(key.item_index)?.id,
            })
        }
        SelectedItemKind::Video => {
            let track = project.video_tracks.get(key.track_index)?;
            Some(ItemAddress::Video {
                sequence_path: Vec::new(),
                track_id: track.id,
                item_id: track.items.get(key.item_index)?.id,
            })
        }
        SelectedItemKind::Audio => {
            let track = project.audio_tracks.get(key.track_index)?;
            Some(ItemAddress::Audio {
                sequence_path: Vec::new(),
                track_id: track.id,
                item_id: track.items.get(key.item_index)?.id,
            })
        }
    }
}

pub fn item_key(project: &Project, address: &ItemAddress) -> Option<SelectedItem> {
    if !address.is_root() {
        return None;
    }
    let (kind, track_index, item_index) = match address {
        ItemAddress::Comment { track_id, item_id } => {
            let track_index = project
                .comment_tracks
                .iter()
                .position(|track| track.id == *track_id)?;
            let item_index = project.comment_tracks[track_index]
                .items
                .iter()
                .position(|item| item.id == *item_id)?;
            (SelectedItemKind::Comment, track_index, item_index)
        }
        ItemAddress::Caption { track_id, item_id } => {
            let track_index = project
                .caption_tracks
                .iter()
                .position(|track| track.id == *track_id)?;
            let item_index = project.caption_tracks[track_index]
                .items
                .iter()
                .position(|item| item.id == *item_id)?;
            (SelectedItemKind::Caption, track_index, item_index)
        }
        ItemAddress::Video {
            track_id, item_id, ..
        } => {
            let track_index = project
                .video_tracks
                .iter()
                .position(|track| track.id == *track_id)?;
            let item_index = project.video_tracks[track_index]
                .items
                .iter()
                .position(|item| item.id == *item_id)?;
            (SelectedItemKind::Video, track_index, item_index)
        }
        ItemAddress::Audio {
            track_id, item_id, ..
        } => {
            let track_index = project
                .audio_tracks
                .iter()
                .position(|track| track.id == *track_id)?;
            let item_index = project.audio_tracks[track_index]
                .items
                .iter()
                .position(|item| item.id == *item_id)?;
            (SelectedItemKind::Audio, track_index, item_index)
        }
    };
    Some(SelectedItem {
        kind,
        track_index,
        item_index,
    })
}

pub fn track_address(project: &Project, key: SelectedTrack) -> Option<TrackAddress> {
    match key.kind {
        SelectedItemKind::Comment => Some(TrackAddress::Comment {
            track_id: project.comment_tracks.get(key.track_index)?.id,
        }),
        SelectedItemKind::Caption => Some(TrackAddress::Caption {
            track_id: project.caption_tracks.get(key.track_index)?.id,
        }),
        SelectedItemKind::Video => Some(TrackAddress::Video {
            sequence_path: Vec::new(),
            track_id: project.video_tracks.get(key.track_index)?.id,
        }),
        SelectedItemKind::Audio => Some(TrackAddress::Audio {
            sequence_path: Vec::new(),
            track_id: project.audio_tracks.get(key.track_index)?.id,
        }),
    }
}

pub fn track_key(project: &Project, address: &TrackAddress) -> Option<SelectedTrack> {
    if !address.is_root() {
        return None;
    }
    let kind = address.kind();
    let track_index = project.root_track_index(kind, address.track_id())?;
    Some(SelectedTrack { kind, track_index })
}
