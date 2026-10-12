use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, TryRecvError},
};

use shrimply_editor_state::player_state::{self, ProjectChange};
use shrimply_transcription::{
    SAMPLE_RATE, TranscribedSegment, prepare_transcription_chunks, sanitize_transcribed_segments,
};

use crate::{
    audio_selection::SelectedAudioProject,
    project::{CaptionItem, CaptionTrack, Project, Time},
    scene::{Scene, selected_timeline_items, selected_timeline_tracks},
};

#[derive(Clone, Copy)]
pub enum SnapSource {
    Audio,
    Video,
    AudioAndVideo,
}

#[derive(Clone, Copy)]
pub struct Options {
    pub chunked: bool,
    pub snap_source: SnapSource,
    pub snap_tolerance: Time,
    pub continue_threshold: Time,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            chunked: true,
            snap_source: SnapSource::Audio,
            snap_tolerance: Time::from_seconds(1),
            continue_threshold: Time::from_seconds(2),
        }
    }
}

impl Options {
    fn validate(self) -> Result<Self, String> {
        if self.snap_tolerance < Time::ZERO || self.snap_tolerance > Time::from_seconds(2) {
            return Err("Snap tolerance must be between 0 and 2 seconds".into());
        }
        if self.continue_threshold < Time::ZERO || self.continue_threshold > Time::from_seconds(10)
        {
            return Err("Continue cut threshold must be between 0 and 10 seconds".into());
        }
        Ok(self)
    }
}

pub enum Update {
    Progress(String),
    Finished { captions: usize },
    Cancelled,
    Failed(String),
}

#[derive(Clone)]
pub struct Handle {
    cancelled: Arc<AtomicBool>,
    active_request: Arc<Mutex<Option<shrimply_compute_client::CancellationToken>>>,
}

impl Handle {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
        if let Some(request) = self
            .active_request
            .lock()
            .expect("transcription active request poisoned")
            .as_ref()
        {
            request.cancel();
        }
    }
}

pub(crate) struct Job {
    receiver: mpsc::Receiver<Message>,
    handle: Handle,
    revision: u64,
    cut_points: Vec<Time>,
    snap_tolerance: Time,
}

impl Drop for Job {
    fn drop(&mut self) {
        self.handle.cancel();
    }
}

enum Message {
    Progress(String),
    Done(Result<Vec<TranscribedSegment>, String>),
    Cancelled,
}

pub fn models(server_url: &str) -> Result<Vec<String>, String> {
    let models = shrimply_compute_client::server_status(server_url)?
        .capabilities
        .into_iter()
        .filter_map(|capability| capability.strip_prefix("stt:").map(str::to_owned))
        .filter(|model| !model.is_empty())
        .collect::<Vec<_>>();
    if models.is_empty() {
        Err("The server does not advertise any speech-to-text models".into())
    } else {
        Ok(models)
    }
}

impl Scene {
    pub fn transcription_chunk_count(&self, options: Options) -> Result<usize, String> {
        let options = options.validate()?;
        let selection = crate::audio_selection::selected_audio_project(
            &self.project.borrow(),
            &selected_timeline_items(&self.selection),
            &selected_timeline_tracks(&self.selection),
        )
        .ok_or("No audio item is selected")?;
        Ok(plan(selection, options).1.len())
    }

    pub fn start_transcription(
        &mut self,
        options: Options,
        server_url: String,
        model: String,
    ) -> Result<Handle, String> {
        let options = options.validate()?;
        if self.active_transcription.is_some() {
            return Err("A transcription is already running".into());
        }
        if server_url.trim().is_empty() {
            return Err("Compute server is not configured".into());
        }
        if model.trim().is_empty() {
            return Err("No speech-to-text model is selected".into());
        }
        let selection = crate::audio_selection::selected_audio_project(
            &self.project.borrow(),
            &selected_timeline_items(&self.selection),
            &selected_timeline_tracks(&self.selection),
        )
        .ok_or("No audio item is selected")?;
        let (project, ranges, cut_points) = plan(selection, options);
        let (sender, receiver) = mpsc::channel();
        let handle = Handle {
            cancelled: Arc::new(AtomicBool::new(false)),
            active_request: Arc::new(Mutex::new(None)),
        };
        let worker = handle.clone();
        std::thread::Builder::new()
            .name("timeline-transcription".into())
            .spawn(move || run(project, ranges, server_url, model, worker, sender))
            .map_err(|error| format!("Could not start transcription: {error}"))?;
        self.active_transcription = Some(Job {
            receiver,
            handle: handle.clone(),
            revision: player_state::snapshot(&self.player).revision,
            cut_points,
            snap_tolerance: options.snap_tolerance,
        });
        Ok(handle)
    }

    pub fn take_transcription_update(&mut self) -> Option<Update> {
        self.transcription_updates.pop_front()
    }

    pub(crate) fn poll_transcription(&mut self) -> bool {
        let Some(job) = self.active_transcription.as_ref() else {
            return false;
        };
        let mut latest_progress = None;
        let message = loop {
            match job.receiver.try_recv() {
                Ok(Message::Progress(progress)) => latest_progress = Some(progress),
                Ok(message) => break Some(message),
                Err(TryRecvError::Empty) => break None,
                Err(TryRecvError::Disconnected) => {
                    break Some(Message::Done(Err(
                        "Transcription worker stopped unexpectedly".into(),
                    )));
                }
            }
        };
        let had_progress = latest_progress.is_some();
        if let Some(progress) = latest_progress {
            self.transcription_updates.clear();
            self.transcription_updates
                .push_back(Update::Progress(progress));
        }
        let Some(message) = message else {
            return had_progress;
        };
        match message {
            Message::Progress(_) => unreachable!("progress messages are coalesced while draining"),
            Message::Cancelled => {
                self.active_transcription.take();
                self.transcription_updates.clear();
                self.transcription_updates.push_back(Update::Cancelled);
            }
            Message::Done(result) => {
                let job = self
                    .active_transcription
                    .take()
                    .expect("active transcription exists");
                let result = result.and_then(|segments| {
                    if player_state::snapshot(&self.player).revision != job.revision {
                        return Err("The project changed while audio was being transcribed".into());
                    }
                    self.apply_transcription(segments, &job.cut_points, job.snap_tolerance)
                });
                self.transcription_updates.clear();
                self.transcription_updates.push_back(match result {
                    Ok(captions) => Update::Finished { captions },
                    Err(error) => Update::Failed(error),
                });
            }
        }
        true
    }

    fn apply_transcription(
        &mut self,
        mut segments: Vec<TranscribedSegment>,
        cut_points: &[Time],
        snap_tolerance: Time,
    ) -> Result<usize, String> {
        if segments.is_empty() {
            return Err("The selected audio produced no transcript".into());
        }
        snap_segments_to_cuts(&mut segments, cut_points, snap_tolerance);
        let mut candidate = self.project.borrow().clone();
        let overlap_count = sanitize_transcribed_segments(&mut segments, candidate.frame_step());
        if overlap_count > 0 {
            tracing::warn!(
                overlap_count,
                segment_count = segments.len(),
                "resolved overlapping transcription segments"
            );
        }
        let items = segments
            .into_iter()
            .map(|segment| CaptionItem::new(segment.start, segment.end, segment.text))
            .collect::<Vec<_>>();
        if items.is_empty() {
            return Err("The selected audio produced no transcript".into());
        }
        let captions = items.len();
        candidate.caption_tracks.push(CaptionTrack {
            id: uuid::Uuid::new_v4(),
            enabled: true,
            language: None,
            items,
        });
        crate::project::commit_edit_checked(&candidate, "transcribe-audio")?;
        let duration = candidate.duration();
        *self.project.borrow_mut() = candidate;
        player_state::refresh_project(
            &self.player,
            ProjectChange {
                duration: Some(duration),
                captions: true,
                inspector: true,
                ..ProjectChange::default()
            },
        );
        Ok(captions)
    }
}

fn plan(
    selection: SelectedAudioProject,
    options: Options,
) -> (Project, Vec<(Time, Time)>, Vec<Time>) {
    let SelectedAudioProject {
        project,
        start,
        end,
        chunks,
        audio_cut_points,
        video_cut_points,
    } = selection;
    let cut_points = match options.snap_source {
        SnapSource::Audio => audio_cut_points,
        SnapSource::Video => video_cut_points,
        SnapSource::AudioAndVideo => {
            let mut cuts = audio_cut_points;
            cuts.extend(video_cut_points);
            cuts.sort();
            cuts.dedup();
            cuts
        }
    };
    let chunks = absorb_short_chunks(chunks, options.continue_threshold);
    let continuous = continuous_intervals(chunks.clone());
    let ranges = if options.chunked || continuous.len() > 1 {
        if options.chunked { chunks } else { continuous }
    } else {
        vec![(start, end)]
    };
    (project, ranges, cut_points)
}

fn run(
    project: Project,
    ranges: Vec<(Time, Time)>,
    server_url: String,
    model: String,
    handle: Handle,
    sender: mpsc::Sender<Message>,
) {
    let result = (|| {
        if handle.cancelled.load(Ordering::Relaxed) {
            return None;
        }
        let chunks = match prepare_transcription_chunks(&project, &ranges) {
            Ok(chunks) => chunks,
            Err(error) => return Some(Err(error)),
        };
        let total = chunks.len();
        let mut output = Vec::new();
        for (index, chunk) in chunks.into_iter().enumerate() {
            if handle.cancelled.load(Ordering::Relaxed) {
                return None;
            }
            let cancellation = match shrimply_compute_client::CancellationToken::new(&server_url) {
                Ok(cancellation) => cancellation,
                Err(error) => return Some(Err(error)),
            };
            *handle
                .active_request
                .lock()
                .expect("transcription active request poisoned") = Some(cancellation.clone());
            if handle.cancelled.load(Ordering::Relaxed) {
                cancellation.cancel();
                return None;
            }
            let _ = sender.send(Message::Progress("Sending request…".into()));
            let progress = sender.clone();
            let transcription = shrimply_compute_client::transcribe(
                &server_url,
                &cancellation,
                &model,
                &shrimply_compute_client::AudioUpload::Pcm {
                    data: chunk
                        .samples
                        .iter()
                        .flat_map(|sample| sample.to_le_bytes())
                        .collect(),
                    sample_rate: SAMPLE_RATE,
                    channels: 1,
                },
                |message| {
                    let _ = progress.send(Message::Progress(format!(
                        "{}/{} · {message}",
                        index + 1,
                        total
                    )));
                },
            );
            handle
                .active_request
                .lock()
                .expect("transcription active request poisoned")
                .take();
            let transcription = match transcription {
                Ok(transcription) => transcription,
                Err(_) if cancellation.is_cancelled() => return None,
                Err(error) => return Some(Err(error)),
            };
            if handle.cancelled.load(Ordering::Relaxed) {
                return None;
            }
            let duration = chunk.end.saturating_sub(chunk.start);
            let mut segments = transcription
                .segments
                .into_iter()
                .filter_map(|segment| {
                    let text = segment.text.trim();
                    if text.is_empty() {
                        return None;
                    }
                    let start = Time::from_fraction(
                        i64::try_from(segment.start_frame).unwrap_or(i64::MAX),
                        i64::from(SAMPLE_RATE),
                    )
                    .min(duration);
                    let mut end = Time::from_fraction(
                        i64::try_from(segment.end_frame).unwrap_or(i64::MAX),
                        i64::from(SAMPLE_RATE),
                    )
                    .min(duration);
                    if end <= start {
                        end = start.saturating_add(Time::from_fraction(1, i64::from(SAMPLE_RATE)));
                    }
                    Some(TranscribedSegment {
                        start: chunk.start.saturating_add(start),
                        end: chunk.start.saturating_add(end).min(chunk.end),
                        text: text.to_owned(),
                    })
                })
                .collect::<Vec<_>>();
            if let Some(first) = segments.first_mut() {
                first.start = chunk.start;
            }
            if let Some(last) = segments.last_mut() {
                last.end = chunk.end;
            }
            output.extend(segments);
            let _ = sender.send(Message::Progress(format!(
                "{}/{} · Complete",
                index + 1,
                total
            )));
        }
        Some(Ok(output))
    })();
    let _ = sender.send(match result {
        Some(result) => Message::Done(result),
        None => Message::Cancelled,
    });
}

fn continuous_intervals(chunks: Vec<(Time, Time)>) -> Vec<(Time, Time)> {
    let mut continuous = Vec::new();
    for (start, end) in chunks {
        let Some((_, last_end)) = continuous.last_mut() else {
            continuous.push((start, end));
            continue;
        };
        if start <= *last_end {
            *last_end = (*last_end).max(end);
        } else {
            continuous.push((start, end));
        }
    }
    continuous
}

pub fn absorb_short_chunks(mut chunks: Vec<(Time, Time)>, threshold: Time) -> Vec<(Time, Time)> {
    if threshold <= Time::ZERO {
        return chunks;
    }
    let mut index = 0;
    while chunks.len() > 1 && index < chunks.len() {
        let duration = chunks[index].1.saturating_sub(chunks[index].0);
        if duration > threshold {
            index += 1;
            continue;
        }
        let previous = index > 0
            && chunks[index - 1].1 == chunks[index].0
            && chunks[index].1.saturating_sub(chunks[index - 1].0) <= threshold;
        let next = index + 1 < chunks.len()
            && chunks[index].1 == chunks[index + 1].0
            && chunks[index + 1].1.saturating_sub(chunks[index].0) <= threshold;
        match (previous, next) {
            (true, true) => {
                let previous_duration = chunks[index].1.saturating_sub(chunks[index - 1].0);
                let next_duration = chunks[index + 1].1.saturating_sub(chunks[index].0);
                if previous_duration <= next_duration {
                    chunks[index - 1].1 = chunks[index].1;
                    chunks.remove(index);
                    index = index.saturating_sub(1);
                } else {
                    chunks[index + 1].0 = chunks[index].0;
                    chunks.remove(index);
                }
            }
            (true, false) => {
                chunks[index - 1].1 = chunks[index].1;
                chunks.remove(index);
                index = index.saturating_sub(1);
            }
            (false, true) => {
                chunks[index + 1].0 = chunks[index].0;
                chunks.remove(index);
            }
            (false, false) => index += 1,
        }
    }
    chunks
}

fn snap_segments_to_cuts(
    segments: &mut [TranscribedSegment],
    cut_points: &[Time],
    tolerance: Time,
) {
    if cut_points.is_empty() || tolerance <= Time::ZERO {
        return;
    }
    for segment in segments {
        segment.start = snap_time_to_cut(segment.start, cut_points, tolerance);
        segment.end = snap_time_to_cut(segment.end, cut_points, tolerance);
    }
}

fn snap_time_to_cut(time: Time, cut_points: &[Time], tolerance: Time) -> Time {
    cut_points
        .iter()
        .copied()
        .filter_map(|cut| {
            let distance = cut.abs_diff(time);
            (distance <= tolerance).then_some((cut, distance))
        })
        .min_by_key(|(_, distance)| *distance)
        .map_or(time, |(cut, _)| cut)
}
