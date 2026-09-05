use super::*;
use shrimply_gtk_components::tr;
use shrimply_gtk_components::ui::I18nAlertDialogExt;

pub(super) fn add_caption_item_context_actions(
    actions: &gio::SimpleActionGroup,
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    preferences: &preferences_store::SharedPreferences,
) {
    add_menu_action(actions, "generate-speech", {
        let area = area.clone();
        let project = project.clone();
        let player_state = player_state.clone();
        let selection_state = selection_state.clone();
        let preferences = preferences.clone();
        move || {
            let url = preferences_store::snapshot(&preferences).compute_server_url;
            if url.is_empty() {
                show_error_dialog(
                    &area,
                    "Compute server is not configured",
                    "Set the Server URL in Preferences before generating speech.",
                );
                return;
            }
            let jobs = caption_tts::jobs_for_items(
                &project.borrow(),
                &selected_timeline_items(&selection_state),
            );
            caption_tts::show_dialog(
                &area,
                &project,
                &player_state,
                &selection_state,
                preferences.clone(),
                jobs,
            );
        }
    });
}

pub(super) fn show_transcribe_dialog(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    preferences: &preferences_store::SharedPreferences,
) {
    let server_url = preferences_store::snapshot(preferences).compute_server_url;
    if server_url.trim().is_empty() {
        show_error_dialog(
            area,
            "Compute server is not configured",
            "Set the Server URL in Preferences before transcribing audio.",
        );
        return;
    }

    let (sender, receiver) =
        async_channel::bounded::<Result<shrimply_server_client::ServerStatus, String>>(1);
    let status_url = server_url.clone();
    thread::spawn(move || {
        let _ = sender.send_blocking(shrimply_server_client::server_status(&status_url));
    });
    let area = area.clone();
    let project = project.clone();
    let player_state = player_state.clone();
    let selection_state = selection_state.clone();
    let preferences = preferences.clone();
    glib::spawn_future_local(async move {
        let status = match receiver.recv().await {
            Ok(Ok(status)) => status,
            Ok(Err(error)) => {
                show_error_dialog(&area, "Could not connect to server", &error);
                return;
            }
            Err(_) => {
                show_error_dialog(
                    &area,
                    "Could not connect to server",
                    "Server status check stopped unexpectedly.",
                );
                return;
            }
        };
        let model_ids = status
            .capabilities
            .into_iter()
            .filter_map(|capability| capability.strip_prefix("stt:").map(str::to_string))
            .filter(|model_id| !model_id.is_empty())
            .collect::<Vec<_>>();
        if model_ids.is_empty() {
            show_error_dialog(
                &area,
                "Speech-to-text is unavailable",
                "The server does not advertise any speech-to-text models.",
            );
            return;
        }
        show_transcribe_dialog_with_models(
            &area,
            &project,
            &player_state,
            &selection_state,
            preferences,
            server_url,
            model_ids,
        );
    });
}

fn show_transcribe_dialog_with_models(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    preferences: preferences_store::SharedPreferences,
    server_url: String,
    model_ids: Vec<String>,
) {
    let model_labels = model_ids.iter().map(String::as_str).collect::<Vec<_>>();
    let model_list = gtk::StringList::new(&model_labels);
    let model = adw::ComboRow::new();
    model.set_title(tr!("Model").as_ref());
    model.set_model(Some(&model_list));
    let last_model = preferences_store::snapshot(&preferences).last_stt_model;
    model.set_selected(
        model_ids
            .iter()
            .position(|model_id| model_id == &last_model)
            .unwrap_or(0) as u32,
    );

    let chunked = adw::SwitchRow::new();
    chunked.set_title(tr!("Follow cuts").as_ref());
    chunked.set_active(true);

    let snap_source_labels = TRANSCRIPTION_SNAP_SOURCES
        .iter()
        .map(|(_, label)| tr!(*label))
        .collect::<Vec<_>>();
    let snap_source_labels = snap_source_labels
        .iter()
        .map(|label| label.as_ref())
        .collect::<Vec<_>>();
    let snap_source_list = gtk::StringList::new(&snap_source_labels);
    let snap_source = adw::ComboRow::new();
    snap_source.set_title(tr!("Snap source").as_ref());
    snap_source.set_model(Some(&snap_source_list));
    snap_source.set_selected(0);

    let tolerance = adw::SpinRow::with_range(0.0, 2.0, 0.01);
    tolerance.set_title(tr!("Snap tolerance").as_ref());
    tolerance.set_value(1.0);
    tolerance.set_digits(2);
    tolerance.set_tooltip_text(Some(
        "Snap generated caption boundaries to nearby cuts within this many seconds.",
    ));

    let continue_threshold = adw::SpinRow::with_range(0.0, 10.0, 0.1);
    continue_threshold.set_title(tr!("Continue cut threshold").as_ref());
    continue_threshold.set_value(2.0);
    continue_threshold.set_digits(1);
    continue_threshold.set_tooltip_text(Some(
        "Absorb chunks shorter than this many seconds only when the merged chunk stays under this threshold.",
    ));
    let selection = {
        let project_state = project.borrow();
        selected_audio_project(
            &project_state,
            &selected_timeline_items(selection_state),
            &selected_timeline_tracks(selection_state),
        )
    };
    let chunk_intervals = selection
        .as_ref()
        .map(|selection| selection.chunks.clone())
        .unwrap_or_default();
    let chunk_count_label = gtk::Label::new(None);
    chunk_count_label.set_halign(gtk::Align::Start);
    let update_chunk_count = {
        let chunk_intervals = Arc::new(chunk_intervals);
        let chunk_count_label = chunk_count_label.clone();
        let chunked = chunked.clone();
        let continue_threshold = continue_threshold.clone();
        move || {
            let threshold = Time::from_nanos(
                (continue_threshold.value() * 1_000_000_000.0)
                    .round()
                    .max(0.0) as u64,
            );
            let chunks = if chunked.is_active() {
                (*chunk_intervals).clone()
            } else {
                continuous_intervals((*chunk_intervals).clone())
            };
            let count = absorb_short_chunks(chunks, threshold).len();
            let key = if count == 1 {
                "%{count} chunk will be transcribed."
            } else {
                "%{count} chunks will be transcribed."
            };
            chunk_count_label.set_label(&shrimply_gtk_components::i18n::text_args(
                key,
                &[("count", count.to_string())],
            ));
        }
    };
    update_chunk_count();
    let update_for_chunked = update_chunk_count.clone();
    chunked.connect_active_notify(move |_| update_for_chunked());
    continue_threshold.connect_value_notify(move |_| update_chunk_count());

    let group = adw::PreferencesGroup::new();
    group.add(&model);
    group.add(&chunked);
    group.add(&snap_source);
    group.add(&tolerance);
    group.add(&continue_threshold);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.set_margin_top(6);
    content.set_margin_bottom(6);
    content.set_margin_start(6);
    content.set_margin_end(6);
    content.append(&group);
    chunk_count_label.add_css_class("dim-label");
    chunk_count_label.set_margin_top(8);
    content.append(&chunk_count_label);

    let dialog = adw::AlertDialog::builder()
        .heading(tr!("Transcribe").as_ref())
        .extra_child(&content)
        .build();
    dialog.add_responses_i18n(&[("cancel", "Cancel"), ("transcribe", "Transcribe")]);
    dialog.set_close_response("cancel");
    dialog.set_default_response(Some("transcribe"));
    dialog.set_response_appearance("transcribe", adw::ResponseAppearance::Suggested);

    let parent = area.clone();
    let Some(selection) = selection else {
        show_error_dialog(area, "Could not transcribe", "No audio item is selected.");
        return;
    };
    let selection = Arc::new(selection);
    let area = area.clone();
    let project = project.clone();
    let player_state = player_state.clone();
    dialog.clone().choose(
        Some(parent.upcast_ref::<gtk::Widget>()),
        None::<&gio::Cancellable>,
        move |response| {
            if response.as_str() == "transcribe" {
                let model_id = model_ids
                    .get(model.selected() as usize)
                    .cloned()
                    .expect("speech-to-text model selection must be valid");
                preferences_store::set_last_stt_model(&preferences, &model_id);
                start_transcription(
                    &area,
                    &project,
                    &player_state,
                    (*selection).clone(),
                    TranscriptionOptions {
                        chunked: chunked.is_active(),
                        snap_source: TRANSCRIPTION_SNAP_SOURCES
                            .get(snap_source.selected() as usize)
                            .map_or(TranscriptionSnapSource::Audio, |(source, _)| *source),
                        snap_tolerance: Time::from_nanos(
                            (tolerance.value() * 1_000_000_000.0).round().max(0.0) as u64,
                        ),
                        continue_threshold: Time::from_nanos(
                            (continue_threshold.value() * 1_000_000_000.0)
                                .round()
                                .max(0.0) as u64,
                        ),
                    },
                    server_url.clone(),
                    model_id,
                );
            }
        },
    );
}

fn start_transcription(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection: SelectedAudioProject,
    options: TranscriptionOptions,
    server_url: String,
    model_id: String,
) {
    let SelectedAudioProject {
        project: transcription_project,
        start,
        end,
        chunks,
        audio_cut_points,
        video_cut_points,
    } = selection;
    let cut_points = match options.snap_source {
        TranscriptionSnapSource::Audio => audio_cut_points,
        TranscriptionSnapSource::Video => video_cut_points,
        TranscriptionSnapSource::AudioAndVideo => {
            let mut cuts = audio_cut_points;
            cuts.extend(video_cut_points);
            cuts.sort();
            cuts.dedup();
            cuts
        }
    };
    let chunks = absorb_short_chunks(chunks, options.continue_threshold);
    let continuous_chunks = continuous_intervals(chunks.clone());

    let (tx, rx) = mpsc::channel();
    let cancelled = Arc::new(AtomicBool::new(false));
    let active_job = Arc::new(Mutex::new(None));
    let progress =
        show_transcription_progress(area, cancelled.clone(), active_job.clone(), &model_id);
    let worker_cancelled = cancelled.clone();
    thread::spawn(move || {
        let chunks = if options.chunked || continuous_chunks.len() > 1 {
            if options.chunked {
                chunks
            } else {
                continuous_chunks
            }
        } else {
            vec![(start, end)]
        };

        let result = run_transcription_process(
            transcription_project,
            chunks,
            &server_url,
            &model_id,
            worker_cancelled,
            active_job,
            tx.clone(),
        );
        let _ = tx.send(match result {
            TranscriptionProcessResult::Done(result) => TranscriptionMessage::Done(result),
            TranscriptionProcessResult::Cancelled => TranscriptionMessage::Cancelled,
        });
    });

    let area = area.clone();
    let project = project.clone();
    let player_state = player_state.clone();
    glib::timeout_add_local(WAVEFORM_POLL_INTERVAL, move || {
        loop {
            match rx.try_recv() {
                Ok(TranscriptionMessage::Progress(message)) => {
                    progress.set_progress(&message);
                }
                Ok(TranscriptionMessage::Done(Ok(segments))) => {
                    progress.close();
                    apply_transcription(
                        &area,
                        &project,
                        &player_state,
                        segments,
                        &cut_points,
                        options.snap_tolerance,
                    );
                    return glib::ControlFlow::Break;
                }
                Ok(TranscriptionMessage::Done(Err(error))) => {
                    progress.close();
                    if error.starts_with("Compute server connection failed") {
                        tracing::error!(%error, "Transcription compute connection failed");
                        show_error_dialog(
                            &area,
                            "Could not transcribe",
                            "Compute server connection failed",
                        );
                    } else {
                        show_error_dialog(&area, "Could not transcribe", &error);
                    }
                    return glib::ControlFlow::Break;
                }
                Ok(TranscriptionMessage::Cancelled) => {
                    progress.close();
                    return glib::ControlFlow::Break;
                }
                Err(TryRecvError::Empty) => return glib::ControlFlow::Continue,
                Err(TryRecvError::Disconnected) => {
                    progress.close();
                    return glib::ControlFlow::Break;
                }
            }
        }
    });
}

struct TranscriptionProgress {
    dialog: adw::AlertDialog,
    progress_label: gtk::Label,
}

impl TranscriptionProgress {
    fn set_progress(&self, message: &str) {
        self.progress_label.set_label(tr!(message).as_ref());
    }

    fn close(&self) {
        self.dialog.close();
    }
}

fn show_transcription_progress(
    area: &gtk::GLArea,
    cancelled: Arc<AtomicBool>,
    active_job: Arc<Mutex<Option<shrimply_server_client::CancellationToken>>>,
    model_id: &str,
) -> TranscriptionProgress {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 10);
    content.set_margin_top(18);
    content.set_margin_bottom(18);
    content.set_margin_start(18);
    content.set_margin_end(18);

    let spinner = adw::Spinner::new();
    spinner.set_halign(gtk::Align::Center);
    spinner.set_size_request(32, 32);
    let progress_label = gtk::Label::new(Some(tr!("Sending request…").as_ref()));
    progress_label.set_halign(gtk::Align::Center);
    let model_label = gtk::Label::new(Some(model_id));
    model_label.set_halign(gtk::Align::Center);
    model_label.set_wrap(true);
    model_label.add_css_class("dim-label");

    content.append(&model_label);
    content.append(&spinner);
    content.append(&progress_label);

    let dialog = adw::AlertDialog::builder()
        .heading(tr!("Transcribing...").as_ref())
        .extra_child(&content)
        .build();
    dialog.add_responses_i18n(&[("cancel", "Cancel")]);
    dialog.set_close_response("cancel");
    let cancel_dialog = dialog.clone();
    let cancel_label = progress_label.clone();
    dialog.clone().choose(
        Some(area.upcast_ref::<gtk::Widget>()),
        None::<&gio::Cancellable>,
        move |_| {
            cancelled.store(true, Ordering::Relaxed);
            cancel_dialog.set_response_enabled("cancel", false);
            cancel_label.set_label(tr!("Cancelling…").as_ref());
            if let Some(cancellation) = active_job
                .lock()
                .expect("transcription active job poisoned")
                .as_ref()
            {
                cancellation.cancel();
            }
        },
    );

    TranscriptionProgress {
        dialog,
        progress_label,
    }
}

#[derive(Clone)]
pub(super) struct SelectedAudioProject {
    pub(super) project: Project,
    pub(super) start: Time,
    end: Time,
    chunks: Vec<(Time, Time)>,
    audio_cut_points: Vec<Time>,
    video_cut_points: Vec<Time>,
}

#[derive(Clone, Copy)]
enum TranscriptionSnapSource {
    Audio,
    Video,
    AudioAndVideo,
}

const TRANSCRIPTION_SNAP_SOURCES: [(TranscriptionSnapSource, &str); 3] = [
    (TranscriptionSnapSource::Audio, "Audio cuts"),
    (TranscriptionSnapSource::Video, "Video cuts"),
    (
        TranscriptionSnapSource::AudioAndVideo,
        "Audio and video cuts",
    ),
];

#[derive(Clone, Copy)]
struct TranscriptionOptions {
    chunked: bool,
    snap_source: TranscriptionSnapSource,
    snap_tolerance: Time,
    continue_threshold: Time,
}

enum TranscriptionMessage {
    Progress(String),
    Done(Result<Vec<TranscribedSegment>, String>),
    Cancelled,
}

enum TranscriptionProcessResult {
    Done(Result<Vec<TranscribedSegment>, String>),
    Cancelled,
}

fn run_transcription_process(
    project: Project,
    ranges: Vec<(Time, Time)>,
    server_url: &str,
    model_id: &str,
    cancelled: Arc<AtomicBool>,
    active_job: Arc<Mutex<Option<shrimply_server_client::CancellationToken>>>,
    tx: mpsc::Sender<TranscriptionMessage>,
) -> TranscriptionProcessResult {
    if cancelled.load(Ordering::Relaxed) {
        return TranscriptionProcessResult::Cancelled;
    }
    let chunks = match prepare_transcription_chunks(&project, &ranges) {
        Ok(chunks) => chunks,
        Err(error) => return TranscriptionProcessResult::Done(Err(error)),
    };
    let total = chunks.len();
    let mut output = Vec::new();
    for (index, chunk) in chunks.into_iter().enumerate() {
        if cancelled.load(Ordering::Relaxed) {
            return TranscriptionProcessResult::Cancelled;
        }
        let progress_tx = tx.clone();
        let cancellation = match shrimply_server_client::CancellationToken::new(server_url) {
            Ok(cancellation) => cancellation,
            Err(error) => return TranscriptionProcessResult::Done(Err(error)),
        };
        *active_job
            .lock()
            .expect("transcription active job poisoned") = Some(cancellation.clone());
        if cancelled.load(Ordering::Relaxed) {
            cancellation.cancel();
            active_job
                .lock()
                .expect("transcription active job poisoned")
                .take();
            return TranscriptionProcessResult::Cancelled;
        }
        let _ = tx.send(TranscriptionMessage::Progress(
            "Sending request…".to_string(),
        ));
        let transcription = match shrimply_server_client::transcribe(
            server_url,
            &cancellation,
            model_id,
            &chunk.samples,
            |message| {
                let _ = progress_tx.send(TranscriptionMessage::Progress(format!(
                    "{}/{} · {message}",
                    index + 1,
                    total
                )));
            },
        ) {
            Ok(transcription) => transcription,
            Err(error) => {
                active_job
                    .lock()
                    .expect("transcription active job poisoned")
                    .take();
                if cancellation.is_cancelled() {
                    return TranscriptionProcessResult::Cancelled;
                }
                return TranscriptionProcessResult::Done(Err(error));
            }
        };
        active_job
            .lock()
            .expect("transcription active job poisoned")
            .take();
        if cancelled.load(Ordering::Relaxed) {
            return TranscriptionProcessResult::Cancelled;
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
                    segment.start_frame.min(i64::MAX as u64) as i64,
                    i64::from(SAMPLE_RATE),
                )
                .min(duration);
                let mut end = Time::from_fraction(
                    segment.end_frame.min(i64::MAX as u64) as i64,
                    i64::from(SAMPLE_RATE),
                )
                .min(duration);
                if end <= start {
                    end = start.saturating_add(Time::from_fraction(1, i64::from(SAMPLE_RATE)));
                }
                Some(TranscribedSegment {
                    start: chunk.start.saturating_add(start),
                    end: chunk.start.saturating_add(end).min(chunk.end),
                    text: text.to_string(),
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
        let _ = tx.send(TranscriptionMessage::Progress(format!(
            "{}/{} · Complete",
            index + 1,
            total
        )));
    }
    TranscriptionProcessResult::Done(Ok(output))
}

pub(super) fn selected_audio_project(
    project: &Project,
    selected_items: &[ItemKey],
    selected_tracks: &[TrackKey],
) -> Option<SelectedAudioProject> {
    let mut start = None::<Time>;
    let mut end = None::<Time>;
    let mut intervals = Vec::new();
    let mut audio_tracks = Vec::new();
    let video_cut_points = cut_points(
        project
            .video_tracks
            .iter()
            .flat_map(|track| track.items.iter().map(|item| (item.start, item.end)))
            .collect(),
    );
    let selected_audio_tracks = selected_tracks
        .iter()
        .any(|key| key.kind == TrackKind::Audio);

    for (track_index, track) in project.audio_tracks.iter().enumerate() {
        let mut track = track.clone();
        track.items = track
            .items
            .into_iter()
            .enumerate()
            .filter_map(|(item_index, item)| {
                let selected = if selected_audio_tracks {
                    selected_tracks
                        .iter()
                        .any(|key| key.kind == TrackKind::Audio && key.track_index == track_index)
                } else {
                    selected_items.iter().any(|key| {
                        key.kind == TrackKind::Audio
                            && key.track_index == track_index
                            && key.item_index == item_index
                    })
                };
                if !selected {
                    return None;
                }
                start = Some(start.map_or(item.start, |value| value.min(item.start)));
                end = Some(end.map_or(item.end, |value| value.max(item.end)));
                intervals.push((item.start, item.end));
                Some(item)
            })
            .collect();
        audio_tracks.push(track);
    }

    Some(SelectedAudioProject {
        project: Project {
            format_version: project.format_version,
            name: project.name.clone(),
            fps: project.fps,
            canvas_size: project.canvas_size,
            caption_tracks: Vec::new(),
            video_tracks: Vec::new(),
            audio_tracks,
            folded_sequences: Vec::new(),
            expanded_sequence_paths: Vec::new(),
            cursor_position: None,
            timeline_zoom: None,
            preview_guides: Default::default(),
        },
        start: start?,
        end: end?,
        chunks: chunk_intervals(intervals.clone()),
        audio_cut_points: cut_points(intervals),
        video_cut_points,
    })
}

fn cut_points(intervals: Vec<(Time, Time)>) -> Vec<Time> {
    let mut cuts = intervals
        .into_iter()
        .flat_map(|(start, end)| [start, end])
        .collect::<Vec<_>>();
    cuts.sort();
    cuts.dedup();
    cuts
}

fn chunk_intervals(mut intervals: Vec<(Time, Time)>) -> Vec<(Time, Time)> {
    intervals.retain(|(start, end)| end > start);
    intervals.sort_by_key(|(start, _)| *start);

    let mut merged: Vec<(Time, Time)> = Vec::new();
    for (start, end) in intervals {
        let Some((_, last_end)) = merged.last_mut() else {
            merged.push((start, end));
            continue;
        };
        if start < *last_end {
            *last_end = (*last_end).max(end);
        } else {
            merged.push((start, end));
        }
    }
    merged
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

fn absorb_short_chunks(mut chunks: Vec<(Time, Time)>, threshold: Time) -> Vec<(Time, Time)> {
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

        let previous_merge = (index > 0
            && chunks[index - 1].1 == chunks[index].0
            && chunks[index].1.saturating_sub(chunks[index - 1].0) <= threshold)
            .then_some(());
        let next_merge = (index + 1 < chunks.len()
            && chunks[index].1 == chunks[index + 1].0
            && chunks[index + 1].1.saturating_sub(chunks[index].0) <= threshold)
            .then_some(());

        match (previous_merge, next_merge) {
            (Some(_), Some(_)) => {
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
            (Some(_), None) => {
                chunks[index - 1].1 = chunks[index].1;
                chunks.remove(index);
                index = index.saturating_sub(1);
            }
            (None, Some(_)) => {
                chunks[index + 1].0 = chunks[index].0;
                chunks.remove(index);
            }
            (None, None) => {
                index += 1;
            }
        }
    }
    chunks
}

fn apply_transcription(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    mut segments: Vec<TranscribedSegment>,
    cut_points: &[Time],
    snap_tolerance: Time,
) {
    if segments.is_empty() {
        show_error_dialog(
            area,
            "No speech detected",
            "The selected audio produced no transcript.",
        );
        return;
    }

    snap_segments_to_cuts(&mut segments, cut_points, snap_tolerance);
    let overlap_count = shrimply_transcription::sanitize_transcribed_segments(
        &mut segments,
        project.borrow().frame_step(),
    );
    if overlap_count > 0 {
        tracing::warn!(
            overlap_count,
            segment_count = segments.len(),
            "resolved overlapping transcription segments"
        );
    }
    let mut project_state = project.borrow_mut();
    let items = segments
        .into_iter()
        .map(|segment| CaptionItem::new(segment.start, segment.end, segment.text))
        .collect::<Vec<_>>();
    if items.is_empty() {
        show_error_dialog(
            area,
            "No speech detected",
            "The selected audio produced no transcript.",
        );
        return;
    }

    project_state.caption_tracks.push(CaptionTrack {
        id: uuid::Uuid::new_v4(),
        enabled: true,
        language: None,
        items,
    });
    if let Err(error) = project_state.validate() {
        project_state.caption_tracks.pop();
        drop(project_state);
        show_error_dialog(area, "Could not add transcript", &error);
        return;
    }
    crate::project::commit_edit(&project_state, "transcribe-audio");
    let duration = project_state.duration();
    drop(project_state);

    player_state::refresh_project(
        player_state,
        ProjectChange {
            duration: Some(duration),
            captions: true,
            inspector: true,
            ..ProjectChange::default()
        },
    );
    area.queue_render();
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
    let mut closest = None::<(Time, Time)>;
    for &cut in cut_points {
        let distance = if cut >= time {
            cut.saturating_sub(time)
        } else {
            time.saturating_sub(cut)
        };
        if distance <= tolerance && closest.is_none_or(|(_, best)| distance < best) {
            closest = Some((cut, distance));
        }
    }
    closest.map_or(time, |(cut, _)| cut)
}
