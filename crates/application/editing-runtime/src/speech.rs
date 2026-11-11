use super::*;

pub(super) async fn list_stt_models(
    preferences: &SharedPreferences,
    canceled: Arc<AtomicBool>,
) -> Result<Value, String> {
    let preferences = preferences::snapshot(preferences);
    let server_url = preferences.compute_server_url;
    let preferred = preferences.last_stt_model;
    let (sender, receiver) = async_channel::bounded(1);
    thread::Builder::new()
        .name("shrimply-editing-stt-models".to_string())
        .spawn(move || {
            let _ = sender.send_blocking(stt_models(&server_url));
        })
        .map_err(|error| format!("could not start STT model discovery: {error}"))?;
    let models = loop {
        if canceled.load(Ordering::Acquire) {
            return Err("editing API client canceled STT model discovery".to_string());
        }
        match receiver.try_recv() {
            Ok(result) => break result?,
            Err(async_channel::TryRecvError::Empty) => {
                glib::timeout_future(CANCELLATION_POLL_INTERVAL).await;
            }
            Err(async_channel::TryRecvError::Closed) => {
                return Err("STT model discovery worker stopped without a result".to_string());
            }
        }
    };
    let default_model = models
        .iter()
        .find(|model| **model == preferred)
        .or_else(|| models.first())
        .cloned();
    serde_json::to_value(ListSttModelsResponse {
        models,
        default_model,
    })
    .map_err(|error| format!("could not serialize STT model response: {error}"))
}

pub(super) fn stt_models(server_url: &str) -> Result<Vec<String>, String> {
    let mut models = shrimply_compute_client::server_status(server_url)?
        .capabilities
        .into_iter()
        .filter_map(|capability| capability.strip_prefix("stt:").map(str::to_string))
        .filter(|model| !model.is_empty())
        .collect::<Vec<_>>();
    models.sort();
    models.dedup();
    Ok(models)
}

struct CompletedTranscription {
    model: String,
    segments: Vec<shrimply_transcription::TranscribedSegment>,
}

pub(super) async fn transcribe_audio(
    live: &Rc<RefCell<Project>>,
    player: &SharedPlayerState,
    selection: &SharedSelectionState,
    preferences: &SharedPreferences,
    request: TranscribeAudioRequest,
    canceled: Arc<AtomicBool>,
) -> Result<Value, String> {
    if let Some(language) = &request.language
        && !caption_languages().contains(language)
    {
        return Err(format!("{language} is not a supported caption language"));
    }
    let project_path = shrimply_project_document::project::normalized_project_path(
        &shrimply_project_document::project::active_project_path(),
    );
    let mut project = live.borrow().clone();
    let original = content_revision(&project)?;
    let (transcription_project, ranges) = transcription_source(&project, &request)?;
    let preference = preferences::snapshot(preferences);
    let active_job = Arc::new(Mutex::new(None));
    let worker_job = active_job.clone();
    let worker_canceled = canceled.clone();
    let requested_model = request.model.clone();
    let (sender, receiver) = async_channel::bounded(1);
    thread::Builder::new()
        .name("shrimply-editing-transcribe".to_string())
        .spawn(move || {
            let result = run_transcription(
                transcription_project,
                ranges,
                preference.compute_server_url,
                preference.last_stt_model,
                requested_model,
                worker_canceled,
                worker_job,
            );
            let _ = sender.send_blocking(result);
        })
        .map_err(|error| format!("could not start transcription worker: {error}"))?;

    let mut transcription = loop {
        if canceled.load(Ordering::Acquire)
            && let Some(job) = active_job
                .lock()
                .expect("editing API transcription active job lock was poisoned")
                .as_ref()
        {
            job.cancel();
        }
        match receiver.try_recv() {
            Ok(result) => break result?,
            Err(async_channel::TryRecvError::Empty) => {
                glib::timeout_future(CANCELLATION_POLL_INTERVAL).await;
            }
            Err(async_channel::TryRecvError::Closed) => {
                return Err("transcription worker stopped without a result".to_string());
            }
        }
    };
    if canceled.load(Ordering::Acquire) {
        return Err("editing API client canceled transcription".to_string());
    }
    let current_path = shrimply_project_document::project::normalized_project_path(
        &shrimply_project_document::project::active_project_path(),
    );
    if current_path != project_path {
        return Err("project path changed while audio was being transcribed".to_string());
    }
    if content_revision(&live.borrow())? != original {
        return Err(
            "project changed while audio was being transcribed; retry the request".to_string(),
        );
    }

    let overlap_count = shrimply_transcription::sanitize_transcribed_segments(
        &mut transcription.segments,
        project.frame_step(),
    );
    if overlap_count > 0 {
        tracing::warn!(
            overlap_count,
            segment_count = transcription.segments.len(),
            "resolved overlapping editing API transcription segments"
        );
    }
    if transcription.segments.is_empty() {
        return Err("the selected audio produced no transcript".to_string());
    }
    let captions = transcription
        .segments
        .into_iter()
        .map(|segment| CaptionCueInput {
            start_frame: segment.start.as_frame(project.fps),
            end_frame: segment.end.as_frame(project.fps),
            text: segment.text,
            copy_style_from: None,
        })
        .collect();
    let operation =
        shrimply_editing_api::protocol::EditOperation::InsertCaptions(InsertCaptionsRequest {
            track: None,
            captions,
            language: request.language,
            enabled: None,
            collision: CollisionBehavior::Reject,
        });
    let mutation = shrimply_editing_api::edit::apply_non_import(
        &mut project,
        &operation,
        0,
        &SequenceScopeId::root(),
    )?;
    project
        .validate()
        .map_err(|error| format!("transcription edit is invalid: {error}"))?;
    let changed_presentations = shrimply_editing_api::query::presentations_affected_by_items(
        &project,
        &mutation.changed_item_ids.iter().copied().collect(),
    )?;
    let result = EditOperationResult {
        details: None,
        index: 0,
        operation: "transcribe_audio".to_string(),
        changed_addresses: changed_presentations
            .iter()
            .map(|clip| clip.address.clone())
            .collect(),
        deleted_addresses: mutation.deleted_addresses,
        changed_tracks: mutation.changed_tracks,
        presentations: changed_presentations,
    };
    let duration = project.duration();
    let frame_rate = project.fps;
    let duration_frame =
        shrimply_editing_api::query::frame_time_from_time(duration, frame_rate, true);
    if canceled.load(Ordering::Acquire) {
        return Err("editing API client canceled transcription before commit".to_string());
    }
    let editor_selection = EditorSelection::capture(selection, &live.borrow());
    shrimply_project_document::project::commit_edit_saved(&project, "editing API transcribe audio")
        .map_err(|error| {
            format!("editing API transcription edit could not be committed: {error}")
        })?;
    *live.borrow_mut() = project;
    editor_selection.restore(selection, &live.borrow());
    preferences::set_last_stt_model(preferences, &transcription.model);
    player_state::refresh_project(
        player,
        ProjectChange {
            duration: Some(duration),
            frame_rate: Some(frame_rate),
            captions: true,
            inspector: true,
            ..Default::default()
        },
    );
    serde_json::to_value(EditResponse {
        operations: vec![result],
        duration: duration_frame,
        revision: player_state::snapshot(player).revision,
    })
    .map_err(|error| format!("could not serialize transcription edit result: {error}"))
}

fn run_transcription(
    project: Project,
    ranges: Vec<(Time, Time)>,
    server_url: String,
    preferred_model: String,
    requested_model: Option<String>,
    canceled: Arc<AtomicBool>,
    active_job: Arc<Mutex<Option<shrimply_compute_client::CancellationToken>>>,
) -> Result<CompletedTranscription, String> {
    let models = stt_models(&server_url)?;
    let model = requested_model
        .as_ref()
        .and_then(|requested| models.iter().find(|model| *model == requested))
        .or_else(|| models.iter().find(|model| **model == preferred_model))
        .or_else(|| models.first())
        .cloned()
        .ok_or_else(|| "the compute server provided no STT models".to_string())?;
    if let Some(requested) = &requested_model
        && requested != &model
    {
        return Err(format!("STT model {requested:?} is not available"));
    }
    let chunks = shrimply_transcription::prepare_transcription_chunks(&project, &ranges)?;
    let mut output = Vec::new();
    for (index, chunk) in chunks.into_iter().enumerate() {
        if canceled.load(Ordering::Acquire) {
            return Err("editing API client canceled transcription".to_string());
        }
        let cancellation = shrimply_compute_client::CancellationToken::new(&server_url)?;
        *active_job
            .lock()
            .expect("editing API transcription active job lock was poisoned") =
            Some(cancellation.clone());
        let result = shrimply_compute_client::transcribe(
            &server_url,
            &cancellation,
            &model,
            &shrimply_compute_client::AudioUpload::Pcm {
                data: chunk
                    .samples
                    .iter()
                    .flat_map(|sample| sample.to_le_bytes())
                    .collect(),
                sample_rate: shrimply_transcription::SAMPLE_RATE,
                channels: 1,
            },
            |message| tracing::info!(chunk = index + 1, %message, "editing API transcription progress"),
        );
        active_job
            .lock()
            .expect("editing API transcription active job lock was poisoned")
            .take();
        let transcription = result?;
        if canceled.load(Ordering::Acquire) {
            return Err("editing API client canceled transcription".to_string());
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
                    i64::from(shrimply_transcription::SAMPLE_RATE),
                )
                .min(duration);
                let mut end = Time::from_fraction(
                    segment.end_frame.min(i64::MAX as u64) as i64,
                    i64::from(shrimply_transcription::SAMPLE_RATE),
                )
                .min(duration);
                if end <= start {
                    end = start.saturating_add(Time::from_fraction(
                        1,
                        i64::from(shrimply_transcription::SAMPLE_RATE),
                    ));
                }
                Some(shrimply_transcription::TranscribedSegment {
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
    }
    Ok(CompletedTranscription {
        model,
        segments: output,
    })
}

pub(super) fn transcription_source(
    project: &Project,
    request: &TranscribeAudioRequest,
) -> Result<(Project, Vec<(Time, Time)>), String> {
    let addresses = match (&request.track, request.clips.is_empty()) {
        (Some(_), false) => {
            return Err("transcribe_audio requires exactly one of track or clips".to_string());
        }
        (None, true) => {
            return Err(
                "transcribe_audio requires an audio track or at least one audio clip".to_string(),
            );
        }
        (Some(track), true) => {
            let address = shrimply_editing_api::query::model_track_address(track)?;
            let TrackRef::Audio(source) = project
                .track(&address)
                .ok_or_else(|| "transcription track was not found".to_string())?
            else {
                return Err("transcription requires an audio track".to_string());
            };
            source
                .items
                .iter()
                .map(|item| address.item(item.id))
                .collect::<Vec<_>>()
        }
        (None, false) => request
            .clips
            .iter()
            .map(|clip| {
                let address = shrimply_editing_api::query::model_item_address(clip)?;
                project
                    .audio_item(&address)
                    .ok_or_else(|| format!("audio clip {} was not found", clip.item_id))?;
                Ok(address)
            })
            .collect::<Result<Vec<_>, String>>()?,
    };
    let addresses = addresses.into_iter().collect::<HashSet<_>>();
    if addresses.is_empty() {
        return Err("the transcription source contains no audio clips".to_string());
    }

    let mut allowed = HashMap::<Option<Uuid>, HashSet<Uuid>>::new();
    let mut included_sequences = HashSet::new();
    let mut ranges = Vec::with_capacity(addresses.len());
    for address in &addresses {
        let ItemAddress::Audio {
            sequence_path,
            track_id,
            item_id,
        } = address
        else {
            return Err("transcription requires audio clips".to_string());
        };
        let mut tracks = project.audio_tracks.as_slice();
        let mut sequence_id = None;
        for host_id in sequence_path {
            let host = tracks
                .iter()
                .flat_map(|track| &track.items)
                .find(|item| item.id == *host_id)
                .ok_or_else(|| "audio clip sequence path was not found".to_string())?;
            allowed.entry(sequence_id).or_default().insert(*host_id);
            let AudioSource::FoldedSequence(reference) = &host.source else {
                return Err("audio clip sequence path contains a non-sequence clip".to_string());
            };
            sequence_id = Some(reference.sequence_id);
            tracks = &project
                .folded_sequence(reference.sequence_id)
                .ok_or_else(|| "audio clip sequence definition was not found".to_string())?
                .audio_tracks;
        }
        let item = tracks
            .iter()
            .find(|track| track.id == *track_id)
            .and_then(|track| track.items.iter().find(|item| item.id == *item_id))
            .ok_or_else(|| "audio clip was not found".to_string())?;
        allowed.entry(sequence_id).or_default().insert(*item_id);
        if let AudioSource::FoldedSequence(reference) = &item.source {
            include_sequence_audio(
                project,
                reference.sequence_id,
                &mut allowed,
                &mut included_sequences,
            )?;
        }
        ranges.push(
            project
                .projected_item_times(address)
                .ok_or_else(|| "audio clip has no visible projected range".to_string())?,
        );
    }

    let mut selected = project.clone();
    selected.caption_tracks.clear();
    selected.video_tracks.clear();
    let root = allowed.get(&None);
    for track in &mut selected.audio_tracks {
        track
            .items
            .retain(|item| root.is_some_and(|items| items.contains(&item.id)));
    }
    for sequence in &mut selected.folded_sequences {
        sequence.video_tracks.clear();
        let items = allowed.get(&Some(sequence.id));
        for track in &mut sequence.audio_tracks {
            track
                .items
                .retain(|item| items.is_some_and(|items| items.contains(&item.id)));
        }
    }

    ranges.sort_by_key(|range| *range);
    let mut chunks: Vec<(Time, Time)> = Vec::new();
    for (start, end) in ranges {
        let Some((_, last_end)) = chunks.last_mut() else {
            chunks.push((start, end));
            continue;
        };
        if start < *last_end {
            *last_end = (*last_end).max(end);
        } else {
            chunks.push((start, end));
        }
    }
    Ok((selected, chunks))
}

pub(super) fn include_sequence_audio(
    project: &Project,
    sequence_id: Uuid,
    allowed: &mut HashMap<Option<Uuid>, HashSet<Uuid>>,
    included: &mut HashSet<Uuid>,
) -> Result<(), String> {
    if !included.insert(sequence_id) {
        return Ok(());
    }
    let sequence = project
        .folded_sequence(sequence_id)
        .ok_or_else(|| format!("audio sequence {sequence_id} was not found"))?;
    for item in sequence.audio_tracks.iter().flat_map(|track| &track.items) {
        allowed
            .entry(Some(sequence_id))
            .or_default()
            .insert(item.id);
        if let AudioSource::FoldedSequence(reference) = &item.source {
            include_sequence_audio(project, reference.sequence_id, allowed, included)?;
        }
    }
    Ok(())
}

pub(super) async fn list_tts_models(
    preferences: &SharedPreferences,
    canceled: Arc<AtomicBool>,
) -> Result<Value, String> {
    let preferences = preferences::snapshot(preferences);
    let server_url = preferences.compute_server_url;
    let preferred = preferences.last_tts_model;
    let (sender, receiver) = async_channel::bounded(1);
    thread::Builder::new()
        .name("shrimply-editing-tts-models".to_string())
        .spawn(move || {
            let _ = sender.send_blocking(shrimply_tts::models(&server_url));
        })
        .map_err(|error| format!("could not start TTS model discovery: {error}"))?;
    let models = loop {
        if canceled.load(Ordering::Acquire) {
            return Err("editing API client canceled TTS model discovery".to_string());
        }
        match receiver.try_recv() {
            Ok(result) => break result?,
            Err(async_channel::TryRecvError::Empty) => {
                glib::timeout_future(CANCELLATION_POLL_INTERVAL).await;
            }
            Err(async_channel::TryRecvError::Closed) => {
                return Err("TTS model discovery worker stopped without a result".to_string());
            }
        }
    };
    let default_model = models
        .iter()
        .find(|model| model.id == preferred)
        .or_else(|| models.first())
        .map(|model| model.id.clone());
    let models = models
        .into_iter()
        .map(|model| {
            serde_json::to_value(model)
                .map_err(|error| format!("could not serialize TTS model: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    serde_json::to_value(ListTtsModelsResponse {
        models,
        default_model,
    })
    .map_err(|error| format!("could not serialize TTS model response: {error}"))
}

struct StagedSpeech {
    staging: PathBuf,
    final_path: PathBuf,
    promoted: bool,
}

impl StagedSpeech {
    fn promote(&mut self) -> Result<(), String> {
        fs::rename(&self.staging, &self.final_path).map_err(|error| {
            format!(
                "could not promote generated speech {}: {error}",
                self.final_path.display()
            )
        })?;
        self.promoted = true;
        Ok(())
    }

    fn rollback(&mut self) {
        fs::rename(&self.final_path, &self.staging).unwrap_or_else(|error| {
            panic!(
                "could not roll back generated speech {}: {error}",
                self.final_path.display()
            )
        });
        self.promoted = false;
    }
}

impl Drop for StagedSpeech {
    fn drop(&mut self) {
        if !self.promoted && self.staging.exists() {
            fs::remove_file(&self.staging).unwrap_or_else(|error| {
                panic!(
                    "could not clean staged generated speech {}: {error}",
                    self.staging.display()
                )
            });
        }
    }
}

struct GeneratedTts {
    speech: StagedSpeech,
    duration: Time,
    settings: shrimply_tts::TtsSettings,
}

pub(super) async fn generate_tts(
    live: &Rc<RefCell<Project>>,
    player: &SharedPlayerState,
    selection: &SharedSelectionState,
    preferences: &SharedPreferences,
    request: GenerateTtsRequest,
    canceled: Arc<AtomicBool>,
) -> Result<Value, String> {
    if request.text.trim().is_empty() {
        return Err("text must not be empty".to_string());
    }
    let project_path = shrimply_project_document::project::normalized_project_path(
        &shrimply_project_document::project::active_project_path(),
    );
    let mut project = live.borrow().clone();
    let original = content_revision(&project)?;
    let playhead_frame = player_state::current_time(player).as_frame(project.fps);
    let active_scope = selection_state::active_scope(selection);
    let preferences = preferences::snapshot(preferences);
    let cancellation =
        shrimply_compute_client::CancellationToken::new(&preferences.compute_server_url)?;
    let worker_cancellation = cancellation.clone();
    let worker_path = project_path.clone();
    let worker_request = request.clone();
    let (sender, receiver) = async_channel::bounded(1);
    thread::Builder::new()
        .name("shrimply-editing-generate-tts".to_string())
        .spawn(move || {
            let result = prepare_generated_tts(
                worker_path,
                preferences.compute_server_url,
                preferences.last_tts_model,
                worker_request,
                worker_cancellation,
            );
            let _ = sender.send_blocking(result);
        })
        .map_err(|error| format!("could not start TTS generation worker: {error}"))?;
    let mut cancellation_sent = false;
    let mut generated = loop {
        if canceled.load(Ordering::Acquire) && !cancellation_sent {
            cancellation.cancel();
            cancellation_sent = true;
        }
        match receiver.try_recv() {
            Ok(result) => break result?,
            Err(async_channel::TryRecvError::Empty) => {
                glib::timeout_future(CANCELLATION_POLL_INTERVAL).await;
            }
            Err(async_channel::TryRecvError::Closed) => {
                return Err("TTS generation worker stopped without a result".to_string());
            }
        }
    };
    if canceled.load(Ordering::Acquire) {
        return Err("editing API client canceled TTS generation".to_string());
    }
    let current_path = shrimply_project_document::project::normalized_project_path(
        &shrimply_project_document::project::active_project_path(),
    );
    if current_path != project_path {
        return Err("project path changed while TTS was being generated".to_string());
    }
    if content_revision(&live.borrow())? != original {
        return Err("project changed while TTS was being generated; retry the request".to_string());
    }
    let editor_selection = EditorSelection::capture(selection, &project);
    let mutation = imports::insert_generated_tts(
        &mut project,
        &request,
        playhead_frame,
        active_scope,
        generated.duration,
        generated.settings,
        generated.speech.final_path.clone(),
    )?;
    project
        .validate()
        .map_err(|error| format!("generated TTS edit is invalid: {error}"))?;
    let changed_presentations = shrimply_editing_api::query::presentations_affected_by_items(
        &project,
        &mutation.changed_item_ids.iter().copied().collect(),
    )?;
    let mut presentations = changed_presentations.clone();
    presentations.extend(mutation.deleted_presentations.clone());
    let result = EditOperationResult {
        details: None,
        index: 0,
        operation: "generate_tts".to_string(),
        changed_addresses: changed_presentations
            .iter()
            .map(|clip| clip.address.clone())
            .collect(),
        deleted_addresses: mutation.deleted_addresses,
        changed_tracks: mutation.changed_tracks,
        presentations,
    };
    let duration = project.duration();
    let frame_rate = project.fps;
    let duration_frame =
        shrimply_editing_api::query::frame_time_from_time(duration, frame_rate, true);
    if canceled.load(Ordering::Acquire) {
        return Err("editing API client canceled TTS generation before commit".to_string());
    }
    generated.speech.promote()?;
    if canceled.load(Ordering::Acquire) {
        generated.speech.rollback();
        return Err("editing API client canceled TTS generation before commit".to_string());
    }
    if let Err(error) =
        shrimply_project_document::project::commit_edit_saved(&project, "editing API generate TTS")
    {
        generated.speech.rollback();
        return Err(format!(
            "editing API TTS edit could not be committed: {error}"
        ));
    }
    *live.borrow_mut() = project;
    editor_selection.restore(selection, &live.borrow());
    player_state::refresh_project(
        player,
        ProjectChange {
            duration: Some(duration),
            frame_rate: Some(frame_rate),
            audio: true,
            audio_beats: true,
            audio_waveforms: true,
            inspector: true,
            ..Default::default()
        },
    );
    serde_json::to_value(EditResponse {
        operations: vec![result],
        duration: duration_frame,
        revision: player_state::snapshot(player).revision,
    })
    .map_err(|error| format!("could not serialize TTS edit result: {error}"))
}

fn prepare_generated_tts(
    project_path: PathBuf,
    server_url: String,
    preferred_model: String,
    request: GenerateTtsRequest,
    cancellation: shrimply_compute_client::CancellationToken,
) -> Result<GeneratedTts, String> {
    let models = shrimply_tts::models(&server_url)?;
    let model = request
        .model
        .as_ref()
        .and_then(|id| models.iter().find(|model| &model.id == id))
        .or_else(|| models.iter().find(|model| model.id == preferred_model))
        .or_else(|| models.first())
        .cloned()
        .ok_or_else(|| "the compute server provided no TTS models".to_string())?;
    if let Some(requested) = &request.model
        && requested != &model.id
    {
        return Err(format!("TTS model {requested:?} is not available"));
    }
    let mut settings = shrimply_tts::TtsSettings::default();
    shrimply_tts::sync_settings(&mut settings, &model);
    for (key, input) in request.inputs {
        let definition = model
            .inputs
            .iter()
            .find(|definition| definition.key() == key)
            .ok_or_else(|| format!("TTS model {} has no input {key:?}", model.id))?;
        if definition.purpose() == Some(shrimply_tts::InputPurpose::Text) {
            return Err(format!(
                "TTS input {key:?} is controlled by the top-level text field"
            ));
        }
        let value = match (definition, input) {
            (shrimply_tts::InputDefinition::Text { .. }, TtsInputValue::Text { value }) => {
                shrimply_tts::TtsValue::Text { value }
            }
            (shrimply_tts::InputDefinition::Select { .. }, TtsInputValue::Select { value }) => {
                shrimply_tts::TtsValue::Select { value }
            }
            (shrimply_tts::InputDefinition::Audio { .. }, TtsInputValue::Audio { path }) => {
                let path = PathBuf::from(&path).canonicalize().map_err(|error| {
                    format!("could not resolve TTS audio input {path:?}: {error}")
                })?;
                if !path.is_file() {
                    return Err(format!(
                        "TTS audio input is not a regular file: {}",
                        path.display()
                    ));
                }
                shrimply_tts::TtsValue::Audio {
                    value: Asset::new(path),
                }
            }
            (shrimply_tts::InputDefinition::Toggle { .. }, TtsInputValue::Toggle { value }) => {
                shrimply_tts::TtsValue::Toggle { value }
            }
            (shrimply_tts::InputDefinition::Number { .. }, TtsInputValue::Number { value }) => {
                if value.denominator <= 0 {
                    return Err(format!(
                        "TTS numeric input {key:?} requires a positive denominator"
                    ));
                }
                shrimply_tts::TtsValue::Number {
                    value: fraction_new(value.numerator, value.denominator),
                }
            }
            (shrimply_tts::InputDefinition::Table { .. }, TtsInputValue::Table { rows }) => {
                shrimply_tts::TtsValue::Table { rows }
            }
            _ => return Err(format!("TTS input {key:?} has the wrong value type")),
        };
        settings.inputs.insert(key, value);
    }
    shrimply_tts::set_text(&mut settings, &model, request.text);
    let speech_request = shrimply_tts::speech_request(&model, &settings)?;
    let speech = shrimply_tts::synthesize(&server_url, &cancellation, &speech_request, |_| {
        !cancellation.is_cancelled()
    })?;
    let directory = project_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("media/tts");
    fs::create_dir_all(&directory)
        .map_err(|error| format!("could not create {}: {error}", directory.display()))?;
    let id = Uuid::new_v4();
    let staging = directory.join(format!(".{id}.staging.opus"));
    let final_path = directory.join(format!("{id}.opus"));
    if staging.exists() || final_path.exists() {
        return Err("generated TTS destination already exists".to_string());
    }
    let duration = shrimply_audio_engine::recording::save_wav_as_opus(&speech.wav, &staging)?;
    shrimply_tts::apply_speed_factor(&mut settings, &model, speech.speed_factor);
    Ok(GeneratedTts {
        speech: StagedSpeech {
            staging,
            final_path,
            promoted: false,
        },
        duration,
        settings,
    })
}
