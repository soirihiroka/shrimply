use super::*;
pub use shrimply_timeline_core::draw_state::*;
pub use shrimply_timeline_core::scene::{
    TimelineModifiers, TrackAddMenuRequest, selected_timeline_items, selected_timeline_tracks,
};
pub(super) struct TimelineRuntime {
    pub(super) scene: shrimply_timeline_core::scene::Scene,
    pub(super) renderer: TimelineRenderer,
    pub(super) animation_tick_active: bool,
    pub(super) active_audio_recording: Option<ActiveAudioRecording>,
    pub(super) active_video_recording: Option<ActiveVideoRecording>,
    pub(super) active_context_menu: Option<gtk::Popover>,
    pub(super) resource_jobs: Vec<shrimply_gtk_components::resource_pipeline::UiSubscription>,
}
impl TimelineRuntime {
    pub(super) fn new(
        project: Rc<RefCell<Project>>,
        player: SharedPlayerState,
        selection: SharedSelectionState,
        preferences: preferences_store::SharedPreferences,
        property_clipboard: shrimply_property_transfer::SharedClipboard,
    ) -> Self {
        Self {
            scene: shrimply_timeline_core::scene::Scene::new(
                project,
                player,
                selection,
                preferences,
                property_clipboard,
            ),
            renderer: TimelineRenderer::new(),
            animation_tick_active: false,
            active_audio_recording: None,
            active_video_recording: None,
            active_context_menu: None,
            resource_jobs: Vec::new(),
        }
    }
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
    pub(super) recording: video_recording::ScreenRecording,
    pub(super) ready: bool,
    pub(super) stopping: bool,
}
