use super::*;
use crate::items::*;
use crate::timeline_operation::SequenceTimeline;
use project::FontFamily;
use selection_state::SharedSelectionState;
use shrimply_state::player_state::{self, ProjectChange, SharedPlayerState};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};
mod context_edits;
mod context_menu;
mod keyboard;
pub use keyboard::{KeyAction, TrackDeletion};
mod drop_preview;
mod frame;
mod input;
pub use input::{Event, PointerButton, PointerState, Requests};
pub mod pointer;
mod scrolling;
pub use frame::Frame;
/// Owns timeline interaction, drawing, selection, media jobs and subscription lifetime.
/// Native hosts supply a canvas and translate OS events; no GPU context is stored here.
pub struct Scene {
    snap_enabled: bool,
    beat_grid_enabled: bool,
    snap_radius_px: f64,
    cut_enabled: bool,
    pub drag_collision_mode: DragCollisionMode,
    suppress_double_click_selection: bool,
    started_at: Instant,
    pending_scrolls: Vec<TimelineScrollEvent>,
    overscroll: Option<TimelineOverscroll>,
    horizontal_scrollbar: shrimply_skia_adw_core::slider::Lifecycle,
    vertical_scrollbar: shrimply_skia_adw_core::slider::Lifecycle,
    modifiers: TimelineModifiers,
    pointer_pos: Option<Vec2>,
    software_cursor: Option<TimelineSoftwareCursor>,
    pointer_press_origin: Option<Vec2>,
    pointer_release_pos: Option<Vec2>,
    primary_pressed: bool,
    primary_down: bool,
    primary_released: bool,
    middle_pressed: bool,
    middle_down: bool,
    middle_released: bool,
    view: TimelineViewState,
    pending_track_toggle: Option<TrackKey>,
    pending_sequence_toggle: Option<Vec<uuid::Uuid>>,
    pending_track_add_menu: Option<TrackAddMenuRequest>,
    pending_audio_record: Option<TrackKey>,
    pending_video_record: Option<TrackKey>,
    pending_pause_playback: bool,
    playhead_visibility_requested: Rc<Cell<bool>>,
    last_playhead_position: Option<Time>,
    initial_center: Option<Time>,
    track_buttons: HashMap<TrackButtonId, shrimply_skia_adw_core::button::Button>,
    hovered_track_button: Option<TrackButtonId>,
    pressed_track_button: Option<TrackButtonId>,
    pressed_track_selection: Option<TrackKey>,
    track_controls_animating: bool,
    dragged_group: Option<DraggedGroup>,
    folded_drag: Option<folded_sequence::FoldedDrag>,
    resize_drag: Option<ResizeDrag>,
    transition_drag: Option<TransitionDrag>,
    clip_transition_drag: Option<ClipTransitionDrag>,
    pub clipboard: Option<TimelineClipboard>,
    pub property_clipboard: shrimply_property_transfer::SharedClipboard,
    import_preview: Option<TimelineImportPreview>,
    text_drop_preview: Option<crate::external_content::TextPreview>,
    cut_preview: Option<TimelineCut>,
    pub waveforms: WaveformMap,
    beats: BeatMap,
    pub snap_repository: crate::snapping::SnapRepo,
    pub default_visual_duration: Time,
    pub default_text_font_family: FontFamily,
    project: Rc<RefCell<Project>>,
    player: SharedPlayerState,
    selection: SharedSelectionState,
    preferences: SharedPreferences,
    beat_updates: mpsc::Receiver<(uuid::Uuid, audio::beat::BeatUpdate)>,
    waveform_updates: mpsc::Receiver<(uuid::Uuid, waveform::WaveformUpdate)>,
    waveform_cancel: Arc<AtomicBool>,
    beat_cancel: Arc<AtomicBool>,
    media_refresh: Rc<MediaRefresh>,
    viewport: Rect,
    context: context_menu::Context,
    drop_preview: Option<drop_preview::DropPreview>,
    pending_seek: Option<Time>,
    suspended: bool,
    revision: u64,
}
#[derive(Default)]
struct MediaRefresh {
    waveforms: Cell<Option<Instant>>,
    beats: Cell<bool>,
}
pub fn selected_timeline_items(selection_state: &SharedSelectionState) -> Vec<ItemKey> {
    selection_state::selected_items(selection_state)
}

pub fn selected_timeline_tracks(selection_state: &SharedSelectionState) -> Vec<TrackKey> {
    selection_state::selected_tracks(selection_state)
}

pub fn focused_timeline_transition(
    selection_state: &SharedSelectionState,
    project: &Project,
) -> Option<(crate::project::ItemAddress, TransitionSide)> {
    selection_state::focused_transition_address(selection_state, project)
}

#[derive(Clone, Copy, Default)]
pub struct TimelineModifiers {
    pub ctrl: bool,
    pub shift: bool,
}

pub struct TrackAddMenuRequest {
    pub key: TrackKey,
    pub import_targets: Vec<TrackKey>,
}

impl Scene {
    pub fn new(
        project: Rc<RefCell<Project>>,
        player: SharedPlayerState,
        selection: SharedSelectionState,
        preferences: SharedPreferences,
        property_clipboard: shrimply_property_transfer::SharedClipboard,
    ) -> Self {
        let (_, waveform_updates) = mpsc::channel();
        let (_, beat_updates) = mpsc::channel();
        let playhead_visibility_requested = Rc::new(Cell::new(false));
        let visibility_alive = Rc::downgrade(&playhead_visibility_requested);
        let visibility_request = visibility_alive.clone();
        player_state::connect_while_alive_named(
            &player,
            "timeline playhead visibility",
            move || visibility_alive.strong_count() > 0,
            move |event| {
                if matches!(event, player_state::PlayerEvent::State(_))
                    && let Some(request) = visibility_request.upgrade()
                {
                    request.set(true);
                }
            },
        );
        let media_refresh = Rc::new(MediaRefresh {
            waveforms: Cell::new(Some(Instant::now() - WAVEFORM_RELOAD_DELAY)),
            beats: Cell::new(true),
        });
        let refresh_alive = Rc::downgrade(&media_refresh);
        let refresh_request = refresh_alive.clone();
        player_state::connect_while_alive_named(
            &player,
            "timeline media refresh",
            move || refresh_alive.strong_count() > 0,
            move |event| {
                let Some(request) = refresh_request.upgrade() else {
                    return;
                };
                if let player_state::PlayerEvent::Project(change) = event {
                    if change.audio_waveforms || change.frame_rate.is_some() {
                        request.waveforms.set(Some(Instant::now()));
                    }
                    request
                        .beats
                        .set(request.beats.get() || change.audio || change.audio_beats);
                }
            },
        );
        let mut view = TimelineViewState::default();
        view.restore_zoom(project.borrow().timeline_zoom);
        let initial_center = view
            .initialized
            .then(|| project.borrow().cursor_position)
            .flatten();
        let snapshot = preferences::snapshot(&preferences);
        let tools = ToolState::from_preferences(&snapshot);

        let revision = player_state::snapshot(&player).revision;
        Self {
            snap_enabled: tools.magnet,
            beat_grid_enabled: tools.beat_grid,
            snap_radius_px: f64::from(snapshot.timeline_snap_radius_px),
            cut_enabled: tools.cursor == CursorTool::Cut,
            drag_collision_mode: tools.drag_collision,
            suppress_double_click_selection: false,
            started_at: Instant::now(),
            pending_scrolls: Vec::new(),
            overscroll: None,
            horizontal_scrollbar: shrimply_skia_adw_core::slider::Lifecycle::default(),
            vertical_scrollbar: shrimply_skia_adw_core::slider::Lifecycle::default(),
            modifiers: TimelineModifiers::default(),
            pointer_pos: None,
            software_cursor: None,
            pointer_press_origin: None,
            pointer_release_pos: None,
            primary_pressed: false,
            primary_down: false,
            primary_released: false,
            middle_pressed: false,
            middle_down: false,
            middle_released: false,
            view,
            pending_track_toggle: None,
            pending_sequence_toggle: None,
            pending_track_add_menu: None,
            pending_audio_record: None,
            pending_video_record: None,
            pending_pause_playback: false,
            playhead_visibility_requested: playhead_visibility_requested.clone(),
            last_playhead_position: None,
            initial_center,
            track_buttons: HashMap::new(),
            hovered_track_button: None,
            pressed_track_button: None,
            pressed_track_selection: None,
            track_controls_animating: false,
            dragged_group: None,
            folded_drag: None,
            resize_drag: None,
            transition_drag: None,
            clip_transition_drag: None,
            clipboard: None,
            property_clipboard,
            import_preview: None,
            text_drop_preview: None,
            cut_preview: None,
            waveforms: WaveformMap::new(),
            beats: BeatMap::new(),
            snap_repository: crate::snapping::SnapRepo::default(),
            default_visual_duration: snapshot.default_visual_duration,
            default_text_font_family: snapshot.default_text_font_family,
            project,
            player,
            selection,
            preferences,
            beat_updates,
            waveform_updates,
            waveform_cancel: Arc::new(AtomicBool::new(false)),
            beat_cancel: Arc::new(AtomicBool::new(false)),
            media_refresh,
            viewport: Rect::from_min_size(Vec2::ZERO, Vec2::ZERO),
            context: context_menu::Context::default(),
            drop_preview: None,
            pending_seek: None,
            suspended: false,
            revision,
        }
    }
    fn finish_pointer_frame(&mut self) {
        self.primary_pressed = false;
        self.primary_released = false;
        self.pointer_release_pos = None;
        self.middle_pressed = false;
        self.middle_released = false;
    }

    fn apply_preferences(&mut self, preferences: &preferences::PreferencesSnapshot) {
        let tools = ToolState::from_preferences(preferences);
        self.snap_enabled = tools.magnet;
        self.beat_grid_enabled = tools.beat_grid;
        self.cut_enabled = tools.cursor == CursorTool::Cut;
        self.drag_collision_mode = tools.drag_collision;
        self.snap_radius_px = f64::from(preferences.timeline_snap_radius_px);
        self.default_visual_duration = preferences.default_visual_duration;
        self.default_text_font_family = preferences.default_text_font_family.clone();
    }
    fn reload_waveforms(&mut self) {
        self.waveform_cancel.store(true, Ordering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        self.waveform_cancel = Arc::clone(&cancel);
        let (sender, receiver) = mpsc::channel();
        self.waveform_updates = receiver;
        self.waveforms.clear();
        let snapshot = self.project.borrow().clone();
        let chunks = waveform_chunks_per_second_from_frame_step(frame_step_seconds(&snapshot));
        std::thread::spawn(move || {
            waveform::load_project_waveforms_cancellable(
                &snapshot,
                chunks,
                || cancel.load(Ordering::Relaxed),
                |key, update| {
                    let _ = sender.send((key, update));
                },
            );
        });
    }

    fn reload_beats(&mut self) {
        self.beat_cancel.store(true, Ordering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        self.beat_cancel = Arc::clone(&cancel);
        let (sender, receiver) = mpsc::channel();
        self.beat_updates = receiver;
        let snapshot = self.project.borrow().clone();
        audio::beat::begin_loading(&snapshot);
        audio::beat::retain_enabled(&mut self.beats, &snapshot);
        std::thread::spawn(move || {
            audio::beat::load_project_beats_cancellable(
                &snapshot,
                || cancel.load(Ordering::Relaxed),
                |key, update| {
                    let _ = sender.send((key, update));
                },
            );
        });
    }
}
impl Drop for Scene {
    fn drop(&mut self) {
        if self.view.drag_mode == DragMode::Seek {
            player_state::set_scrubbing(&self.player, false);
        }
        self.waveform_cancel.store(true, Ordering::Relaxed);
        self.beat_cancel.store(true, Ordering::Relaxed);
    }
}
