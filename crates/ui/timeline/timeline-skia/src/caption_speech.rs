use hashbrown::HashSet;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, TryRecvError},
};

use shrimply_editor_state::player_state::{self, ProjectChange};

use crate::{
    external_content::OwnedFile,
    items::{ItemKey, TrackKind},
    project::{AudioItem, AudioSource, AudioTrack, Project, Time},
    scene::{Scene, selected_timeline_items, selected_timeline_tracks},
};

const PREVIEW_LENGTH: usize = 80;
const ERROR_LENGTH: usize = 300;
const FAILURE_DETAILS: usize = 8;

#[derive(Clone)]
struct CaptionSpeechJob {
    caption_id: uuid::Uuid,
    track_index: usize,
    item_index: usize,
    start: Time,
    end: Time,
    text: String,
}

#[derive(Clone)]
pub struct Plan {
    jobs: Vec<CaptionSpeechJob>,
    skipped: usize,
    revision: u64,
}

impl Plan {
    pub fn chunks(&self) -> usize {
        self.jobs.len()
    }

    pub fn skipped(&self) -> usize {
        self.skipped
    }
}

#[derive(Clone)]
pub struct Options {
    pub server_url: String,
    pub model: shrimply_tts::TtsModel,
    pub settings: shrimply_tts::TtsSettings,
}

#[derive(Clone)]
pub struct Summary {
    pub generated: usize,
    pub failed: usize,
    pub skipped: usize,
    pub unattempted: usize,
    pub cancelled: bool,
    pub details: Vec<String>,
}

impl std::fmt::Display for Summary {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "Generated: {}\nFailed: {}\nSkipped: {}\nUnattempted: {}",
            self.generated, self.failed, self.skipped, self.unattempted
        )?;
        if self.cancelled {
            write!(formatter, "\nGeneration was cancelled.")?;
        }
        for detail in &self.details {
            write!(formatter, "\n\n{detail}")?;
        }
        Ok(())
    }
}

pub enum Update {
    Progress {
        current: usize,
        total: usize,
        message: String,
        preview: String,
    },
    Finished(Summary),
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
            .expect("caption speech active request poisoned")
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
}

impl Drop for Job {
    fn drop(&mut self) {
        self.handle.cancel();
    }
}

struct GeneratedSpeech {
    job: CaptionSpeechJob,
    file: OwnedFile,
    duration: Time,
    settings: shrimply_tts::TtsSettings,
}

struct GenerationFailure {
    job: CaptionSpeechJob,
    error: String,
}

struct RunResult {
    generated: Vec<GeneratedSpeech>,
    failures: Vec<GenerationFailure>,
    skipped: usize,
    unattempted: usize,
    cancelled: bool,
}

enum Message {
    Progress {
        current: usize,
        total: usize,
        message: String,
        preview: String,
    },
    Done(RunResult),
}

pub fn models(server_url: &str) -> Result<Vec<shrimply_tts::TtsModel>, String> {
    let models = shrimply_tts::models(server_url)?
        .into_iter()
        .filter(|model| {
            model
                .inputs
                .iter()
                .any(|input| input.purpose() == Some(shrimply_tts::InputPurpose::Duration))
        })
        .collect::<Vec<_>>();
    if models.is_empty() {
        Err("The server does not provide a text-to-speech model with caption timing".into())
    } else {
        Ok(models)
    }
}

impl Scene {
    pub fn caption_speech_plan(&self) -> Result<Plan, String> {
        let project = self.project.borrow();
        let selected_items = selected_timeline_items(&self.selection);
        let mut jobs = jobs_for_items(&project, &selected_items);
        let mut candidate_count = selected_items
            .iter()
            .filter(|key| key.kind == TrackKind::Caption)
            .count();
        if candidate_count == 0 {
            let tracks = selected_timeline_tracks(&self.selection);
            for key in tracks.iter().filter(|key| key.kind == TrackKind::Caption) {
                candidate_count += project
                    .caption_tracks
                    .get(key.track_index)
                    .map_or(0, |track| track.items.len());
                jobs.extend(jobs_for_track(&project, key.track_index));
            }
        }
        jobs.sort_by_key(|job| (job.start, job.track_index, job.item_index));
        jobs.dedup_by_key(|job| job.caption_id);
        if jobs.is_empty() {
            return Err("The selected captions are empty or have invalid durations".into());
        }
        Ok(Plan {
            skipped: candidate_count.saturating_sub(jobs.len()),
            jobs,
            revision: player_state::snapshot(&self.player).revision,
        })
    }

    pub fn start_caption_speech(&mut self, plan: Plan, options: Options) -> Result<Handle, String> {
        if self.active_caption_speech.is_some() {
            return Err("Speech generation is already running".into());
        }
        if player_state::snapshot(&self.player).revision != plan.revision {
            return Err("The project changed while configuring speech generation".into());
        }
        if options.server_url.trim().is_empty() {
            return Err("Compute server is not configured".into());
        }
        if !options
            .model
            .inputs
            .iter()
            .any(|input| input.purpose() == Some(shrimply_tts::InputPurpose::Duration))
        {
            return Err("The selected model does not support caption timing".into());
        }
        if options.settings.model.as_deref() != Some(options.model.id.as_str()) {
            return Err("Text-to-speech settings do not match the selected model".into());
        }
        let (sender, receiver) = mpsc::channel();
        let handle = Handle {
            cancelled: Arc::new(AtomicBool::new(false)),
            active_request: Arc::new(Mutex::new(None)),
        };
        let worker = handle.clone();
        let skipped = plan.skipped;
        let revision = plan.revision;
        std::thread::Builder::new()
            .name("timeline-caption-speech".into())
            .spawn(move || {
                let result = run_generation(plan.jobs, skipped, options, worker, &sender);
                let _ = sender.send(Message::Done(result));
            })
            .map_err(|error| format!("Could not start speech generation: {error}"))?;
        self.active_caption_speech = Some(Job {
            receiver,
            handle: handle.clone(),
            revision,
        });
        Ok(handle)
    }

    pub fn take_caption_speech_update(&mut self) -> Option<Update> {
        self.caption_speech_updates.pop_front()
    }

    pub(crate) fn poll_caption_speech(&mut self) -> bool {
        let Some(job) = self.active_caption_speech.as_ref() else {
            return false;
        };
        let mut progress = None;
        let terminal = loop {
            match job.receiver.try_recv() {
                Ok(message @ Message::Done(_)) => break Some(message),
                Ok(message @ Message::Progress { .. }) => progress = Some(message),
                Err(TryRecvError::Empty) => break None,
                Err(TryRecvError::Disconnected) => {
                    self.active_caption_speech.take();
                    self.caption_speech_updates.clear();
                    self.caption_speech_updates.push_back(Update::Failed(
                        "Speech generation worker stopped unexpectedly".into(),
                    ));
                    return true;
                }
            }
        };
        let had_progress = progress.is_some();
        if let Some(Message::Progress {
            current,
            total,
            message,
            preview,
        }) = progress
        {
            self.caption_speech_updates.clear();
            self.caption_speech_updates.push_back(Update::Progress {
                current,
                total,
                message,
                preview,
            });
        }
        let Some(Message::Done(result)) = terminal else {
            return had_progress;
        };
        let job = self
            .active_caption_speech
            .take()
            .expect("active speech generation exists");
        self.caption_speech_updates.clear();
        if player_state::snapshot(&self.player).revision != job.revision {
            self.caption_speech_updates.push_back(Update::Failed(
                "The project changed while speech was being generated".into(),
            ));
        } else {
            let update = match self.apply_caption_speech(result) {
                Ok(summary) => Update::Finished(summary),
                Err(error) => Update::Failed(error),
            };
            self.caption_speech_updates.push_back(update);
        }
        true
    }

    fn apply_caption_speech(&mut self, mut result: RunResult) -> Result<Summary, String> {
        let original_successes = result.generated.len();
        let frame_step = self.project.borrow().frame_step();
        let mut items = result
            .generated
            .drain(..)
            .map(|speech| {
                let start = speech.job.start.snapped(frame_step);
                let item = AudioItem::builder(
                    start,
                    start.saturating_add(speech.duration).snapped(frame_step),
                )
                .source_duration(speech.duration)
                .source(AudioSource::Tts(Box::new(speech.settings)))
                .file(speech.file.path().to_owned())
                .build();
                (item, speech.file)
            })
            .collect::<Vec<_>>();
        resolve_overlaps(&mut items, frame_step);
        items.retain(|(item, _)| item.end > item.start && item.time_offset < item.source_duration);
        let collision_failures = original_successes.saturating_sub(items.len());
        let mut summary = summary(&result, items.len(), collision_failures);
        if items.is_empty() {
            return Err(summary.to_string());
        }

        let mut candidate = self.project.borrow().clone();
        let track_index = candidate
            .audio_tracks
            .iter()
            .position(|track| {
                items.iter().all(|(item, _)| {
                    !crate::timeline_search::collides(&track.items, item.start, item.end)
                })
            })
            .unwrap_or_else(|| {
                candidate.audio_tracks.push(AudioTrack::default());
                candidate.audio_tracks.len() - 1
            });
        let mut keys = Vec::with_capacity(items.len());
        for (item, _) in &items {
            let item_index = crate::items::insert_sorted(
                &mut candidate.audio_tracks[track_index].items,
                item.clone(),
            );
            keys.push(ItemKey {
                kind: TrackKind::Audio,
                track_index,
                item_index,
            });
        }
        crate::project::commit_edit_checked(&candidate, "generate-speech")?;
        let duration = candidate.duration();
        *self.project.borrow_mut() = candidate;
        for (_, file) in items {
            file.keep();
        }
        crate::scene::pointer::set_timeline_selection(
            &self.project.borrow(),
            &self.selection,
            keys.clone(),
            keys.first().copied(),
        );
        player_state::refresh_project(
            &self.player,
            ProjectChange {
                duration: Some(duration),
                audio: true,
                audio_waveforms: true,
                inspector: true,
                ..ProjectChange::default()
            },
        );
        summary.generated = keys.len();
        Ok(summary)
    }
}

fn jobs_for_items(project: &Project, keys: &[ItemKey]) -> Vec<CaptionSpeechJob> {
    let mut seen = HashSet::new();
    keys.iter()
        .filter(|key| key.kind == TrackKind::Caption)
        .filter_map(|key| {
            let item = project
                .caption_tracks
                .get(key.track_index)?
                .items
                .get(key.item_index)?;
            let text = crate::caption::clean_text_for_speech(&item.text);
            (seen.insert(item.id) && item.end > item.start && !text.is_empty()).then_some(
                CaptionSpeechJob {
                    caption_id: item.id,
                    track_index: key.track_index,
                    item_index: key.item_index,
                    start: item.start,
                    end: item.end,
                    text,
                },
            )
        })
        .collect()
}

fn jobs_for_track(project: &Project, track_index: usize) -> Vec<CaptionSpeechJob> {
    project
        .caption_tracks
        .get(track_index)
        .into_iter()
        .flat_map(|track| track.items.iter().enumerate())
        .filter_map(|(item_index, item)| {
            let text = crate::caption::clean_text_for_speech(&item.text);
            (item.end > item.start && !text.is_empty()).then_some(CaptionSpeechJob {
                caption_id: item.id,
                track_index,
                item_index,
                start: item.start,
                end: item.end,
                text,
            })
        })
        .collect()
}

fn run_generation(
    jobs: Vec<CaptionSpeechJob>,
    skipped: usize,
    options: Options,
    handle: Handle,
    sender: &mpsc::Sender<Message>,
) -> RunResult {
    let total = jobs.len();
    let mut generated = Vec::new();
    let mut failures = Vec::new();
    for (index, job) in jobs.into_iter().enumerate() {
        if handle.cancelled.load(Ordering::Relaxed) {
            return RunResult {
                generated,
                failures,
                skipped,
                unattempted: total.saturating_sub(index),
                cancelled: true,
            };
        }
        let mut settings = options.settings.clone();
        shrimply_tts::set_text(&mut settings, &options.model, job.text.clone());
        shrimply_tts::set_duration(
            &mut settings,
            &options.model,
            job.end.saturating_sub(job.start).seconds,
        );
        let cancellation =
            match shrimply_compute_client::CancellationToken::new(&options.server_url) {
                Ok(cancellation) => cancellation,
                Err(error) => {
                    failures.push(GenerationFailure { job, error });
                    continue;
                }
            };
        *handle
            .active_request
            .lock()
            .expect("caption speech active request poisoned") = Some(cancellation.clone());
        if handle.cancelled.load(Ordering::Relaxed) {
            cancellation.cancel();
            handle
                .active_request
                .lock()
                .expect("caption speech active request poisoned")
                .take();
            return RunResult {
                generated,
                failures,
                skipped,
                unattempted: total.saturating_sub(index),
                cancelled: true,
            };
        }
        let preview = preview(&job.text, PREVIEW_LENGTH);
        let _ = sender.send(Message::Progress {
            current: index + 1,
            total,
            message: "Sending request…".into(),
            preview: preview.clone(),
        });
        let request = shrimply_tts::speech_request(&options.model, &settings);
        let result = request
            .and_then(|request| {
                shrimply_tts::synthesize(&options.server_url, &cancellation, &request, |message| {
                    let _ = sender.send(Message::Progress {
                        current: index + 1,
                        total,
                        message: message.to_string(),
                        preview: preview.clone(),
                    });
                    !handle.cancelled.load(Ordering::Relaxed)
                })
            })
            .and_then(save_speech);
        handle
            .active_request
            .lock()
            .expect("caption speech active request poisoned")
            .take();
        match result {
            Ok((file, duration, speed)) => {
                shrimply_tts::apply_speed_factor(&mut settings, &options.model, speed);
                generated.push(GeneratedSpeech {
                    job,
                    file,
                    duration,
                    settings,
                });
            }
            Err(_) if handle.cancelled.load(Ordering::Relaxed) => {
                return RunResult {
                    generated,
                    failures,
                    skipped,
                    unattempted: total.saturating_sub(index),
                    cancelled: true,
                };
            }
            Err(error) => failures.push(GenerationFailure {
                job,
                error: shorten_error(&error),
            }),
        }
    }
    RunResult {
        generated,
        failures,
        skipped,
        unattempted: 0,
        cancelled: false,
    }
}

fn save_speech(speech: shrimply_tts::Speech) -> Result<(OwnedFile, Time, crate::Fraction), String> {
    let path = crate::project::project_directory()
        .join("media/tts")
        .join(format!("{}.opus", uuid::Uuid::new_v4()));
    shrimply_audio_engine::recording::save_wav_as_opus(&speech.wav, &path)
        .map(|duration| (OwnedFile::new(path), duration, speech.speed_factor))
}

fn resolve_overlaps(items: &mut Vec<(AudioItem, OwnedFile)>, frame_step: Time) {
    loop {
        items.sort_by_key(|(item, _)| (item.start, item.end, item.id));
        let Some(index) = items
            .windows(2)
            .position(|pair| pair[0].0.end > pair[1].0.start)
        else {
            break;
        };
        let overlap_start = items[index].0.start.max(items[index + 1].0.start);
        let overlap_end = items[index].0.end.min(items[index + 1].0.end);
        let cut = Time {
            seconds: overlap_start.seconds + (overlap_end.seconds - overlap_start.seconds) / 2,
        }
        .snapped(frame_step);
        items[index].0.end = cut;
        let trim = cut.saturating_sub(items[index + 1].0.start);
        items[index + 1].0.start = cut;
        items[index + 1].0.time_offset = items[index + 1].0.time_offset.saturating_add(trim);
        items.retain(|(item, _)| item.end > item.start && item.time_offset < item.source_duration);
    }
}

fn summary(result: &RunResult, generated: usize, collision_failures: usize) -> Summary {
    let mut details = Vec::new();
    if collision_failures > 0 {
        details.push(format!(
            "{collision_failures} generated clip{} became invalid while resolving overlaps.",
            if collision_failures == 1 { "" } else { "s" }
        ));
    }
    details.extend(result.failures.iter().take(FAILURE_DETAILS).map(|failure| {
        format!(
            "Track {}, item {} ({}) — {}\n{}",
            failure.job.track_index + 1,
            failure.job.item_index + 1,
            failure.job.caption_id,
            preview(&failure.job.text, 50),
            failure.error
        )
    }));
    Summary {
        generated,
        failed: result.failures.len() + collision_failures,
        skipped: result.skipped,
        unattempted: result.unattempted,
        cancelled: result.cancelled,
        details,
    }
}

fn preview(text: &str, limit: usize) -> String {
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= limit {
        compact
    } else {
        format!("{}…", compact.chars().take(limit).collect::<String>())
    }
}

fn shorten_error(error: &str) -> String {
    if error.starts_with("Compute server connection failed") {
        tracing::error!(%error, "Caption speech compute connection failed");
        "Compute server connection failed".into()
    } else {
        preview(error, ERROR_LENGTH)
    }
}
