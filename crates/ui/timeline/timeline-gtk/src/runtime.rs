use super::*;
pub use shrimply_timeline_skia::scene::{
    TimelineModifiers, TrackAddMenuRequest, selected_timeline_items, selected_timeline_tracks,
};
pub(super) struct TimelineRuntime {
    pub(super) scene: shrimply_timeline_skia::scene::Scene,
    pub(super) renderer: TimelineRenderer,
    pub(super) animation_tick_active: bool,
    pub(super) screen_recording: Option<video_recording::ScreenRecording>,
    pub(super) active_context_menu: Option<gtk::Popover>,
}
impl TimelineRuntime {
    pub(super) fn new(
        project: Rc<RefCell<Project>>,
        player: SharedPlayerState,
        selection: SharedSelectionState,
        preferences: preferences_store::SharedPreferences,
        property_clipboard: shrimply_property_transfer::SharedClipboard,
        playback_performance: playback_performance::SharedCollector,
    ) -> Self {
        let mut scene = shrimply_timeline_skia::scene::Scene::new(
            project,
            player,
            selection,
            preferences,
            property_clipboard,
            playback_performance,
        );
        scene.set_stabilization_handler(shrimply_visual_cuda::video_stabilization::request);
        Self {
            scene,
            renderer: TimelineRenderer::new(),
            animation_tick_active: false,
            screen_recording: None,
            active_context_menu: None,
        }
    }
}
