use super::*;

pub(super) struct TimelineSoftwareCursor {
    pub(super) position: Vec2,
    pub(super) cursor: cursor::SoftwareCursor,
}

pub(super) struct TimelineRuntime {
    pub(super) renderer: TimelineRenderer,
    pub(super) snap_enabled: bool,
    pub(super) beat_grid_enabled: bool,
    pub(super) snap_radius_px: f64,
    pub(super) cut_enabled: bool,
    pub(super) drag_collision_mode: DragCollisionMode,
    pub(super) suppress_double_click_selection: bool,
    pub(super) started_at: Instant,
    pub(super) pending_scrolls: Vec<TimelineScrollEvent>,
    pub(super) overscroll: Option<TimelineOverscroll>,
    pub(super) animation_tick_active: bool,
    pub(super) horizontal_scrollbar: shrimply_skia_adw_ui::slider::Lifecycle,
    pub(super) vertical_scrollbar: shrimply_skia_adw_ui::slider::Lifecycle,
    pub(super) modifiers: TimelineModifiers,
    pub(super) pointer_pos: Option<Vec2>,
    pub(super) software_cursor: Option<TimelineSoftwareCursor>,
    pub(super) pointer_press_origin: Option<Vec2>,
    pub(super) pointer_release_pos: Option<Vec2>,
    pub(super) primary_pressed: bool,
    pub(super) primary_down: bool,
    pub(super) primary_released: bool,
    pub(super) middle_pressed: bool,
    pub(super) middle_down: bool,
    pub(super) middle_released: bool,
    pub(super) view: TimelineViewState,
    pub(super) pending_track_toggle: Option<TrackKey>,
    pub(super) pending_sequence_toggle: Option<Vec<uuid::Uuid>>,
    pub(super) pending_track_add_menu: Option<TrackAddMenuRequest>,
    pub(super) pending_audio_record: Option<TrackKey>,
    pub(super) pending_video_record: Option<TrackKey>,
    pub(super) pending_pause_playback: bool,
    pub(super) playhead_visibility_requested: Rc<Cell<bool>>,
    pub(super) last_playhead_position: Option<Time>,
    pub(super) initial_center: Option<Time>,
    pub(super) active_audio_recording: Option<ActiveAudioRecording>,
    pub(super) active_video_recording: Option<ActiveVideoRecording>,
    pub(super) track_buttons: HashMap<TrackButtonId, shrimply_skia_adw_ui::button::Button>,
    pub(super) hovered_track_button: Option<TrackButtonId>,
    pub(super) pressed_track_button: Option<TrackButtonId>,
    pub(super) pressed_track_selection: Option<TrackKey>,
    pub(super) track_controls_animating: bool,
    pub(super) dragged_group: Option<DraggedGroup>,
    pub(super) folded_drag: Option<folded_sequence::FoldedDrag>,
    pub(super) resize_drag: Option<ResizeDrag>,
    pub(super) transition_drag: Option<TransitionDrag>,
    pub(super) clip_transition_drag: Option<ClipTransitionDrag>,
    pub(super) clipboard: Option<TimelineClipboard>,
    pub(super) property_clipboard: shrimply_property_transfer::SharedClipboard,
    pub(super) active_context_menu: Option<gtk::Popover>,
    pub(super) resource_jobs: Vec<shrimply_ui_foundation::resource_pipeline::UiSubscription>,
    pub(super) import_preview: Option<TimelineImportPreview>,
    pub(super) text_drop_preview: Option<external_content::TextPreview>,
    pub(super) cut_preview: Option<TimelineCut>,
    pub(super) waveforms: WaveformMap,
    pub(super) waveform_job: Option<shrimply_ui_foundation::resource_pipeline::UiSubscription>,
    pub(super) beats: BeatMap,
    pub(super) beat_job: Option<shrimply_ui_foundation::resource_pipeline::UiSubscription>,
    pub(super) snap_repository: crate::snapping::SnapRepo,
    pub(super) default_visual_duration: Time,
    pub(super) default_text_font_family: FontFamily,
}

pub(super) fn selected_timeline_items(selection_state: &SharedSelectionState) -> Vec<ItemKey> {
    selection_state::selected_items(selection_state)
}

pub(super) fn selected_timeline_tracks(selection_state: &SharedSelectionState) -> Vec<TrackKey> {
    selection_state::selected_tracks(selection_state)
}

pub(super) fn focused_timeline_transition(
    selection_state: &SharedSelectionState,
    project: &Project,
) -> Option<(crate::project::ItemAddress, TransitionSide)> {
    selection_state::focused_transition_address(selection_state, project)
}

impl TimelineRuntime {
    pub(super) fn new(
        waveforms: WaveformMap,
        preferences: preferences_store::PreferencesSnapshot,
        playhead_visibility_requested: Rc<Cell<bool>>,
        timeline_zoom: Option<Time>,
        timeline_center: Option<Time>,
        property_clipboard: shrimply_property_transfer::SharedClipboard,
    ) -> Self {
        let mut view = TimelineViewState::default();
        if let Some(zoom) = timeline_zoom.filter(|zoom| *zoom > Time::ZERO) {
            let seconds_per_pixel = zoom.as_secs_f64();
            if seconds_per_pixel.is_finite() {
                view.seconds_per_pixel = seconds_per_pixel;
                view.drag_start_seconds_per_pixel = seconds_per_pixel;
                view.initialized = true;
            }
        }
        Self {
            renderer: TimelineRenderer::new(),
            snap_enabled: timeline_magnet_from_preference(&preferences.timeline_magnet),
            beat_grid_enabled: timeline_beat_grid_from_preference(&preferences.timeline_beat_grid),
            snap_radius_px: f64::from(preferences.timeline_snap_radius_px),
            cut_enabled: preferences.timeline_cursor == "cut",
            drag_collision_mode: drag_collision_mode_from_preference(
                &preferences.timeline_drag_collision_mode,
            ),
            suppress_double_click_selection: false,
            started_at: Instant::now(),
            pending_scrolls: Vec::new(),
            overscroll: None,
            animation_tick_active: false,
            horizontal_scrollbar: shrimply_skia_adw_ui::slider::Lifecycle::default(),
            vertical_scrollbar: shrimply_skia_adw_ui::slider::Lifecycle::default(),
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
            playhead_visibility_requested,
            last_playhead_position: None,
            initial_center: timeline_center,
            active_audio_recording: None,
            active_video_recording: None,
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
            active_context_menu: None,
            resource_jobs: Vec::new(),
            import_preview: None,
            text_drop_preview: None,
            cut_preview: None,
            waveforms,
            waveform_job: None,
            beats: BeatMap::new(),
            beat_job: None,
            snap_repository: crate::snapping::SnapRepo::default(),
            default_visual_duration: preferences.default_visual_duration,
            default_text_font_family: preferences.default_text_font_family,
        }
    }

    pub(super) fn finish_pointer_frame(&mut self) {
        self.primary_pressed = false;
        self.primary_released = false;
        self.pointer_release_pos = None;
        self.middle_pressed = false;
        self.middle_released = false;
    }
}

#[derive(Clone, Copy, Default)]
pub(super) struct TimelineModifiers {
    pub(super) ctrl: bool,
    pub(super) shift: bool,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum TrackLabelAction {
    Select,
    Toggle,
    Add,
    AudioRecord,
    VideoRecord,
}

pub(crate) type TrackButtonId = (TrackKey, TrackLabelAction);

pub(super) struct TrackAddMenuRequest {
    pub(super) key: TrackKey,
    pub(super) import_targets: Vec<TrackKey>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum GeneratedItemKind {
    Text,
    Shape,
    Paint,
    Background,
    Scene3d,
}

pub(super) struct ActiveAudioRecording {
    pub(super) key: TrackKey,
    pub(super) start: Time,
    pub(super) recording: crate::audio::recording::MicRecording,
}

pub(super) struct ActiveVideoRecording {
    pub(super) key: TrackKey,
    pub(super) start: Time,
    pub(super) stop_at: Option<Time>,
    #[cfg(feature = "screen-recording")]
    pub(super) recording: video_recording::ScreenRecording,
    pub(super) ready: bool,
    pub(super) stopping: bool,
}

pub(super) struct LiveRecordingDraw {
    pub(super) key: TrackKey,
    pub(super) item: AudioItem,
    pub(super) waveform: waveform::Waveform,
}

#[derive(Clone, Copy)]
pub(super) struct LiveVideoRecordingDraw {
    pub(super) key: TrackKey,
    pub(super) start: Time,
    pub(super) end: Time,
}

#[derive(Clone, Copy)]
pub(super) enum WaveformState<'a> {
    Loading,
    Loaded(Option<&'a waveform::Waveform>),
}

#[derive(Clone, Copy)]
pub(super) struct NaturalEndMarker {
    pub(super) start: Time,
    pub(super) end: Time,
    pub(super) position: Option<Time>,
    pub(super) repeat_interval: Option<Time>,
    pub(super) real_start: Option<Time>,
    pub(super) real_end: Option<Time>,
}

#[derive(Clone, Copy)]
pub(super) struct TimedItemBox {
    pub(super) marker: NaturalEndMarker,
    pub(super) bounds: Rect,
    pub(super) fill: Color,
    pub(super) timeline_x: f64,
    pub(super) view: TimelineViewState,
    pub(super) selected: bool,
    pub(super) selected_border_color: Color,
}

#[derive(Clone, Copy)]
pub(super) enum PreviewTimeMode {
    Move,
    Resize,
}

pub(super) struct TimelineDraw<'a> {
    pub(super) painter: &'a TimelinePainter,
    pub(super) waveforms: &'a WaveformMap,
    pub(super) timeline_x: f64,
    pub(super) timeline_width: f64,
    pub(super) waveform_chunks_per_second: u32,
    pub(super) view: TimelineViewState,
    pub(super) animation_seconds: f64,
}

pub(super) struct TrackControlDraw<'a> {
    pub(super) animation_active: &'a mut bool,
    pub(super) buttons: &'a mut HashMap<TrackButtonId, shrimply_skia_adw_ui::button::Button>,
    pub(super) active_audio_recording_key: Option<TrackKey>,
    pub(super) active_video_recording_key: Option<TrackKey>,
}

#[derive(Clone)]
pub(super) struct TimelineImportPreview {
    pub(super) source: crate::project::Asset,
    pub(super) duration: Time,
    pub(super) visual_kind: Option<import::VisualMediaKind>,
    pub(super) preview: import::ImportPreview,
    pub(super) y: f64,
}

#[derive(Clone)]
pub(super) struct TimelineCut {
    pub(super) key: crate::project::ItemAddress,
    pub(super) time: Time,
    pub(super) keys: Vec<crate::project::ItemAddress>,
}
