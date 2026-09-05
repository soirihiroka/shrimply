use hashbrown::HashSet;
use shrimply_gtk_components::tr;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use adw::prelude::*;
use gtk::{gio, glib};
use uuid::Uuid;

use crate::player_state::{self, ProjectChange, SharedPlayerState};
use crate::preferences::store as preferences_store;
use crate::project::{AudioItem, AudioSource, AudioTrack, Project, Time};
use crate::selection_state::SharedSelectionState;
use crate::timeline_search;

use super::items::{ItemKey, TrackKind};

const POLL_INTERVAL: Duration = Duration::from_millis(33);
const DIALOG_WIDTH: i32 = 720;
const DIALOG_HEIGHT: i32 = 640;

#[derive(Clone)]
pub(super) struct CaptionSpeechJob {
    caption_id: Uuid,
    track_index: usize,
    item_index: usize,
    start: Time,
    end: Time,
    text: String,
}

struct GenerationOptions {
    server_url: String,
    model: shrimply_tts::TtsModel,
    settings: shrimply_tts::TtsSettings,
}

struct GeneratedSpeech {
    job: CaptionSpeechJob,
    path: PathBuf,
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

enum WorkerMessage {
    Status(String),
    Progress {
        current: usize,
        total: usize,
        preview: String,
    },
    Done(RunResult),
}

pub(super) fn jobs_for_items(project: &Project, keys: &[ItemKey]) -> Vec<CaptionSpeechJob> {
    let mut seen = HashSet::new();
    let mut jobs = keys
        .iter()
        .filter(|key| key.kind == TrackKind::Caption)
        .filter_map(|key| {
            let item = project
                .caption_tracks
                .get(key.track_index)?
                .items
                .get(key.item_index)?;
            if !seen.insert(item.id) {
                return None;
            }
            let text = crate::caption::clean_text_for_speech(&item.text);
            (!text.is_empty()).then_some(CaptionSpeechJob {
                caption_id: item.id,
                track_index: key.track_index,
                item_index: key.item_index,
                start: item.start,
                end: item.end,
                text,
            })
        })
        .collect::<Vec<_>>();
    jobs.sort_by_key(|job| (job.start, job.track_index, job.item_index));
    jobs
}

pub(super) fn jobs_for_track(project: &Project, track_index: usize) -> Vec<CaptionSpeechJob> {
    project
        .caption_tracks
        .get(track_index)
        .map(|track| {
            track
                .items
                .iter()
                .enumerate()
                .filter_map(|(item_index, item)| {
                    let text = crate::caption::clean_text_for_speech(&item.text);
                    (!text.is_empty()).then_some(CaptionSpeechJob {
                        caption_id: item.id,
                        track_index,
                        item_index,
                        start: item.start,
                        end: item.end,
                        text,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn show_dialog(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    preferences: preferences_store::SharedPreferences,
    jobs: Vec<CaptionSpeechJob>,
) {
    let valid = jobs
        .iter()
        .filter(|job| job.end > job.start && !job.text.trim().is_empty())
        .cloned()
        .collect::<Vec<_>>();
    let skipped = jobs.len().saturating_sub(valid.len());
    if valid.is_empty() {
        super::interaction::show_error_dialog(
            area,
            "Could not generate speech",
            "The selected captions are empty or have invalid durations.",
        );
        return;
    }

    let settings = Rc::new(RefCell::new(shrimply_tts::TtsSettings::default()));
    let models = Rc::new(RefCell::new(Vec::<shrimply_tts::TtsModel>::new()));
    let configuration = shrimply_tts_gtk::caption_configuration(
        preferences.clone(),
        settings.clone(),
        models.clone(),
        Rc::new(|_| {}),
        Rc::new(|| {}),
    );
    let summary = format!(
        "{} caption chunk{} will be generated with each caption's exact duration.{}",
        valid.len(),
        if valid.len() == 1 { "" } else { "s" },
        if skipped == 0 {
            String::new()
        } else {
            format!(
                " {skipped} empty or invalid chunk{} will be skipped.",
                if skipped == 1 { "" } else { "s" }
            )
        }
    );
    let generate = adw::ButtonRow::builder()
        .title(tr!("Generate Speech").as_ref())
        .build();
    generate.add_css_class("suggested-action");
    let action_group = adw::PreferencesGroup::builder()
        .description(&summary)
        .build();
    action_group.add(&generate);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);
    content.append(&configuration);
    content.append(&action_group);
    let page = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::new();
    group.add(&content);
    page.add(&group);
    let dialog = adw::PreferencesDialog::builder()
        .title(tr!("Generate Speech").as_ref())
        .search_enabled(false)
        .content_width(DIALOG_WIDTH)
        .content_height(DIALOG_HEIGHT)
        .build();
    dialog.add(&page);

    let area_for_generation = area.clone();
    let project = project.clone();
    let player_state = player_state.clone();
    let selection_state = selection_state.clone();
    let dialog_for_generation = dialog.clone();
    generate.connect_activated(move |_| {
        let current = settings.borrow().clone();
        let Some(model) = current.model.as_ref().and_then(|id| {
            models
                .borrow()
                .iter()
                .find(|model| &model.id == id)
                .cloned()
        }) else {
            super::interaction::show_error_dialog(
                &area_for_generation,
                "Could not generate speech",
                "The server has not provided a text-to-speech model.",
            );
            return;
        };
        let supports_duration = model
            .inputs
            .iter()
            .any(|input| input.purpose() == Some(shrimply_tts::InputPurpose::Duration));
        if !supports_duration {
            super::interaction::show_error_dialog(
                &area_for_generation,
                "Model does not support caption timing",
                "Select a model that exposes a duration input.",
            );
            return;
        }
        dialog_for_generation.close();
        start_generation(
            &area_for_generation,
            &project,
            &player_state,
            &selection_state,
            valid.clone(),
            skipped,
            GenerationOptions {
                server_url: preferences_store::snapshot(&preferences).compute_server_url,
                model,
                settings: current,
            },
        );
    });
    dialog.present(Some(area.upcast_ref::<gtk::Widget>()));
}

fn start_generation(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    jobs: Vec<CaptionSpeechJob>,
    skipped: usize,
    options: GenerationOptions,
) {
    let (sender, receiver) = mpsc::channel();
    let cancelled = Arc::new(AtomicBool::new(false));
    let active_job = Arc::new(Mutex::new(None));
    let worker_cancelled = cancelled.clone();
    let worker_active_job = active_job.clone();
    thread::spawn(move || {
        let result = run_generation(
            jobs,
            skipped,
            options,
            worker_cancelled,
            worker_active_job,
            &sender,
        );
        let _ = sender.send(WorkerMessage::Done(result));
    });

    let progress = show_progress(area, cancelled, active_job);
    let area = area.clone();
    let project = project.clone();
    let player_state = player_state.clone();
    let selection_state = selection_state.clone();
    glib::timeout_add_local(POLL_INTERVAL, move || {
        loop {
            match receiver.try_recv() {
                Ok(WorkerMessage::Status(status)) => progress.set_status(&status),
                Ok(WorkerMessage::Progress {
                    current,
                    total,
                    preview,
                }) => progress.set(current, total, &preview),
                Ok(WorkerMessage::Done(result)) => {
                    progress.close();
                    apply_result(&area, &project, &player_state, &selection_state, result);
                    return glib::ControlFlow::Break;
                }
                Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                Err(mpsc::TryRecvError::Disconnected) => {
                    progress.close();
                    super::interaction::show_error_dialog(
                        &area,
                        "Could not generate speech",
                        "The speech worker stopped unexpectedly.",
                    );
                    return glib::ControlFlow::Break;
                }
            }
        }
    });
}

fn run_generation(
    jobs: Vec<CaptionSpeechJob>,
    skipped: usize,
    options: GenerationOptions,
    cancelled: Arc<AtomicBool>,
    active_job: Arc<Mutex<Option<shrimply_server_client::CancellationToken>>>,
    sender: &mpsc::Sender<WorkerMessage>,
) -> RunResult {
    let total = jobs.len();
    let mut generated = Vec::new();
    let mut failures = Vec::new();
    for (index, job) in jobs.into_iter().enumerate() {
        if cancelled.load(Ordering::Relaxed) {
            return RunResult {
                generated,
                failures,
                skipped,
                unattempted: total.saturating_sub(index),
                cancelled: true,
            };
        }
        let _ = sender.send(WorkerMessage::Progress {
            current: index + 1,
            total,
            preview: preview(&job.text, 80),
        });
        let mut settings = options.settings.clone();
        shrimply_tts::set_text(&mut settings, &options.model, job.text.clone());
        let duration = job.end.saturating_sub(job.start).seconds;
        shrimply_tts::set_duration(&mut settings, &options.model, duration);
        let cancellation = match shrimply_server_client::CancellationToken::new(&options.server_url)
        {
            Ok(cancellation) => cancellation,
            Err(error) => {
                failures.push(GenerationFailure { job, error });
                continue;
            }
        };
        *active_job.lock().expect("caption TTS active job poisoned") = Some(cancellation.clone());
        if cancelled.load(Ordering::Relaxed) {
            cancellation.cancel();
            active_job
                .lock()
                .expect("caption TTS active job poisoned")
                .take();
            return RunResult {
                generated,
                failures,
                skipped,
                unattempted: total.saturating_sub(index),
                cancelled: true,
            };
        }
        let _ = sender.send(WorkerMessage::Status("Sending request…".to_string()));
        let result = shrimply_tts::speech_request(
            &options.model,
            &settings,
            shrimply_audio::recording::transcode_to_wav,
        )
        .and_then(|request| {
            shrimply_tts::synthesize(&options.server_url, &cancellation, &request, |message| {
                let _ = sender.send(WorkerMessage::Status(message.to_string()));
                !cancelled.load(Ordering::Relaxed)
            })
        })
        .and_then(shrimply_tts_gtk::save_speech);
        active_job
            .lock()
            .expect("caption TTS active job poisoned")
            .take();
        match result {
            Ok((path, duration, speed)) => {
                shrimply_tts::apply_speed_factor(&mut settings, &options.model, speed);
                generated.push(GeneratedSpeech {
                    job,
                    path,
                    duration,
                    settings,
                });
            }
            Err(_) if cancelled.load(Ordering::Relaxed) => {
                return RunResult {
                    generated,
                    failures,
                    skipped,
                    unattempted: total.saturating_sub(index),
                    cancelled: true,
                };
            }
            Err(error) => {
                let error = if error.starts_with("Compute server connection failed") {
                    tracing::error!(%error, "Caption TTS compute connection failed");
                    "Compute server connection failed".to_string()
                } else {
                    shorten_error(&error)
                };
                failures.push(GenerationFailure { job, error });
            }
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

fn apply_result(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    mut result: RunResult,
) {
    let original_successes = result.generated.len();
    let frame_step = project.borrow().frame_step();
    let mut items = result
        .generated
        .drain(..)
        .map(|speech| {
            let start = speech.job.start.snapped(frame_step);
            AudioItem::builder(
                start,
                start.saturating_add(speech.duration).snapped(frame_step),
            )
            .source_duration(speech.duration)
            .source(AudioSource::Tts(Box::new(speech.settings)))
            .file(speech.path)
            .build()
        })
        .collect::<Vec<_>>();
    resolve_overlaps(&mut items, frame_step);
    items.retain(|item| item.end > item.start && item.time_offset < item.source_duration);
    let collision_failures = original_successes.saturating_sub(items.len());
    let summary = result_summary(&result, items.len(), collision_failures);
    if items.is_empty() {
        super::interaction::show_error_dialog(area, "Speech generation finished", &summary);
        return;
    }

    let mut project_state = project.borrow_mut();
    let track_index = project_state
        .audio_tracks
        .iter()
        .position(|track| {
            items
                .iter()
                .all(|item| !timeline_search::collides(&track.items, item.start, item.end))
        })
        .unwrap_or_else(|| {
            project_state.audio_tracks.push(AudioTrack::default());
            project_state.audio_tracks.len() - 1
        });
    let mut keys = Vec::with_capacity(items.len());
    for item in items {
        let item_index =
            super::items::insert_sorted(&mut project_state.audio_tracks[track_index].items, item);
        keys.push(ItemKey {
            kind: TrackKind::Audio,
            track_index,
            item_index,
        });
    }
    crate::project::commit_edit(&project_state, "generate-speech");
    let duration = project_state.duration();
    drop(project_state);

    super::interaction::set_timeline_selection(
        &project.borrow(),
        selection_state,
        keys.clone(),
        keys.first().copied(),
    );
    player_state::refresh_project(
        player_state,
        ProjectChange {
            duration: Some(duration),
            audio: true,
            audio_waveforms: true,
            inspector: true,
            ..ProjectChange::default()
        },
    );
    area.queue_render();
    if result.cancelled
        || !result.failures.is_empty()
        || result.skipped > 0
        || result.unattempted > 0
        || collision_failures > 0
    {
        super::interaction::show_error_dialog(area, "Speech generation finished", &summary);
    }
}

fn resolve_overlaps(items: &mut Vec<AudioItem>, frame_step: Time) {
    loop {
        items.sort_by_key(|item| (item.start, item.end, item.id));
        let Some(index) = items
            .windows(2)
            .position(|pair| pair[0].end > pair[1].start)
        else {
            break;
        };
        let overlap_start = items[index].start.max(items[index + 1].start);
        let overlap_end = items[index].end.min(items[index + 1].end);
        let cut = Time {
            seconds: overlap_start.seconds + (overlap_end.seconds - overlap_start.seconds) / 2,
        }
        .snapped(frame_step);
        items[index].end = cut;
        let trim = cut.saturating_sub(items[index + 1].start);
        items[index + 1].start = cut;
        items[index + 1].time_offset = items[index + 1].time_offset.saturating_add(trim);
        items.retain(|item| item.end > item.start && item.time_offset < item.source_duration);
    }
}

fn result_summary(result: &RunResult, generated: usize, collision_failures: usize) -> String {
    let mut summary = format!(
        "Generated: {}\nFailed: {}\nSkipped: {}\nUnattempted: {}",
        generated,
        result.failures.len() + collision_failures,
        result.skipped,
        result.unattempted
    );
    if result.cancelled {
        summary.push_str("\nGeneration was cancelled.");
    }
    if collision_failures > 0 {
        summary.push_str(&format!(
            "\n{collision_failures} generated clip{} became invalid while resolving overlaps.",
            if collision_failures == 1 { "" } else { "s" }
        ));
    }
    for failure in result.failures.iter().take(8) {
        summary.push_str(&format!(
            "\n\nTrack {}, item {} ({}) — {}\n{}",
            failure.job.track_index + 1,
            failure.job.item_index + 1,
            failure.job.caption_id,
            preview(&failure.job.text, 50),
            failure.error
        ));
    }
    summary
}

struct ProgressDialog {
    dialog: adw::AlertDialog,
    chunk: gtk::Label,
    preview: gtk::Label,
}

impl ProgressDialog {
    fn set_status(&self, status: &str) {
        self.chunk.set_label(tr!(status).as_ref());
    }

    fn set(&self, current: usize, total: usize, preview: &str) {
        self.chunk
            .set_label(&shrimply_gtk_components::i18n::text_args(
                "Chunk %{current}/%{total}",
                &[
                    ("current", current.to_string()),
                    ("total", total.to_string()),
                ],
            ));
        self.preview.set_label(preview);
    }

    fn close(&self) {
        self.dialog.close();
    }
}

fn show_progress(
    area: &gtk::GLArea,
    cancelled: Arc<AtomicBool>,
    active_job: Arc<Mutex<Option<shrimply_server_client::CancellationToken>>>,
) -> ProgressDialog {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 10);
    content.set_margin_top(18);
    content.set_margin_bottom(18);
    content.set_margin_start(18);
    content.set_margin_end(18);
    let spinner = adw::Spinner::new();
    spinner.set_halign(gtk::Align::Center);
    spinner.set_size_request(32, 32);
    let chunk = gtk::Label::new(Some(tr!("Sending request…").as_ref()));
    let preview = gtk::Label::new(None);
    preview.add_css_class("dim-label");
    preview.set_ellipsize(gtk::pango::EllipsizeMode::End);
    content.append(&spinner);
    content.append(&chunk);
    content.append(&preview);
    let dialog = adw::AlertDialog::builder()
        .heading(tr!("Generating Speech…").as_ref())
        .extra_child(&content)
        .build();
    dialog.add_response("cancel", tr!("Cancel").as_ref());
    dialog.set_close_response("cancel");
    let cancel_dialog = dialog.clone();
    let cancel_chunk = chunk.clone();
    dialog.clone().choose(
        Some(area.upcast_ref::<gtk::Widget>()),
        None::<&gio::Cancellable>,
        move |_| {
            cancelled.store(true, Ordering::Relaxed);
            cancel_dialog.set_response_enabled("cancel", false);
            cancel_chunk.set_label(tr!("Cancelling…").as_ref());
            if let Some(cancellation) = active_job
                .lock()
                .expect("caption TTS active job poisoned")
                .as_ref()
            {
                cancellation.cancel();
            }
        },
    );
    ProgressDialog {
        dialog,
        chunk,
        preview,
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
    preview(error, 300)
}
