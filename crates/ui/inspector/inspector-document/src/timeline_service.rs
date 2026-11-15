//! Shared configuration, jobs and native-control descriptions for timeline services.
mod presentation;
use serde_json::{Value, json};
use shrimply_export_core::{audio, job::Job};
use shrimply_inspector_core::tts;
use shrimply_preferences_core::{self as preferences, SharedPreferences};
use shrimply_timeline_skia::{
    ContextMenuRequest, caption_speech, scene::Scene, silence, transcription,
};
use shrimply_tts::{TtsModel, TtsSettings};
use std::{
    path::PathBuf,
    sync::{Arc, atomic::Ordering},
};

enum Operation {
    Speech {
        plan: caption_speech::Plan,
        models: Vec<TtsModel>,
        settings: TtsSettings,
    },
    Transcribe {
        models: Vec<String>,
        model: String,
        options: transcription::Options,
    },
    Silences(silence::Config),
    Audio {
        project: shrimply_project_document::Project,
        format: audio::Format,
    },
}
#[derive(Clone)]
enum Catalog {
    Speech(Vec<TtsModel>),
    Transcribe(Vec<String>),
}
enum Running {
    Speech(caption_speech::Handle),
    Transcribe(transcription::Handle),
    Audio(Job<PathBuf>),
}
pub struct Session {
    id: String,
    revision: u64,
    operation: Operation,
    server_url: String,
    catalog: shrimply_inspector_core::model_catalog::ModelCatalog<Catalog>,
    catalog_loading: bool,
    running: Option<Running>,
    stage: &'static str,
    message: String,
    error: Option<String>,
    directory: Option<PathBuf>,
    output: Option<Value>,
    cancelling: bool,
    close_pending: bool,
    references: Vec<shrimply_asset::AssetSnapshot>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl Session {
    pub fn open(
        request: ContextMenuRequest,
        scene: &Scene,
        preferences: &SharedPreferences,
        revision: u64,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Self, String> {
        let snapshot = preferences::snapshot(preferences);
        let operation = match request {
            ContextMenuRequest::GenerateSpeech => Operation::Speech {
                plan: scene.caption_speech_plan()?,
                models: Vec::new(),
                settings: TtsSettings::default(),
            },
            ContextMenuRequest::Transcribe => {
                scene.transcription_chunk_count(transcription::Options::default())?;
                Operation::Transcribe {
                    models: Vec::new(),
                    model: snapshot.last_stt_model.clone(),
                    options: Default::default(),
                }
            }
            ContextMenuRequest::RemoveSilences => Operation::Silences(Default::default()),
            ContextMenuRequest::ExportAudio => Operation::Audio {
                project: scene
                    .selected_audio_project()
                    .ok_or("No audio is selected.")?
                    .at_zero(),
                format: audio::Format::Wav,
            },
            _ => return Err("This action is not a timeline media service".into()),
        };
        let mut session = Self {
            id: uuid::Uuid::new_v4().to_string(),
            revision,
            operation,
            server_url: snapshot.compute_server_url,
            catalog: Default::default(),
            catalog_loading: false,
            running: None,
            stage: "configure",
            message: String::new(),
            error: None,
            directory: None,
            output: None,
            cancelling: false,
            close_pending: false,
            references: Vec::new(),
            wake,
        };
        if let Operation::Transcribe { options, .. } = &session.operation {
            session.message = shrimply_i18n::text_args(
                "%{count} chunks will be transcribed.",
                &[(
                    "count",
                    scene.transcription_chunk_count(*options)?.to_string(),
                )],
            );
        }
        if matches!(
            session.operation,
            Operation::Speech { .. } | Operation::Transcribe { .. }
        ) {
            session.load_catalog()?;
        }
        Ok(session)
    }

    fn catalog_result(&mut self) -> Option<Result<Vec<Catalog>, String>> {
        let load: fn(&str) -> Result<Vec<Catalog>, String> =
            if matches!(self.operation, Operation::Speech { .. }) {
                |url| caption_speech::models(url).map(|models| vec![Catalog::Speech(models)])
            } else {
                |url| transcription::models(url).map(|models| vec![Catalog::Transcribe(models)])
            };
        self.catalog
            .request_with_waker(&self.server_url, None, load, Some(self.wake.clone()))
    }

    fn load_catalog(&mut self) -> Result<(), String> {
        if self.server_url.trim().is_empty() {
            return Err("Compute server is not configured".into());
        }
        self.error = None;
        self.catalog.retry_failed(&self.server_url);
        self.catalog_loading = true;
        self.catalog_result();
        Ok(())
    }

    pub fn accepts(&self, request: &Value) -> Result<(), String> {
        if request["session"].as_str() != Some(self.id.as_str()) {
            return Err("The timeline service changed; open it again".into());
        }
        Ok(())
    }

    pub fn validate_file(&self, request: &Value) -> Result<(), String> {
        self.accepts(request)?;
        if self.stage != "configure" {
            return Err("The timeline service is running".into());
        }
        let Operation::Speech {
            models, settings, ..
        } = &self.operation
        else {
            return Err("This timeline service does not accept files".into());
        };
        let model = models
            .iter()
            .find(|model| settings.model.as_deref() == Some(model.id.as_str()))
            .ok_or("Text-to-speech model is required")?;
        let fields = tts::caption_editor_controls(settings, model);
        if request["action"] != "tts_audio" || request["model"].as_str() != Some(model.id.as_str())
        {
            return Err("Text-to-speech model changed".into());
        }
        let mut command = request.clone();
        command["path"] = Value::Null;
        crate::remote::tts::input_edit(&fields, "tts_audio", &command)?;
        Ok(())
    }

    /// Returns true only when the host may close the service surface.
    pub fn apply(
        &mut self,
        scene: &mut Scene,
        preferences: &SharedPreferences,
        revision: u64,
        request: &Value,
    ) -> Result<bool, String> {
        self.accepts(request)?;
        let action = request["action"]
            .as_str()
            .ok_or("Timeline service action is required")?;
        if matches!(action, "dismiss" | "close" | "saved") {
            if self.running.is_some() {
                self.close_pending = true;
                self.cancel();
                return Ok(false);
            }
            self.cleanup(Some(scene))?;
            return Ok(true);
        }
        if action == "cancel" {
            self.cancel();
            return Ok(false);
        }
        if self.stage != "configure" {
            return Err("The timeline service is running".into());
        }
        if revision != self.revision {
            return Err("The project changed while configuring this action".into());
        }
        self.error = None;
        if action == "retry" {
            self.load_catalog()?;
            return Ok(false);
        }
        if action == "submit" {
            self.submit(scene, preferences)?;
            return Ok(false);
        }
        match &mut self.operation {
            Operation::Speech {
                models, settings, ..
            } => {
                if action == "tts_model" {
                    let id = request["value"]
                        .as_str()
                        .ok_or("Text-to-speech model is required")?;
                    let model = models
                        .iter()
                        .find(|model| model.id == id)
                        .ok_or("Text-to-speech model is unavailable")?;
                    shrimply_tts::sync_settings(settings, model);
                    preferences::set_last_tts_model(preferences, id);
                } else {
                    let model = models
                        .iter()
                        .find(|model| settings.model.as_deref() == Some(model.id.as_str()))
                        .filter(|model| request["model"].as_str() == Some(model.id.as_str()))
                        .ok_or("Text-to-speech model changed")?;
                    let (key, edit) = crate::remote::tts::input_edit(
                        &tts::caption_editor_controls(settings, model),
                        action,
                        request,
                    )?;
                    tts::edit_input(settings, model, &key, edit)?;
                }
            }
            Operation::Transcribe {
                models,
                model,
                options,
            } => match request["key"].as_str() {
                Some("model") => {
                    let value = request["value"]
                        .as_str()
                        .ok_or("Speech-to-text model is required")?;
                    if !models.iter().any(|model| model == value) {
                        return Err("Speech-to-text model is unavailable".into());
                    }
                    *model = value.into();
                    preferences::set_last_stt_model(preferences, value);
                }
                Some(
                    key @ ("chunked" | "snap_source" | "snap_tolerance" | "continue_threshold"),
                ) => {
                    let value = match request.get("value") {
                        Some(Value::Bool(value)) => value.to_string(),
                        Some(Value::String(value)) => value.clone(),
                        _ => presentation::editor_value(request)?.to_owned(),
                    };
                    options.set_field(key, &value)?;
                }
                _ => return Err("Unknown transcription field".into()),
            },
            Operation::Silences(config) => config.set_field(
                request["key"].as_str().ok_or("Silence field is required")?,
                presentation::editor_value(request)?,
            )?,
            Operation::Audio { format, .. } => {
                let extension = request["value"]
                    .as_str()
                    .ok_or("Audio format is required")?;
                *format = audio::Format::ALL
                    .into_iter()
                    .find(|format| format.extension() == extension)
                    .ok_or("Unknown audio format")?;
            }
        }
        self.cleanup_references(Some(scene), true)?;
        if let Operation::Transcribe { options, .. } = &self.operation {
            let chunks = scene.transcription_chunk_count(*options)?;
            self.message = shrimply_i18n::text_args(
                "%{count} chunks will be transcribed.",
                &[("count", chunks.to_string())],
            );
        }
        Ok(false)
    }

    fn submit(&mut self, scene: &mut Scene, preferences: &SharedPreferences) -> Result<(), String> {
        if self.catalog_loading {
            return Err("Wait for the model list to finish loading".into());
        }
        self.message.clear();
        self.running = match &self.operation {
            Operation::Speech {
                plan,
                models,
                settings,
            } => {
                let model = models
                    .iter()
                    .find(|model| settings.model.as_deref() == Some(model.id.as_str()))
                    .ok_or("Text-to-speech model is required")?;
                preferences::set_last_tts_model(preferences, &model.id);
                Some(Running::Speech(scene.start_caption_speech(
                    plan.clone(),
                    caption_speech::Options {
                        server_url: self.server_url.clone(),
                        model: model.clone(),
                        settings: settings.clone(),
                    },
                )?))
            }
            Operation::Transcribe { model, options, .. } => Some(Running::Transcribe(
                scene.start_transcription(*options, self.server_url.clone(), model.clone())?,
            )),
            Operation::Silences(config) => {
                let removed = scene.remove_silences(*config)?;
                self.message = if removed == 0 {
                    "No selected-audio gaps matched the configured threshold and duration.".into()
                } else {
                    shrimply_i18n::text_args(
                        "Removed %{count} silence ranges",
                        &[("count", removed.to_string())],
                    )
                };
                None
            }
            Operation::Audio { project, format } => {
                let directory = shrimply_path_core::cache_directory()?
                    .join("exports")
                    .join(&self.id);
                std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
                let path = directory.join(format!("selected-audio.{}", format.extension()));
                self.directory = Some(directory);
                let project = project.clone();
                let format = *format;
                let wake = self.wake.clone();
                Some(Running::Audio(Job::spawn(
                    move |cancelled, progress| {
                        audio::export_with_progress(&project, &path, format, |update| {
                            progress(match update {
                                audio::ExportProgress::Mixing {
                                    completed_frames,
                                    total_frames,
                                } => shrimply_export_core::video::ExportProgress::MixingAudio {
                                    current_frame: completed_frames,
                                    total_frames,
                                },
                                audio::ExportProgress::Encoding {
                                    completed_frames,
                                    total_frames,
                                } => shrimply_export_core::video::ExportProgress::EncodingAudio {
                                    current_frame: completed_frames,
                                    total_frames,
                                },
                            });
                            !cancelled.load(Ordering::Acquire)
                        })?;
                        Ok(path)
                    },
                    move || wake(),
                )?))
            }
        };
        self.stage = if self.running.is_some() {
            "running"
        } else {
            "finished"
        };
        Ok(())
    }

    fn cancel(&mut self) {
        self.cancelling = true;
        if let Some(running) = &self.running {
            match running {
                Running::Speech(handle) => handle.cancel(),
                Running::Transcribe(handle) => handle.cancel(),
                Running::Audio(job) => job.cancel(),
            }
        }
    }

    pub fn poll(&mut self, scene: &mut Scene, preferences: &SharedPreferences) -> bool {
        let mut changed = false;
        if self.catalog_loading {
            self.catalog.poll();
        }
        if self.catalog_loading
            && let Some(result) = self.catalog_result()
        {
            match (
                result.map(|mut models| {
                    assert_eq!(models.len(), 1, "timeline service catalog response");
                    models.pop().expect("catalog response")
                }),
                &mut self.operation,
            ) {
                (
                    Ok(Catalog::Speech(available)),
                    Operation::Speech {
                        models, settings, ..
                    },
                ) => {
                    *models = available;
                    if let Some(model) = tts::selected_model(
                        models,
                        settings,
                        &preferences::snapshot(preferences).last_tts_model,
                    ) {
                        shrimply_tts::sync_settings(settings, model);
                    }
                }
                (
                    Ok(Catalog::Transcribe(available)),
                    Operation::Transcribe { models, model, .. },
                ) => {
                    *models = available;
                    if !models.contains(model) {
                        *model = models.first().cloned().unwrap_or_default();
                    }
                }
                (Err(error), _) => self.error = Some(error),
                _ => unreachable!("timeline service catalog kind"),
            }
            self.catalog_loading = false;
            changed = true;
        }
        let mut finished = None;
        match &mut self.running {
            Some(Running::Speech(_)) => {
                while let Some(update) = scene.take_caption_speech_update() {
                    changed = true;
                    match update {
                        caption_speech::Update::Progress {
                            current,
                            total,
                            message,
                            preview,
                        } => self.message = format!("{current}/{total} · {message}\n{preview}"),
                        caption_speech::Update::Finished(summary) => {
                            finished = Some(Ok(summary.to_string()))
                        }
                        caption_speech::Update::Failed(error) => finished = Some(Err(error)),
                    }
                }
            }
            Some(Running::Transcribe(_)) => {
                while let Some(update) = scene.take_transcription_update() {
                    changed = true;
                    match update {
                        transcription::Update::Progress(message) => self.message = message,
                        transcription::Update::Finished { captions } => {
                            finished = Some(Ok(shrimply_i18n::text_args(
                                "Created %{count} captions",
                                &[("count", captions.to_string())],
                            )))
                        }
                        transcription::Update::Cancelled => {
                            finished = Some(Ok("Transcription cancelled".into()))
                        }
                        transcription::Update::Failed(error) => finished = Some(Err(error)),
                    }
                }
            }
            Some(Running::Audio(job)) => {
                let update = job.poll();
                changed |= update.progress.is_some() || update.finished.is_some();
                if let Some(progress) = update.progress {
                    let presentation =
                        shrimply_export_core::video::progress::presentation(progress);
                    self.message = shrimply_i18n::text_args(
                        presentation.message,
                        &presentation.arguments.into_iter().collect::<Vec<_>>(),
                    );
                }
                if let Some(result) = update.finished {
                    finished = Some(result.map(|path| {
                    self.output = Some(json!({"path":path,"filename":path.file_name().and_then(|name| name.to_str()).expect("audio export filename"),"mime":match &self.operation { Operation::Audio { format, .. } => format.mime_type(), _ => unreachable!("audio export operation") }}));
                    "Audio exported".into()
                }));
                }
            }
            None => {}
        }
        if let Some(result) = finished {
            self.running = None;
            match result {
                Ok(message) => {
                    self.message = message;
                    self.stage = "finished";
                }
                Err(error) => {
                    self.error = Some(error);
                    self.stage = "failed";
                }
            }
        }
        changed
    }

    pub fn set_error(&mut self, error: String) {
        self.error = Some(error);
    }
    pub fn snapshot(&self) -> Value {
        presentation::snapshot(self)
    }
    pub fn ready_to_close(&self) -> bool {
        self.close_pending && self.running.is_none()
    }

    pub fn retain_reference(&mut self, snapshot: shrimply_asset::AssetSnapshot) {
        self.references.push(snapshot);
    }

    fn cleanup_references(
        &mut self,
        scene: Option<&Scene>,
        keep_settings: bool,
    ) -> Result<(), String> {
        let settings = match &self.operation {
            Operation::Speech { settings, .. } if keep_settings => Some(settings),
            _ => None,
        };
        let mut kept = Vec::new();
        let mut pending = std::mem::take(&mut self.references).into_iter();
        while let Some(snapshot) = pending.next() {
            if scene.is_some_and(|scene| scene.references_asset(snapshot.path())) {
                continue;
            }
            if settings.is_some_and(|settings| settings.inputs.values().any(|input| matches!(input, shrimply_tts::TtsValue::Audio { value } if value.path() == snapshot.path()))) { kept.push(snapshot); continue; }
            if snapshot.verify_current().is_ok() {
                match std::fs::remove_file(snapshot.path()) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => {
                        kept.push(snapshot);
                        kept.extend(pending);
                        self.references = kept;
                        return Err(error.to_string());
                    }
                }
            }
        }
        self.references = kept;
        Ok(())
    }

    pub fn cleanup(&mut self, scene: Option<&Scene>) -> Result<(), String> {
        self.cancel();
        self.running = None;
        self.catalog_loading = false;
        self.cleanup_references(scene, false)?;
        if let Some(directory) = self.directory.take() {
            std::fs::remove_dir_all(directory).map_err(|error| error.to_string())?;
        }
        Ok(())
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.cancel();
    }
}
