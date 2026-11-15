use super::*;
use shrimply_project_document::project::{
    AudioItem, AudioSource, ItemAddress, ItemKind, ProjectItem, Time, TrackAddress, TrackMut,
    TrackRef, Transform, VideoItemContent,
};
impl InspectorController {
    pub(super) fn apply_generated_video(
        &self,
        target: &InspectorTarget,
        settings: &VideoGenerationSettings,
        generated: worker::Generated,
    ) -> Result<(), String> {
        let InspectorTarget::Item(address @ ItemAddress::Video { .. }) = target else {
            return Err("Video-generation item is no longer available".into());
        };
        let mut candidate = self.project.borrow().clone();
        let item = candidate
            .video_item(address)
            .filter(|item| item.video_generation.as_deref() == Some(settings))
            .ok_or("Video-generation settings changed while generating")?;
        let result = generated.metadata;
        if !shrimply_math_core::fraction_is_finite(result.duration)
            || result.duration <= shrimply_math_core::FRACTION_ZERO
            || result.video_streams != 1
            || result.audio_streams > 1
            || result.width == 0
            || result.height == 0
        {
            return Err("Video generation produced invalid media metadata".into());
        }
        let source_duration = Time {
            seconds: result.duration,
        };
        let start = item.start;
        let item_id = item.id;
        let previous_file = item.file.path().to_owned();
        let had_output = !previous_file.as_os_str().is_empty();
        let previous_group = item.group_id;
        let next_video_start = match candidate.track(&address.track()) {
            Some(TrackRef::Video(track)) => track
                .items
                .iter()
                .filter_map(|item| (item.id != item_id && item.start > start).then_some(item.start))
                .min(),
            _ => None,
        };
        let scope = candidate
            .track_scope(&address.track())
            .ok_or("Video-generation sequence is no longer available")?;
        let tracks = candidate
            .audio_tracks_for_scope(&scope)
            .ok_or("Video-generation audio sequence is no longer available")?;
        let paired = previous_group.and_then(|group| {
            tracks.iter().find_map(|track| {
                track.items.iter().find_map(|item| {
                    (had_output
                        && item.group_id == Some(group)
                        && matches!(item.source, AudioSource::Media)
                        && item.file.path() == previous_file)
                        .then_some((track.id, item.id))
                })
            })
        });
        let audio_changed = result.audio_streams > 0 || paired.is_some();
        let audio_path = if audio_changed {
            Some(
                candidate
                    .sequence_path_for_scope(ItemKind::Audio, &scope)
                    .ok_or("Video-generation sequence has no audio presenter")?,
            )
        } else {
            None
        };
        let audio_track = paired
            .map(|(track, _)| track)
            .into_iter()
            .chain(tracks.iter().map(|track| track.id))
            .find(|track_id| {
                tracks
                    .iter()
                    .find(|track| track.id == *track_id)
                    .is_some_and(|track| {
                        track.items.iter().all(|item| {
                            paired.is_some_and(|(_, id)| item.id == id)
                                || item.end <= start
                                || item.start > start
                        })
                    })
            });
        let next_audio_start = audio_track
            .and_then(|id| tracks.iter().find(|track| track.id == id))
            .and_then(|track| {
                track
                    .items
                    .iter()
                    .filter_map(|item| {
                        (paired.is_none_or(|(_, id)| item.id != id) && item.start > start)
                            .then_some(item.start)
                    })
                    .min()
            });
        let mut end = start
            .saturating_add(source_duration)
            .snapped(candidate.frame_step());
        if let Some(next) = next_video_start {
            end = end.min(next)
        }
        if result.audio_streams > 0
            && let Some(next) = next_audio_start
        {
            end = end.min(next)
        }
        if end <= start {
            return Err("Generated video has no usable timeline duration".into());
        }
        let group =
            previous_group.unwrap_or_else(|| shrimply_timeline_edit::next_group_id(&candidate));
        let previous_audio = if let Some((track, id)) = paired {
            let track = TrackAddress::Audio {
                sequence_path: audio_path.as_ref().expect("paired audio presenter").clone(),
                track_id: track,
            };
            match candidate.track_mut(&track) {
                Some(TrackMut::Audio(track)) => track
                    .items
                    .iter()
                    .position(|item| item.id == id)
                    .map(|index| track.items.remove(index)),
                _ => None,
            }
        } else {
            None
        };
        if result.audio_streams > 0 {
            let audio_path = audio_path.as_ref().expect("generated audio presenter");
            let mut audio = previous_audio
                .unwrap_or_else(|| AudioItem::builder(start, end).group_id(Some(group)).build());
            audio.start = start;
            audio.end = end;
            audio.time_offset = Time::ZERO;
            audio.source_duration = source_duration;
            audio.playback_speed = shrimply_project_document::project::default_playback_speed();
            audio.repeat_strategy = shrimply_project_document::project::RepeatStrategy::Hold;
            audio.group_id = Some(group);
            audio.source = AudioSource::Media;
            audio.track_id = 0;
            audio.file = generated.path().into();
            let inserted = if let Some(track) = audio_track {
                candidate.insert_item(
                    &TrackAddress::Audio {
                        sequence_path: audio_path.clone(),
                        track_id: track,
                    },
                    ProjectItem::Audio(Box::new(audio)),
                )
            } else {
                candidate.insert_item_on_new_track(audio_path, ProjectItem::Audio(Box::new(audio)))
            };
            if inserted.is_none() {
                return Err("Could not insert generated audio into its sequence".into());
            }
        }
        let transform = Transform::natural_size(candidate.canvas_size, result.width, result.height);
        let item = candidate
            .video_item_mut(address)
            .expect("validated generated video item");
        item.end = end;
        item.time_offset = Time::ZERO;
        item.source_duration = source_duration;
        item.playback_speed = shrimply_project_document::project::default_playback_speed();
        item.playback_fps = result.frame_rate;
        item.repeat_strategy = shrimply_project_document::project::RepeatStrategy::Hold;
        if !had_output {
            item.transform = transform.clone();
            item.default_transform = Some(transform)
        }
        item.source_width = result.width;
        item.source_height = result.height;
        item.group_id = Some(group);
        item.content = VideoItemContent::Media;
        item.track_id = 0;
        item.file = generated.path().into();
        shrimply_project_document::project::commit_edit_checked(&candidate, "generate-video")?;
        generated.keep();
        let duration = candidate.duration();
        *self.project.borrow_mut() = candidate;
        shrimply_editor_state::player_state::refresh_project(
            &self.player_state,
            shrimply_editor_state::player_state::ProjectChange {
                duration: Some(duration),
                video: true,
                audio: audio_changed,
                audio_beats: audio_changed,
                audio_waveforms: audio_changed,
                inspector: true,
                ..Default::default()
            },
        );
        Ok(())
    }
}
