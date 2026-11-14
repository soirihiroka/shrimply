use crate::{
    TrackKey,
    draw_state::LiveRecordingDraw,
    items::{self, ItemKey, TrackKind},
    metrics::RECORDING_DURATION_HEADROOM_SECONDS,
    project::{
        self, AudioItem, Project, RepeatStrategy, Time, Transform, VideoItem, VideoItemContent,
        VideoSampleMethod, default_playback_speed,
    },
    scene::Scene,
};
use shrimply_player_core::{self as player_state, ProjectChange, SharedPlayerState};
use shrimply_property_model::timeline_value::{TimelineBool, TimelineValue};
use std::{cell::RefCell, path::PathBuf};

pub struct AudioRecording {
    pub key: TrackKey,
    pub start: Time,
    recording: shrimply_audio_engine::recording::MicRecording,
}

impl AudioRecording {
    pub fn start(
        key: TrackKey,
        project: &Project,
        player: &SharedPlayerState,
    ) -> Result<Self, String> {
        if project.audio_tracks.get(key.track_index).is_none() {
            return Err("Audio track no longer exists".into());
        }
        let start = player_state::snapshot(player)
            .position
            .snapped(project.frame_step());
        let recording = shrimply_audio_engine::recording::MicRecording::start()?;
        ensure_duration(player, start);
        player_state::set_playing(player, true);
        Ok(Self {
            key,
            start,
            recording,
        })
    }

    pub fn finish(
        self,
        project: &RefCell<Project>,
        player: &SharedPlayerState,
    ) -> Result<(), String> {
        let finished = self.recording.finish()?;
        if finished.duration <= Time::ZERO {
            return Ok(());
        }
        let mut project = project.borrow_mut();
        let end = self
            .start
            .saturating_add(finished.duration)
            .snapped(project.frame_step());
        if end <= self.start {
            let _ = std::fs::remove_file(&finished.path);
            return Err("The recording is shorter than one project frame".into());
        }
        let Some(track) = project.audio_tracks.get_mut(self.key.track_index) else {
            let _ = std::fs::remove_file(&finished.path);
            return Err("Audio track no longer exists".into());
        };
        let item = AudioItem::builder(self.start, end)
            .source_duration(finished.duration)
            .file(finished.path)
            .build();
        items::overwrite_items(&mut track.items, self.start, end);
        items::insert_sorted(&mut track.items, item);
        crate::project::commit_edit(&project, "record-audio");
        let duration = project.duration();
        drop(project);
        player_state::refresh_project(
            player,
            ProjectChange {
                duration: Some(duration),
                audio: true,
                audio_waveforms: true,
                inspector: true,
                ..Default::default()
            },
        );
        Ok(())
    }

    pub fn maintain_duration(&self, player: &SharedPlayerState) {
        ensure_duration(player, player_state::snapshot(player).position);
    }

    pub fn draw(&self, waveform_chunks_per_second: u32) -> Option<LiveRecordingDraw> {
        let snapshot = self.recording.snapshot();
        const CHANNELS: usize = 2;
        let frames = snapshot.samples.len() / CHANNELS;
        if frames == 0 {
            return None;
        }
        let duration = Time::from_fraction(
            i64::try_from(frames).ok()?,
            i64::from(snapshot.sample_rate.max(1)),
        );
        Some(LiveRecordingDraw {
            key: self.key,
            item: AudioItem::builder(self.start, self.start.saturating_add(duration))
                .source_duration(duration)
                .file(std::path::PathBuf::from("live-recording.opus"))
                .build(),
            waveform: shrimply_audio_engine::waveform::from_stereo_samples(
                &snapshot.samples,
                snapshot.sample_rate,
                waveform_chunks_per_second,
            ),
        })
    }
}

fn ensure_duration(player: &SharedPlayerState, position: Time) {
    let duration = position.saturating_add(Time::from_seconds(RECORDING_DURATION_HEADROOM_SECONDS));
    if duration > player_state::snapshot(player).duration {
        player_state::set_duration(player, duration);
    }
}

#[derive(Clone, Copy, Debug)]
pub enum VideoRecordingCommand {
    Start { fps: crate::Fraction },
    Stop,
}

pub enum VideoRecordingEvent {
    Ready { width: u32, height: u32 },
    Cancelled,
    Finished(Result<FinishedVideoRecording, String>),
}

pub struct FinishedVideoRecording {
    path: Option<PathBuf>,
    pub duration: Time,
    pub width: u32,
    pub height: u32,
    pub alpha_track_id: Option<u32>,
}

impl FinishedVideoRecording {
    pub fn new(
        path: PathBuf,
        duration: Time,
        width: u32,
        height: u32,
        alpha_track_id: Option<u32>,
    ) -> Self {
        Self {
            path: Some(path),
            duration,
            width,
            height,
            alpha_track_id,
        }
    }

    fn path(&self) -> &std::path::Path {
        self.path.as_deref().expect("recording file is owned")
    }

    fn keep(mut self) {
        self.path.take();
    }
}

impl Drop for FinishedVideoRecording {
    fn drop(&mut self) {
        if let Some(path) = self.path.take()
            && let Err(error) = std::fs::remove_file(&path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(path = %path.display(), %error, "Could not remove abandoned screen recording");
        }
    }
}

pub(crate) struct ActiveVideoRecording {
    track_id: uuid::Uuid,
    start: Time,
    stop_at: Option<Time>,
    ready: bool,
    stopping: bool,
}

impl Scene {
    pub fn toggle_video_recording(&mut self, key: TrackKey) -> Result<(), String> {
        if key.kind != TrackKind::Video {
            return Err("Screen recording requires a video track".into());
        }
        if self.active_video_recording.is_some() {
            self.stop_video_recording();
            return Ok(());
        }
        let project = self.project.borrow();
        let start = player_state::snapshot(&self.player)
            .position
            .snapped(project.frame_step());
        let track = project
            .video_tracks
            .get(key.track_index)
            .ok_or("Video track no longer exists")?;
        if track
            .items
            .iter()
            .any(|item| item.start <= start && start < item.end)
        {
            return Err("The playhead is inside an existing item on this video track".into());
        }
        let track_id = track.id;
        let stop_at = track
            .items
            .iter()
            .find(|item| item.start > start)
            .map(|item| item.start);
        let fps = project.fps;
        drop(project);
        player_state::set_playing(&self.player, false);
        self.active_video_recording = Some(ActiveVideoRecording {
            track_id,
            start,
            stop_at,
            ready: false,
            stopping: false,
        });
        self.video_recording_commands
            .push_back(VideoRecordingCommand::Start { fps });
        Ok(())
    }

    pub fn take_video_recording_command(&mut self) -> Option<VideoRecordingCommand> {
        self.video_recording_commands.pop_front()
    }

    pub fn handle_video_recording_event(
        &mut self,
        event: VideoRecordingEvent,
    ) -> Result<(), String> {
        match event {
            VideoRecordingEvent::Ready { width, height } => {
                if width == 0 || height == 0 {
                    self.stop_video_recording();
                    return Err("Screen capture returned an empty frame size".into());
                }
                let Some(active) = self.active_video_recording.as_ref() else {
                    return Err("Screen capture became ready after it was cancelled".into());
                };
                if active.stopping {
                    return Ok(());
                }
                let track_id = active.track_id;
                let start = active.start;
                let project = self.project.borrow();
                let occupied = project
                    .video_tracks
                    .iter()
                    .find(|track| track.id == track_id)
                    .is_none_or(|track| {
                        track
                            .items
                            .iter()
                            .any(|item| item.start <= start && start < item.end)
                    });
                drop(project);
                if occupied {
                    self.stop_video_recording();
                    return Err(
                        "The recording position became occupied while selecting a screen or application"
                            .into(),
                    );
                }
                self.active_video_recording
                    .as_mut()
                    .expect("active screen recording exists")
                    .ready = true;
                ensure_duration(&self.player, start);
                player_state::set_playing(&self.player, true);
                Ok(())
            }
            VideoRecordingEvent::Cancelled => {
                if self
                    .active_video_recording
                    .take()
                    .is_some_and(|active| active.ready)
                {
                    player_state::set_playing(&self.player, false);
                }
                Ok(())
            }
            VideoRecordingEvent::Finished(result) => {
                let Some(active) = self.active_video_recording.take() else {
                    if let Ok(finished) = result {
                        drop(finished);
                    }
                    return Ok(());
                };
                if active.ready {
                    player_state::set_playing(&self.player, false);
                }
                let finished = result?;
                self.finish_video_recording(active, finished)
            }
        }
    }

    pub(crate) fn maintain_video_recording(&mut self) -> bool {
        let Some(active) = self.active_video_recording.as_ref() else {
            return false;
        };
        let snapshot = player_state::snapshot(&self.player);
        if !active.ready || active.stopping {
            return false;
        }
        if !self
            .project
            .borrow()
            .video_tracks
            .iter()
            .any(|track| track.id == active.track_id)
        {
            self.stop_video_recording();
            return true;
        }
        if !snapshot.playing
            || active
                .stop_at
                .is_some_and(|stop_at| snapshot.position >= stop_at)
        {
            self.stop_video_recording();
            return true;
        }
        ensure_duration(&self.player, snapshot.position);
        true
    }

    pub(crate) fn stop_video_recording_before_backward_seek(&mut self, position: Time) {
        if self.active_video_recording.is_some()
            && position < player_state::snapshot(&self.player).position
        {
            self.stop_video_recording();
        }
    }

    pub(crate) fn video_recording_draw(
        &self,
    ) -> (
        Option<TrackKey>,
        Option<crate::draw_state::LiveVideoRecordingDraw>,
    ) {
        let Some(active) = self.active_video_recording.as_ref() else {
            return (None, None);
        };
        let Some(track_index) = self
            .project
            .borrow()
            .video_tracks
            .iter()
            .position(|track| track.id == active.track_id)
        else {
            return (None, None);
        };
        let key = TrackKey {
            kind: TrackKind::Video,
            track_index,
        };
        let end = active.stop_at.map_or_else(
            || player_state::current_time(&self.player),
            |stop_at| player_state::current_time(&self.player).min(stop_at),
        );
        (
            Some(key),
            (active.ready && end > active.start).then_some(
                crate::draw_state::LiveVideoRecordingDraw {
                    key,
                    start: active.start,
                    end,
                },
            ),
        )
    }

    pub(crate) fn stop_video_recording(&mut self) {
        let Some(active) = self.active_video_recording.as_mut() else {
            return;
        };
        if active.stopping {
            return;
        }
        active.stopping = true;
        if active.ready {
            player_state::set_playing(&self.player, false);
        }
        self.video_recording_commands
            .push_back(VideoRecordingCommand::Stop);
    }

    fn finish_video_recording(
        &mut self,
        active: ActiveVideoRecording,
        finished: FinishedVideoRecording,
    ) -> Result<(), String> {
        if !active.ready || finished.duration <= Time::ZERO {
            return Ok(());
        }
        let mut candidate = self.project.borrow().clone();
        let track_index = candidate
            .video_tracks
            .iter()
            .position(|track| track.id == active.track_id)
            .ok_or("Video track no longer exists")?;
        let recorded_end = active
            .start
            .saturating_add(finished.duration)
            .snapped(candidate.frame_step());
        let end = active
            .stop_at
            .map_or(recorded_end, |stop_at| recorded_end.min(stop_at));
        if end <= active.start {
            return Err("The recording has no free space on the video track".into());
        }
        let track = candidate
            .video_tracks
            .get(track_index)
            .ok_or("Video track no longer exists")?;
        if crate::timeline_search::collides(&track.items, active.start, end) {
            return Err("The recording position overlaps another video item".into());
        }
        let transform =
            Transform::natural_size(candidate.canvas_size, finished.width, finished.height);
        let item = VideoItem {
            id: uuid::Uuid::new_v4(),
            start: active.start,
            end,
            time_offset: Time::ZERO,
            source_duration: finished.duration,
            playback_speed: default_playback_speed(),
            playback_fps: project::native_playback_fps(),
            repeat_strategy: RepeatStrategy::Hold,
            stabilize_video: false,
            stabilization_method: Default::default(),
            stabilization_crop_ratio: project::default_video_stabilization_crop_ratio(),
            stabilization_first_derivative_weight:
                project::default_video_stabilization_first_derivative_weight(),
            stabilization_second_derivative_weight:
                project::default_video_stabilization_second_derivative_weight(),
            stabilization_third_derivative_weight:
                project::default_video_stabilization_third_derivative_weight(),
            mesh_flow_rows: project::default_mesh_flow_rows(),
            mesh_flow_columns: project::default_mesh_flow_columns(),
            mesh_flow_smoothing_radius: project::default_mesh_flow_smoothing_radius(),
            mesh_flow_iterations: project::default_mesh_flow_iterations(),
            mesh_flow_adaptive_weights: Default::default(),
            animation_time_offset: Time::ZERO,
            motion_blur: Default::default(),
            transform: transform.clone(),
            modifiers: Vec::new(),
            sample_method: TimelineValue::new_const(VideoSampleMethod::Xbrz),
            skia_drawing_strategy: Default::default(),
            compositing: Default::default(),
            visibility: TimelineValue::new_const(TimelineBool::True),
            alpha_mask_video: finished.alpha_track_id,
            transitions: Default::default(),
            svg_color_overrides: Vec::new(),
            source_width: finished.width,
            source_height: finished.height,
            default_transform: Some(transform),
            content: VideoItemContent::Media,
            video_generation: None,
            group_id: None,
            render_canvas_size: None,
            track_id: 0,
            file: finished.path().to_path_buf().into(),
        };
        let item_index = items::insert_sorted(&mut candidate.video_tracks[track_index].items, item);
        project::commit_edit_checked(&candidate, "record-video")?;
        let duration = candidate.duration();
        *self.project.borrow_mut() = candidate;
        finished.keep();
        let key = ItemKey {
            kind: TrackKind::Video,
            track_index,
            item_index,
        };
        crate::scene::pointer::set_timeline_selection(
            &self.project.borrow(),
            &self.selection,
            vec![key],
            Some(key),
        );
        player_state::refresh_project(
            &self.player,
            ProjectChange {
                duration: Some(duration),
                video: true,
                inspector: true,
                ..ProjectChange::default()
            },
        );
        Ok(())
    }
}
