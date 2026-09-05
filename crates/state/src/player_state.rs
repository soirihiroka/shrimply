use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use shrimply_math_core::{Fraction, time_from_frame};
use shrimply_project::project::{Time, default_playback_speed, scaled_time_delta};

pub type SharedPlayerState = Rc<RefCell<PlayerState>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Snapshot {
    pub position: Time,
    pub duration: Time,
    pub frame_rate: Fraction,
    pub playing: bool,
    pub scrubbing: bool,
    pub playback_speed: Fraction,
    pub revision: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ProjectChange {
    pub duration: Option<Time>,
    pub frame_rate: Option<Fraction>,
    pub audio: bool,
    pub audio_beats: bool,
    pub audio_waveforms: bool,
    pub video: bool,
    pub live_preview: bool,
    pub captions: bool,
    pub inspector: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PositionChange {
    Seek,
    Playback,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StateChange {
    pub position: Option<PositionChange>,
    pub duration: bool,
    pub playing: bool,
    pub playback_speed: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlayerEvent {
    State(StateChange),
    Project(ProjectChange),
}

pub struct PlayerState {
    position: Time,
    duration: Time,
    frame_rate: Fraction,
    playing: bool,
    scrubbing: bool,
    playback_speed: Fraction,
    clock: Option<PlaybackAnchor>,
    revision: u64,
    listeners: Vec<ListenerEntry>,
}

#[derive(Clone, Copy)]
struct PlaybackAnchor {
    position: Time,
    started_at: Instant,
}

type PlayerListener = Rc<dyn Fn(PlayerEvent)>;
type ListenerAlive = Rc<dyn Fn() -> bool>;

struct ListenerEntry {
    label: &'static str,
    listener: PlayerListener,
    alive: Option<ListenerAlive>,
}

pub fn new(duration: Time, frame_rate: Fraction) -> SharedPlayerState {
    assert_positive_frame_rate(frame_rate);
    Rc::new(RefCell::new(PlayerState {
        position: Time::ZERO,
        duration: duration.max(Time::ZERO),
        frame_rate,
        playing: false,
        scrubbing: false,
        playback_speed: default_playback_speed(),
        clock: None,
        revision: 0,
        listeners: Vec::new(),
    }))
}

pub fn connect_named(
    state: &SharedPlayerState,
    label: &'static str,
    listener: impl Fn(PlayerEvent) + 'static,
) {
    state.borrow_mut().listeners.push(ListenerEntry {
        label,
        listener: Rc::new(listener),
        alive: None,
    });
}

pub fn connect_while_alive_named(
    state: &SharedPlayerState,
    label: &'static str,
    alive: impl Fn() -> bool + 'static,
    listener: impl Fn(PlayerEvent) + 'static,
) {
    // Use for UI owned by rebuildable trees. Plain connect leaks stale callbacks.
    state.borrow_mut().listeners.push(ListenerEntry {
        label,
        listener: Rc::new(listener),
        alive: Some(Rc::new(alive)),
    });
}

pub fn snapshot(state: &SharedPlayerState) -> Snapshot {
    snapshot_inner(&state.borrow())
}

/// The authoritative rational project time.
pub fn current_time(state: &SharedPlayerState) -> Time {
    state.borrow().position
}

pub fn refresh_project(state: &SharedPlayerState, change: ProjectChange) {
    update(state, |state| {
        let now = Instant::now();
        advance_to(state, now);
        if let Some(frame_rate) = change.frame_rate {
            assert_positive_frame_rate(frame_rate);
            state.frame_rate = frame_rate;
            state.position = state.position.snapped(frame_step(frame_rate));
        }
        if let Some(duration) = change.duration {
            state.duration = duration.max(Time::ZERO);
        }
        reset_clock(state, now);
        state.revision = state.revision.wrapping_add(1);
        Some(PlayerEvent::Project(change))
    });
}

pub fn seek_time(state: &SharedPlayerState, requested: Time) {
    update(state, |state| seek_inner(state, requested));
}

/// Interaction quality hint; seeking itself remains the source of position events.
pub fn set_scrubbing(state: &SharedPlayerState, scrubbing: bool) {
    state.borrow_mut().scrubbing = scrubbing;
}

pub fn set_duration(state: &SharedPlayerState, duration: Time) {
    update(state, |state| {
        let duration = duration.max(Time::ZERO);
        if state.duration == duration {
            return None;
        }
        state.duration = duration;
        Some(PlayerEvent::State(StateChange {
            duration: true,
            ..StateChange::default()
        }))
    });
}

pub fn set_playing(state: &SharedPlayerState, playing: bool) {
    update(state, |state| {
        if playing {
            state.scrubbing = false;
        }
        let now = Instant::now();
        let position_changed = advance_to(state, now);
        if state.playing == playing {
            return position_changed.then_some(PlayerEvent::State(StateChange {
                position: Some(PositionChange::Playback),
                ..StateChange::default()
            }));
        }
        tracing::trace!(
            "Player playing changed: {} -> {} at {}",
            state.playing,
            playing,
            state.position.as_label()
        );
        let speed_changed = state.playback_speed != default_playback_speed();
        state.playback_speed = default_playback_speed();
        state.playing = playing;
        reset_clock(state, now);
        Some(PlayerEvent::State(StateChange {
            position: position_changed.then_some(PositionChange::Playback),
            playing: true,
            playback_speed: speed_changed,
            ..StateChange::default()
        }))
    });
}

pub fn set_playback_speed(state: &SharedPlayerState, playback_speed: Fraction) {
    assert!(
        time_from_frame(0, playback_speed).is_some(),
        "playback speed must be positive"
    );
    update(state, |state| {
        let now = Instant::now();
        let position_changed = advance_to(state, now);
        if state.playback_speed == playback_speed {
            return position_changed.then_some(PlayerEvent::State(StateChange {
                position: Some(PositionChange::Playback),
                ..StateChange::default()
            }));
        }
        tracing::info!(
            "Player playback speed changed: {} -> {}",
            state.playback_speed,
            playback_speed
        );
        state.playback_speed = playback_speed;
        reset_clock(state, now);
        Some(PlayerEvent::State(StateChange {
            position: position_changed.then_some(PositionChange::Playback),
            playback_speed: true,
            ..StateChange::default()
        }))
    });
}

pub fn tick(state: &SharedPlayerState) {
    update(state, |state| {
        if !state.playing {
            return None;
        }
        let position_changed = advance_to(state, Instant::now());
        position_changed.then_some(PlayerEvent::State(StateChange {
            position: Some(PositionChange::Playback),
            ..StateChange::default()
        }))
    });
}

pub fn step_playback_speed_forward(state: &SharedPlayerState) {
    let snapshot = snapshot(state);
    if !snapshot.playing {
        set_playback_speed(state, default_playback_speed());
        set_playing(state, true);
        return;
    }

    let next = match shrimply_project::project::fraction_numerator(snapshot.playback_speed) {
        value if value < 2 => 2,
        value if value < 4 => 4,
        _ => 8,
    };
    set_playback_speed(state, Fraction::new_raw(next, 1));
}

pub fn toggle_playing(state: &SharedPlayerState) {
    let snapshot = snapshot(state);
    set_playing(state, !snapshot.playing);
}

fn snapshot_inner(state: &PlayerState) -> Snapshot {
    Snapshot {
        position: state.position,
        duration: state.duration,
        frame_rate: state.frame_rate,
        playing: state.playing,
        scrubbing: state.scrubbing,
        playback_speed: state.playback_speed,
        revision: state.revision,
    }
}

fn seek_inner(state: &mut PlayerState, requested: Time) -> Option<PlayerEvent> {
    let position = requested
        .max(Time::ZERO)
        .snapped(frame_step(state.frame_rate));
    if state.position == position {
        return None;
    }
    let previous = state.position;
    state.position = position;
    reset_clock(state, Instant::now());
    tracing::trace!(
        "Player seek: {} -> {}",
        previous.as_label(),
        position.as_label()
    );
    Some(PlayerEvent::State(StateChange {
        position: Some(PositionChange::Seek),
        ..StateChange::default()
    }))
}

fn advance_to(state: &mut PlayerState, now: Instant) -> bool {
    if !state.playing {
        return false;
    }
    let position = playback_position_at(state, now);
    if position == state.position {
        return false;
    }
    state.position = position;
    true
}

fn playback_position_at(state: &mut PlayerState, now: Instant) -> Time {
    let anchor = state.clock.unwrap_or(PlaybackAnchor {
        position: state.position,
        started_at: now,
    });
    state.clock = Some(anchor);
    let elapsed = now.saturating_duration_since(anchor.started_at);
    let delta = scaled_time_delta(Time::from_duration(elapsed), state.playback_speed)
        .snapped(frame_step(state.frame_rate));
    anchor.position.saturating_add(delta)
}

fn reset_clock(state: &mut PlayerState, now: Instant) {
    state.clock = state.playing.then_some(PlaybackAnchor {
        position: state.position,
        started_at: now,
    });
}

fn frame_step(frame_rate: Fraction) -> Time {
    time_from_frame(1, frame_rate).expect("validated playback FPS must produce an exact frame step")
}

fn assert_positive_frame_rate(frame_rate: Fraction) {
    assert!(
        time_from_frame(0, frame_rate).is_some(),
        "playback frame rate must be positive"
    );
}

fn update(state: &SharedPlayerState, change: impl FnOnce(&mut PlayerState) -> Option<PlayerEvent>) {
    let (event, listeners) = {
        let mut state = state.borrow_mut();
        let Some(event) = change(&mut state) else {
            return;
        };
        state
            .listeners
            .retain(|entry| entry.alive.as_ref().is_none_or(|alive| alive()));
        let listeners = state
            .listeners
            .iter()
            .map(|entry| (entry.label, entry.listener.clone(), entry.alive.clone()))
            .collect::<Vec<_>>();
        (event, listeners)
    };

    let listener_count = listeners.len();
    let _span = tracing::debug_span!(
        "player.dispatch",
        event = ?event,
        listener_count,
    )
    .entered();
    let _measurement = shrimply_benchmarking::measure("Player / Dispatch");
    for (index, (label, listener, alive)) in listeners.into_iter().enumerate() {
        if alive.as_ref().is_some_and(|alive| !alive()) {
            continue;
        }
        let _listener_span = tracing::debug_span!("player.listener", label, index).entered();
        shrimply_support::crash::set_context(format!(
            "player listener begin {label} event={event:?}"
        ));
        listener(event);
        shrimply_support::crash::set_context(format!(
            "player listener end {label} event={event:?}"
        ));
    }
}
