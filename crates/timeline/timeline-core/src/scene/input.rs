use super::*;

#[derive(Clone, Copy)]
pub enum PointerButton {
    Primary,
    Middle,
}

pub enum Event {
    Modifiers(TimelineModifiers),
    Motion {
        point: Vec2,
        modifiers: TimelineModifiers,
    },
    RelativeMotion {
        delta: Vec2,
    },
    Press {
        point: Vec2,
        button: PointerButton,
        double: bool,
        modifiers: TimelineModifiers,
    },
    Release {
        point: Vec2,
        button: PointerButton,
        modifiers: TimelineModifiers,
    },
    Leave,
    Scroll(TimelineScrollEvent),
    Cancel,
}

pub struct Requests {
    pub pause_playback: bool,
    pub track_add: Option<TrackAddMenuRequest>,
    pub audio_record: Option<TrackKey>,
    pub video_record: Option<TrackKey>,
}

#[derive(Clone, Copy)]
pub struct PointerState {
    pub position: Option<Vec2>,
    pub modifiers: TimelineModifiers,
    pub capture_requested: bool,
    pub software_position: Option<Vec2>,
}

impl Scene {
    pub fn view(&self) -> TimelineViewState {
        self.view
    }

    pub fn pointer_state(&self) -> PointerState {
        PointerState {
            position: self.pointer_pos,
            modifiers: self.modifiers,
            capture_requested: self.middle_down && self.view.drag_mode == DragMode::MiddlePan,
            software_position: self.software_cursor.as_ref().map(|cursor| cursor.position),
        }
    }

    pub fn begin_relative_pointer(
        &mut self,
        position: Vec2,
        cursor: shrimply_skia_adw_core::cursor::SoftwareCursor,
    ) {
        self.software_cursor = Some(TimelineSoftwareCursor { position, cursor });
    }

    fn relative_motion(&mut self, delta: Vec2) {
        let Some(cursor) = self.software_cursor.as_mut() else {
            return;
        };
        let bounds = Rect::from_min_max(
            vec2(timeline_x() as f32, 0.0),
            vec2(
                (timeline_x() + timeline_width(f64::from(self.viewport.width()))) as f32,
                self.viewport.max.y,
            ),
        );
        let unwrapped = cursor.position + delta;
        let wrapped = bounds.wrap_point(unwrapped);
        let wrap_offset = wrapped - unwrapped;
        if self.view.drag_mode == DragMode::MiddlePan {
            self.view.drag_start_x += f64::from(wrap_offset.x);
            self.view.drag_start_y += f64::from(wrap_offset.y);
        }
        self.pointer_pos = Some(wrapped);
        cursor.position = wrapped;
    }

    pub fn end_relative_pointer(&mut self) -> Option<Vec2> {
        self.software_cursor.take().map(|cursor| cursor.position)
    }

    /// Queue toolkit input for the next update. Native hosts never implement gesture rules.
    pub fn event(&mut self, event: Event) {
        self.sync_revision();
        match event {
            Event::Modifiers(modifiers) => self.modifiers = modifiers,
            Event::Motion { point, modifiers } => {
                self.pointer_pos = Some(point);
                self.modifiers = modifiers;
            }
            Event::RelativeMotion { delta } => self.relative_motion(delta),
            Event::Press {
                point,
                button,
                double,
                modifiers,
            } => {
                self.modifiers = modifiers;
                self.pointer_pos = Some(point);
                self.pointer_press_origin = Some(point);
                self.pointer_release_pos = None;
                match button {
                    PointerButton::Primary => {
                        self.suppress_double_click_selection = double;
                        self.primary_pressed = true;
                        self.primary_down = true;
                    }
                    PointerButton::Middle => {
                        self.middle_pressed = true;
                        self.middle_down = true;
                    }
                }
            }
            Event::Release {
                point,
                button,
                modifiers,
            } => {
                self.modifiers = modifiers;
                self.pointer_pos = Some(point);
                self.pointer_release_pos = Some(point);
                match button {
                    PointerButton::Primary => {
                        self.primary_released = true;
                        self.primary_down = false;
                    }
                    PointerButton::Middle => {
                        self.middle_released = true;
                        self.middle_down = false;
                    }
                }
            }
            Event::Leave => self.pointer_exited(),
            Event::Scroll(scroll) => self.pending_scrolls.push(scroll),
            Event::Cancel => self.pointer_cancelled(),
        }
    }

    pub fn draw(&mut self, canvas: &skia_safe::Canvas, size: Vec2) {
        self.draw_frame(
            canvas,
            size,
            Frame {
                before_seek: None,
                playback_performance: &playback_performance::Snapshot::default(),
                accent_color: Color::BLUE3,
                active_audio_recording_key: None,
                active_video_recording_key: None,
                live_recording: None,
                live_video_recording: None,
            },
        );
        self.apply_internal_actions();
    }

    pub fn draw_frame(&mut self, canvas: &skia_safe::Canvas, size: Vec2, mut frame: Frame<'_>) {
        self.suspended = false;
        self.sync_revision();
        self.viewport = Rect::from_min_size(Vec2::ZERO, size);
        self.apply_preferences(&preferences::snapshot(&self.preferences));
        self.update_media();
        self.track_controls_animating = false;
        let before_seek = frame.before_seek.take();
        frame::draw(self, &TimelinePainter::new(canvas), size, frame);
        self.flush_seek(before_seek);
        self.finish_pointer_frame();
    }

    fn flush_seek(&mut self, before_seek: Option<&mut dyn FnMut(Time)>) {
        if let Some(position) = self.pending_seek.take() {
            if let Some(before_seek) = before_seek {
                before_seek(position);
            }
            player_state::seek_time(&self.player, position);
        }
    }

    /// Poll background results without requiring a live graphics context.
    pub fn update_media(&mut self) -> bool {
        if self.suspended {
            return false;
        }
        let mut changed = self.poll_drop_preview();
        if self
            .media_refresh
            .waveforms
            .get()
            .is_some_and(|requested| requested.elapsed() >= WAVEFORM_RELOAD_DELAY)
        {
            self.media_refresh.waveforms.set(None);
            self.reload_waveforms();
            changed = true;
        }
        if self.media_refresh.beats.replace(false) {
            self.reload_beats();
            changed = true;
        }
        while let Ok((key, update)) = self.waveform_updates.try_recv() {
            waveform::apply_update(&mut self.waveforms, key, update);
            changed = true;
        }
        while let Ok((key, update)) = self.beat_updates.try_recv() {
            audio::beat::apply_update(&mut self.beats, key, update);
            changed = true;
        }
        changed
    }

    /// Detach from a native surface while retaining view state for a later realization.
    pub fn suspend(&mut self) {
        self.pointer_cancelled();
        self.suspended = true;
        self.clear_drop_preview();
        self.text_drop_preview = None;
        self.waveform_cancel.store(true, Ordering::Relaxed);
        self.beat_cancel.store(true, Ordering::Relaxed);
        self.waveform_updates = mpsc::channel().1;
        self.beat_updates = mpsc::channel().1;
        self.media_refresh
            .waveforms
            .set(Some(Instant::now() - WAVEFORM_RELOAD_DELAY));
        self.media_refresh.beats.set(true);
    }

    fn sync_revision(&mut self) {
        let revision = player_state::snapshot(&self.player).revision;
        if self.revision != revision {
            self.revision = revision;
            self.pointer_cancelled();
        }
    }

    pub fn animating(&self) -> bool {
        !self.pending_scrolls.is_empty()
            || self.track_controls_animating
            || self.horizontal_scrollbar.animating()
            || self.vertical_scrollbar.animating()
            || self.overscroll.is_some_and(|overscroll| {
                shrimply_skia_adw_core::overshoot_distance(
                    overscroll.distance,
                    overscroll.started_at.elapsed(),
                ) > shrimply_skia_adw_core::OVERSHOOT_VISIBLE_DISTANCE
            })
    }

    pub(super) fn pointer_in_viewport(&self, point: Vec2) -> bool {
        self.viewport.contains(point) && point.cmplt(self.viewport.max).all()
    }

    pub fn cursor_at(&self, point: Vec2) -> TimelineCursor {
        items::timeline_cursor(
            &self.project.borrow(),
            items::CursorState {
                view: self.view,
                drag_mode: self.view.drag_mode,
                resize_edge: self.resize_drag.as_ref().map(|resize| resize.edge),
                folded_drag: self.folded_drag.as_ref().map(|drag| drag.kind),
                item_transition_drag: self.transition_drag.is_some(),
                clip_transition_center_drag: self
                    .clip_transition_drag
                    .as_ref()
                    .map(|drag| drag.center_resize),
            },
            point.x.into(),
            point.y.into(),
        )
    }

    pub fn pointer_cursor(&self) -> TimelineCursor {
        self.pointer_pos
            .map_or(TimelineCursor::Default, |point| self.cursor_at(point))
    }

    pub fn pointer_moved(&mut self, point: Vec2) {
        self.pointer_pos = Some(point);
        self.update_input();
    }

    pub fn pointer_exited(&mut self) {
        self.pointer_pos = None;
        self.cut_preview = None;
        if let Some(id) = self.hovered_track_button.take() {
            self.track_buttons
                .entry(id)
                .or_default()
                .event(shrimply_skia_adw_core::button::Event::PointerLeft);
        }
    }

    pub fn pointer_cancelled(&mut self) {
        self.pending_seek = None;
        if self.view.drag_mode == DragMode::Seek {
            player_state::set_scrubbing(&self.player, false);
        }
        self.horizontal_scrollbar.end_drag();
        self.vertical_scrollbar.end_drag();
        self.horizontal_scrollbar.cancel_scroll();
        self.vertical_scrollbar.cancel_scroll();
        for id in [
            self.pressed_track_button.take(),
            self.hovered_track_button.take(),
        ]
        .into_iter()
        .flatten()
        {
            self.track_buttons
                .entry(id)
                .or_default()
                .event(shrimply_skia_adw_core::button::Event::Cancelled);
        }
        self.pressed_track_selection = None;
        self.dragged_group = None;
        self.folded_drag = None;
        self.resize_drag = None;
        self.transition_drag = None;
        self.clip_transition_drag = None;
        self.cut_preview = None;
        self.view.selection = None;
        self.view.drag_mode = DragMode::None;
        self.view.drag_moved = false;
        self.primary_down = false;
        self.middle_down = false;
        self.suppress_double_click_selection = false;
        self.finish_pointer_frame();
    }

    pub fn double_click_down(&mut self, point: Vec2, toggle: bool, extend: bool) {
        self.pointer_down_with_count(point, toggle, extend, true);
    }

    pub fn pointer_down(&mut self, point: Vec2, toggle: bool, extend: bool) {
        self.pointer_down_with_count(point, toggle, extend, false);
    }

    fn pointer_down_with_count(&mut self, point: Vec2, toggle: bool, extend: bool, double: bool) {
        self.pointer_cancelled();
        self.event(Event::Press {
            point,
            button: PointerButton::Primary,
            double,
            modifiers: TimelineModifiers {
                ctrl: toggle,
                shift: extend,
            },
        });
        self.update_input();
        self.apply_internal_actions();
    }

    pub fn pointer_dragged(&mut self, point: Vec2) {
        self.pointer_pos = Some(point);
        self.update_input();
    }

    pub fn pointer_up(&mut self, point: Vec2) -> Result<Option<TrackButtonId>, String> {
        self.event(Event::Release {
            point,
            button: PointerButton::Primary,
            modifiers: self.modifiers,
        });
        self.update_input();
        self.apply_internal_actions();
        Ok(self
            .pending_track_add_menu
            .take()
            .map(|request| (request.key, TrackLabelAction::Add))
            .or_else(|| {
                self.pending_audio_record
                    .take()
                    .map(|key| (key, TrackLabelAction::AudioRecord))
            })
            .or_else(|| {
                self.pending_video_record
                    .take()
                    .map(|key| (key, TrackLabelAction::VideoRecord))
            }))
    }

    fn apply_internal_actions(&mut self) {
        self.flush_seek(None);
        if std::mem::take(&mut self.pending_pause_playback) {
            player_state::set_playing(&self.player, false);
        }
        if let Some(key) = self.pending_track_toggle.take() {
            let mut project = self.project.borrow_mut();
            if track_controls::toggle_track_enabled(&mut project, key) {
                project::commit_edit(&project, "toggle-track-enabled");
                let change = ProjectChange {
                    duration: Some(project.duration()),
                    audio: key.kind == TrackKind::Audio,
                    video: key.kind == TrackKind::Video,
                    captions: key.kind == TrackKind::Caption,
                    inspector: true,
                    ..Default::default()
                };
                drop(project);
                player_state::refresh_project(&self.player, change);
            }
        }
        if let Some(path) = self.pending_sequence_toggle.take() {
            folded_sequence::toggle_sequence(&self.project, &self.selection, path);
        }
    }

    /// Apply completed shared edits and return only operations requiring a native service.
    pub fn take_requests(&mut self) -> Requests {
        let pause_playback = std::mem::take(&mut self.pending_pause_playback);
        self.apply_internal_actions();
        Requests {
            pause_playback,
            track_add: self.pending_track_add_menu.take(),
            audio_record: self.pending_audio_record.take(),
            video_record: self.pending_video_record.take(),
        }
    }

    pub(super) fn update_input(&mut self) {
        self.sync_revision();
        self.apply_preferences(&preferences::snapshot(&self.preferences));
        let project = self.project.clone();
        let player = self.player.clone();
        let selection = self.selection.clone();
        let (duration, step, track_height) = {
            let project = project.borrow();
            let player = player_state::snapshot(&player);
            self.snap_repository = crate::snapping::repository(
                &project,
                crate::snapping::Request {
                    folded_drag: self.folded_drag.as_ref(),
                    dragged_group: self.dragged_group.as_ref(),
                    resize_drag: self.resize_drag.as_ref(),
                    beats: if self.beat_grid_enabled {
                        beat_grid::snap_targets(&project, &self.beats, self.view)
                    } else {
                        Vec::new()
                    },
                    playhead: player.position,
                    distance: self
                        .snap_enabled
                        .then(|| crate::math::snap_distance(self.view, self.snap_radius_px)),
                },
            );
            let virtual_tracks = drawing::active_virtual_tracks(
                self.dragged_group.as_ref(),
                self.import_preview.as_ref(),
            );
            (
                folded_sequence::expanded_timeline_end(&project)
                    .max(player.duration)
                    .max(player.position)
                    .max(Time::from_seconds(1))
                    .as_secs_f64(),
                frame_step_seconds(&project),
                timeline_track_content_height(&project, &virtual_tracks),
            )
        };
        let width = f64::from(self.viewport.width());
        let height = f64::from(self.viewport.height());
        pointer::handle_timeline_input(
            &project,
            &player,
            &selection,
            self,
            width,
            timeline_width(width),
            height,
            track_height,
            duration,
            step,
        );
        self.finish_pointer_frame();
    }

    /// Deltas use content motion coordinates; native wheel direction is translated by the host.
    pub fn scroll(&mut self, point: Vec2, delta: Vec2, zoom: bool) {
        self.pointer_pos = Some(point);
        self.pending_scrolls.push(TimelineScrollEvent {
            delta: -delta,
            ctrl: zoom,
            pointer: Some(point),
        });
        self.update_input();
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
}
