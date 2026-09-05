use super::*;
use shrimply_state::player_state;
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
mod dragging;
mod editing;
mod keyboard;
pub use keyboard::KeyAction;
mod drop_preview;
mod scrolling;
mod track_buttons;

/// Toolkit-independent state for presenting the existing timeline on a Skia canvas.
pub struct Scene {
    pub view: TimelineViewState,
    project: Rc<RefCell<Project>>,
    player: player_state::SharedPlayerState,
    selection: selection_state::SharedSelectionState,
    preferences: SharedPreferences,
    waveforms: WaveformMap,
    beats: BeatMap,
    beat_updates: mpsc::Receiver<(uuid::Uuid, audio::beat::BeatUpdate)>,
    waveform_updates: mpsc::Receiver<(uuid::Uuid, waveform::WaveformUpdate)>,
    waveform_cancel: Arc<AtomicBool>,
    revision: Option<u64>,
    buttons: HashMap<TrackButtonId, shrimply_skia_adw_core::button::Button>,
    hovered_button: Option<TrackButtonId>,
    pressed_label: Option<track_buttons::PressedTrackLabel>,
    pointer: Option<Vec2>,
    viewport: Rect,
    horizontal_scrollbar: shrimply_skia_adw_core::slider::Lifecycle,
    vertical_scrollbar: shrimply_skia_adw_core::slider::Lifecycle,
    scrollbar_drag: Option<shrimply_skia_adw_core::Axis>,
    started: Instant,
    seeking: bool,
    last_playhead_position: Option<Time>,
    playhead_visibility_requested: Rc<Cell<bool>>,
    initial_center: Option<Time>,
    dragged_group: Option<DraggedGroup>,
    resize_drag: Option<items::ResizeDrag>,
    folded_drag: Option<folded_sequence::FoldedDrag>,
    cut_preview: Option<TimelineCut>,
    cutting: bool,
    transition: Option<crate::transitions::Gesture>,
    drag_origin: Vec2,
    drag_moved: bool,
    selection_toggle: bool,
    double_click: bool,
    snap_repository: shrimply_timeline_snap::SnapRepo,
    context: context_menu::Context,
    clipboard: Option<items::TimelineClipboard>,
    property_clipboard: shrimply_property_transfer::SharedClipboard,
    drop_preview: Option<drop_preview::DropPreview>,
    import_preview: Option<TimelineImportPreview>,
}

impl Scene {
    pub fn new(
        project: Rc<RefCell<Project>>,
        player: player_state::SharedPlayerState,
        selection: selection_state::SharedSelectionState,
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
        let mut view = TimelineViewState::default();
        view.restore_zoom(project.borrow().timeline_zoom);
        let initial_center = view
            .initialized
            .then(|| project.borrow().cursor_position)
            .flatten();
        Self {
            view,
            project,
            player,
            selection,
            preferences,
            waveforms: WaveformMap::new(),
            beats: BeatMap::new(),
            beat_updates,
            waveform_updates,
            waveform_cancel: Arc::new(AtomicBool::new(false)),
            revision: None,
            buttons: HashMap::new(),
            hovered_button: None,
            pressed_label: None,
            pointer: None,
            viewport: Rect::from_min_size(Vec2::ZERO, Vec2::ZERO),
            horizontal_scrollbar: shrimply_skia_adw_core::slider::Lifecycle::default(),
            vertical_scrollbar: shrimply_skia_adw_core::slider::Lifecycle::default(),
            scrollbar_drag: None,
            started: Instant::now(),
            seeking: false,
            last_playhead_position: None,
            playhead_visibility_requested,
            initial_center,
            dragged_group: None,
            resize_drag: None,
            folded_drag: None,
            cut_preview: None,
            cutting: false,
            transition: None,
            drag_origin: Vec2::ZERO,
            drag_moved: false,
            selection_toggle: false,
            double_click: false,
            snap_repository: shrimply_timeline_snap::SnapRepo::default(),
            context: context_menu::Context::default(),
            clipboard: None,
            property_clipboard,
            drop_preview: None,
            import_preview: None,
        }
    }

    pub fn draw(&mut self, canvas: &skia_safe::Canvas, size: Vec2) {
        self.viewport = Rect::from_min_size(Vec2::ZERO, size);
        let mut player = player_state::snapshot(&self.player);
        if self.revision != Some(player.revision) {
            self.reload_waveforms();
            self.revision = Some(player.revision);
            self.pointer_cancelled();
        }
        while let Ok((key, update)) = self.waveform_updates.try_recv() {
            waveform::apply_update(&mut self.waveforms, key, update);
        }
        while let Ok((key, update)) = self.beat_updates.try_recv() {
            audio::beat::apply_update(&mut self.beats, key, update);
        }
        self.horizontal_scrollbar
            .apply_scroll(|value| self.view.scroll_seconds = value);
        self.vertical_scrollbar
            .apply_scroll(|value| self.view.scroll_y = value);
        if self.seeking
            && let Some(point) = self.pointer
        {
            self.pointer_dragged(point);
            player = player_state::snapshot(&self.player);
        }
        self.poll_drop_preview();
        let virtual_tracks = drawing::active_virtual_tracks(
            self.dragged_group.as_ref(),
            self.import_preview.as_ref(),
        );
        let project = self.project.borrow();
        let width = f64::from(size.x);
        let height = f64::from(size.y);
        let timeline_width = timeline_width(width);
        let duration = folded_sequence::expanded_timeline_end(&project)
            .max(player.duration)
            .max(player.position)
            .max(Time::from_seconds(1));
        let step = frame_step_seconds(&project);
        let minimum = min_seconds_per_pixel(step);
        let track_height = timeline_track_content_height(&project, &virtual_tracks);
        self.view
            .initialize(duration.as_secs_f64(), timeline_width, minimum);
        if let Some(time) = self.initial_center.take() {
            self.view.center_time(
                time,
                duration.as_secs_f64(),
                timeline_width,
                minimum,
                track_height,
                height,
            );
        }
        self.view.clamp(
            duration.as_secs_f64(),
            timeline_width,
            minimum,
            track_height,
            height,
        );
        let position_changed = self
            .last_playhead_position
            .is_some_and(|time| time != player.position);
        self.last_playhead_position = Some(player.position);
        let visibility_requested = self.playhead_visibility_requested.replace(false);
        if player.playing || self.seeking || (visibility_requested && position_changed) {
            self.horizontal_scrollbar.cancel_scroll();
            self.vertical_scrollbar.cancel_scroll();
            self.view.keep_time_visible(
                player.position,
                duration.as_secs_f64(),
                timeline_width,
                minimum,
                track_height,
                height,
            );
        }
        drop(project);
        if let Some(point) = self.pointer {
            self.pointer_moved(point);
        }
        let (horizontal, vertical) = self.scrollbar_geometry();
        let pointer = self
            .pointer
            .filter(|point| self.pointer_in_viewport(*point));
        let horizontal = self.horizontal_scrollbar.frame(Some(horizontal), pointer);
        let vertical = self.vertical_scrollbar.frame(vertical, pointer);
        let project = self.project.borrow();
        let painter = TimelinePainter::new(canvas);
        drawing::draw_timeline(drawing::TimelineInput {
            painter: &painter,
            project: &project,
            playback_performance: &playback_performance::Snapshot::default(),
            current_time: player.position,
            waveforms: &self.waveforms,
            beats: &self.beats,
            beat_grid_enabled: ToolState::from_preferences(&preferences::snapshot(
                &self.preferences,
            ))
            .beat_grid,
            selected_items: &selection_state::selected_items(&self.selection),
            selected_nested_items: &selection_state::selected_nested_items(&self.selection),
            selected_tracks: &selection_state::selected_track_addresses(&self.selection, &project),
            selected_gap: selection_state::selected_gap(&self.selection),
            track_control_draw: &mut TrackControlDraw {
                animation_active: &mut false,
                buttons: &mut self.buttons,
                active_audio_recording_key: None,
                active_video_recording_key: None,
            },
            dragged_group: self.dragged_group.as_ref().filter(|_| self.drag_moved),
            folded_drag: self.folded_drag.as_ref().filter(|_| self.drag_moved),
            resize_drag: self.resize_drag.as_ref().filter(|_| self.drag_moved),
            transition_drag: self
                .transition
                .as_ref()
                .and_then(|gesture| gesture.item.as_ref()),
            clip_transition_drag: self
                .transition
                .as_ref()
                .and_then(|gesture| gesture.clip.as_ref()),
            focused_transition: selection_state::focused_transition_address(
                &self.selection,
                &project,
            ),
            import_preview: self.import_preview.as_ref(),
            text_drop_preview: None,
            cut_preview: self.cut_preview.as_ref(),
            live_recording: None,
            live_video_recording: None,
            view: self.view,
            virtual_tracks: &virtual_tracks,
            width,
            height,
            timeline_width,
            frame_step_seconds: step,
            animation_seconds: self.started.elapsed().as_secs_f64(),
            waveform_chunks_per_second: waveform_chunks_per_second_from_frame_step(step),
            accent_color: Color::BLUE3,
            overscroll: None,
            horizontal_scrollbar: horizontal.scrollbar,
            vertical_scrollbar: vertical.scrollbar,
            software_cursor: None,
        });
    }

    pub fn double_click_down(&mut self, point: Vec2, toggle: bool, extend: bool) {
        self.pointer_cancelled();
        let path = self
            .scrollbar_at(point)
            .is_none()
            .then(|| {
                folded_sequence::hit_folded_item(
                    &self.project.borrow(),
                    self.view,
                    point.x.into(),
                    point.y.into(),
                )
            })
            .flatten();
        if let Some(path) = path {
            folded_sequence::toggle_sequence(&self.project, &self.selection, path);
            return;
        }
        self.pointer_down(point, toggle, extend);
        self.snap_repository = self.build_snap_repository();
        self.view.selection = None;
        self.double_click = true;
    }

    pub fn pointer_down(&mut self, point: Vec2, toggle: bool, extend: bool) {
        self.pointer_cancelled();
        self.drag_origin = point;
        self.selection_toggle = toggle;
        self.view.drag_start_x = point.x.into();
        self.view.drag_start_y = point.y.into();
        self.pointer_moved(point);
        if !self.pointer_in_viewport(point)
            || self.begin_scrollbar_drag(point)
            || self.press_track_label(point, toggle, extend)
        {
            return;
        }
        self.snap_repository = self.build_snap_repository();
        self.begin_item_edit(point, toggle, extend);
        self.drag_moved = false;
        player_state::set_playing(&self.player, false);
        if self.seeking {
            player_state::set_scrubbing(&self.player, true);
        }
        self.pointer_dragged(point);
    }

    pub fn pointer_dragged(&mut self, point: Vec2) {
        self.snap_repository = self.build_snap_repository();
        self.pointer_moved(point);
        if self.drag_scrollbar(point) {
            return;
        }
        if self.pressed_label.is_some() {
            self.drag_moved |=
                f64::from((point - self.drag_origin).abs().max_element()) > CLICK_DRAG_TOLERANCE;
            return;
        }
        self.drag_moved |=
            f64::from((point - self.drag_origin).abs().max_element()) > CLICK_DRAG_TOLERANCE;
        if self.update_item_edit(point) {
            return;
        }
        if let Some(group) = &mut self.dragged_group {
            self.drag_moved |=
                f64::from((point - self.drag_origin).abs().max_element()) > CLICK_DRAG_TOLERANCE;
            if self.drag_moved {
                items::update_dragged_group(
                    group,
                    &self.project.borrow(),
                    self.view,
                    point.x.into(),
                    point.y.into(),
                    &self.snap_repository,
                );
                crate::drop_area::update_root_preview(
                    crate::drop_area::DragDropContext {
                        project: &self.project.borrow(),
                        view: self.view,
                        position: point.as_dvec2(),
                        collision_mode: ToolState::from_preferences(&preferences::snapshot(
                            &self.preferences,
                        ))
                        .drag_collision,
                    },
                    group,
                );
            }
        }
        if self.seeking {
            let project = self.project.borrow();
            let target = crate::math::time_at_x(self.view, point.x.into());
            player_state::seek_time(
                &self.player,
                self.snap_repository
                    .snap(target)
                    .unwrap_or(target)
                    .snapped(project.frame_step()),
            );
        }
    }

    pub fn pointer_up(&mut self, point: Vec2) -> Result<Option<TrackButtonId>, String> {
        if std::mem::take(&mut self.double_click) {
            let mut edited = self.project.borrow().clone();
            if let Some(key) = crate::caption_creation::insert(
                &mut edited,
                point.as_dvec2(),
                self.view,
                &self.snap_repository,
                preferences::snapshot(&self.preferences).default_visual_duration,
            ) {
                let address =
                    selection_state::item_address(&edited, key).expect("created caption address");
                self.pointer_cancelled();
                self.commit_context_edit(
                    edited,
                    "create-caption-on-double-click",
                    Some(vec![address]),
                )?;
                return Ok(None);
            }
        }

        self.pointer_dragged(point);
        if self.seeking {
            self.pointer_dragged(point);
            player_state::set_scrubbing(&self.player, false);
        }
        self.seeking = false;
        if self.drag_scrollbar(point) {
            self.horizontal_scrollbar.end_drag();
            self.vertical_scrollbar.end_drag();
            self.scrollbar_drag = None;
            return Ok(None);
        }
        if self.pressed_label.is_some() {
            return self.release_track_label();
        }
        if self.finish_item_edit(point)? {
            return Ok(None);
        }
        if let Some(group) = self.dragged_group.take() {
            if !self.drag_moved {
                if !self.selection_toggle
                    && selection_state::selected_items(&self.selection).len() > 1
                {
                    let project = self.project.borrow();
                    if let Some(address) = selection_state::item_address(&project, group.grabbed) {
                        selection_state::set_selected_item_addresses(
                            &self.selection,
                            &project,
                            vec![address.clone()],
                            Some(address),
                        );
                    }
                }
                return Ok(None);
            }
            let item_drop = crate::drop_area::root_item_drop(
                crate::drop_area::DragDropContext {
                    project: &self.project.borrow(),
                    view: self.view,
                    position: point.as_dvec2(),
                    collision_mode: ToolState::from_preferences(&preferences::snapshot(
                        &self.preferences,
                    ))
                    .drag_collision,
                },
                &group,
            );
            if let Some(item_drop) = item_drop {
                let mut edited = self.project.borrow().clone();
                if let Some(moved) = item_drop.apply(&mut edited) {
                    self.commit_context_edit(
                        edited,
                        "move-timeline-item-between-tracks",
                        Some(vec![moved]),
                    )?;
                }
                return Ok(None);
            }
            if !group.valid_drop {
                return Ok(None);
            }
            let mut project = self.project.borrow_mut();
            // Validate and apply the whole group atomically before changing the live project.
            let mut edited = project.clone();
            let focused_identity = items::item_identity(&edited, group.grabbed);
            if let Some(selected) = items::move_dragged_group(&mut edited, &group) {
                let focused =
                    focused_identity.and_then(|id| items::item_key_for_identity(&edited, id));
                edited.normalize_clip_transitions();
                project::commit_edit_checked(&edited, "move-timeline-items")
                    .map_err(|error| format!("Could not save timeline move: {error}"))?;
                *project = edited;
                let change = dragging::project_change(&group, project.duration());
                drop(project);
                selection_state::set_selected_items(&self.selection, selected, focused);
                player_state::refresh_project(&self.player, change);
            }
        }
        Ok(None)
    }

    pub fn scroll(&mut self, point: Vec2, delta: Vec2, zoom: bool) {
        self.pointer_moved(point);
        if !zoom && self.scroll_over_scrollbar(point, delta) {
            return;
        }
        self.horizontal_scrollbar.cancel_scroll();
        self.vertical_scrollbar.cancel_scroll();
        if zoom {
            if f64::from(point.x) < timeline_x() {
                return;
            }
            let minimum = min_seconds_per_pixel(frame_step_seconds(&self.project.borrow()));
            crate::math::zoom_at_x(
                &mut self.view,
                point.x.into(),
                crate::math::scroll_zoom_factor(delta.y.into()),
                minimum,
            );
            self.save_zoom();
        } else {
            self.view.scroll_seconds =
                crate::math::time_at_x(self.view, timeline_x() - f64::from(delta.x)).as_secs_f64();
            self.view.scroll_y = (self.view.scroll_y - f64::from(delta.y)).max(0.0);
        }
    }

    pub fn magnify(&mut self, point: Vec2, magnification: f64) {
        if f64::from(point.x) < timeline_x() {
            return;
        }
        self.horizontal_scrollbar.cancel_scroll();
        self.vertical_scrollbar.cancel_scroll();
        let minimum = min_seconds_per_pixel(frame_step_seconds(&self.project.borrow()));
        crate::math::zoom_at_x(
            &mut self.view,
            point.x.into(),
            crate::math::pinch_zoom_factor(magnification),
            minimum,
        );
        self.save_zoom();
    }

    fn reload_waveforms(&mut self) {
        self.waveform_cancel.store(true, Ordering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        self.waveform_cancel = Arc::clone(&cancel);
        let (sender, receiver) = mpsc::channel();
        self.waveform_updates = receiver;
        self.waveforms.clear();
        self.beats.clear();
        let (beat_sender, beat_receiver) = mpsc::channel();
        self.beat_updates = beat_receiver;
        let beat_project = self.project.borrow().clone();
        let beat_cancel = cancel.clone();
        std::thread::spawn(move || {
            audio::beat::load_project_beats_cancellable(
                &beat_project,
                || beat_cancel.load(Ordering::Relaxed),
                |key, update| {
                    let _ = beat_sender.send((key, update));
                },
            );
        });
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
}

impl Drop for Scene {
    fn drop(&mut self) {
        if self.seeking {
            player_state::set_scrubbing(&self.player, false);
        }
        self.waveform_cancel.store(true, Ordering::Relaxed);
    }
}
