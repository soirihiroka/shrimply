use std::{collections::HashMap, path::PathBuf};

use shrimply_project_document::project::{AudioSource, ItemAddress};
use shrimply_tts::{TtsModel, TtsSettings};

use super::{GenerationEvent, GenerationTask, TtsChoice, TtsEditorControl, TtsInputEdit};
use crate::{InspectorController, InspectorTarget, model_catalog::ModelCatalog};

#[derive(Clone, Debug, PartialEq)]
pub struct EditorPresentation {
    pub server_url: String,
    pub generation: GenerationPresentation,
    pub models: Vec<TtsChoice>,
    pub model: Option<TtsModel>,
    pub controls: Vec<TtsEditorControl>,
    pub generated: bool,
    pub loading: bool,
    pub catalog_status: String,
    pub can_retry: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GenerationPresentation {
    pub running: bool,
    pub cancelling: bool,
    pub status: String,
}

struct Job {
    task: Option<GenerationTask>,
    model: TtsModel,
    project_path: PathBuf,
    presentation: GenerationPresentation,
}

#[derive(Default)]
pub struct Runtime {
    catalog: ModelCatalog<TtsModel>,
    jobs: HashMap<InspectorTarget, Job>,
}

impl InspectorController {
    pub fn tts_editor(
        &self,
        target: &InspectorTarget,
        server_url: &str,
        remembered: &str,
    ) -> Result<EditorPresentation, String> {
        let (settings, generated) = self.tts_settings(target)?;
        let mut runtime = self.tts_runtime.borrow_mut();
        runtime
            .jobs
            .retain(|owner, job| owner == target || job.task.is_some());
        let mut configured = settings.clone();
        let mut presentation = runtime.configuration(
            &mut configured,
            server_url,
            remembered,
            ConfigurationKind::Regular,
        );
        presentation.generated = generated;
        if let Some(job) = runtime.jobs.get(target) {
            presentation.generation = job.presentation.clone();
            if job.task.is_some() {
                // An active request keeps its original model when server preferences change.
                presentation.models = vec![TtsChoice {
                    value: job.model.id.clone(),
                    label: job.model.label.clone(),
                }];
                presentation.controls = super::editor_controls(
                    &super::synchronized_settings(&settings, &job.model),
                    &job.model,
                );
                presentation.model = Some(job.model.clone());
                presentation.loading = false;
                presentation.catalog_status.clear();
                presentation.can_retry = false;
            }
        }
        Ok(presentation)
    }

    pub fn select_tts_model(
        &self,
        target: &InspectorTarget,
        server_url: &str,
        id: &str,
    ) -> Result<(), String> {
        self.ensure_tts_editable(target)?;
        let (mut settings, _) = self.tts_settings(target)?;
        self.tts_runtime.borrow_mut().select_configuration_model(
            &mut settings,
            server_url,
            id,
            ConfigurationKind::Regular,
        )?;
        self.set_tts_settings(target, settings, false)
    }

    pub fn edit_tts_input(
        &self,
        target: &InspectorTarget,
        model: &TtsModel,
        key: &str,
        edit: TtsInputEdit,
    ) -> Result<bool, String> {
        self.ensure_tts_editable(target)?;
        let mut settings = self.tts_model_settings(target, model)?;
        let live = super::edit_input(&mut settings, model, key, edit)?;
        self.set_tts_settings(target, settings, live)?;
        Ok(live)
    }

    pub fn retry_tts_models(&self, server_url: &str) {
        self.tts_runtime
            .borrow_mut()
            .catalog
            .retry_failed(server_url);
    }

    pub fn tts_generation(&self, target: &InspectorTarget) -> GenerationPresentation {
        self.tts_runtime
            .borrow()
            .jobs
            .get(target)
            .map(|job| job.presentation.clone())
            .unwrap_or_default()
    }

    pub fn cancel_tts_generation(&self, target: &InspectorTarget) -> Result<(), String> {
        let mut runtime = self.tts_runtime.borrow_mut();
        let job = runtime
            .jobs
            .get_mut(target)
            .filter(|job| job.task.is_some())
            .ok_or("Text-to-speech generation is not running")?;
        job.task.as_ref().expect("active generation").cancel();
        job.presentation.cancelling = true;
        job.presentation.status = "Cancelling…".to_string();
        Ok(())
    }

    pub fn toggle_tts_generation(
        &self,
        target: &InspectorTarget,
        server_url: &str,
        model: &TtsModel,
    ) -> Result<(), String> {
        let mut runtime = self.tts_runtime.borrow_mut();
        if runtime
            .jobs
            .get(target)
            .is_some_and(|job| job.task.is_some())
        {
            drop(runtime);
            return self.cancel_tts_generation(target);
        }
        let settings = self.tts_model_settings(target, model)?;
        self.set_tts_settings(target, settings.clone(), false)?;
        let task = GenerationTask::start(server_url, model.clone(), settings)?;
        runtime.jobs.insert(
            target.clone(),
            Job {
                task: Some(task),
                model: model.clone(),
                project_path: shrimply_project_document::project::active_project_path(),
                presentation: GenerationPresentation {
                    running: true,
                    cancelling: false,
                    status: "Sending request…".to_string(),
                },
            },
        );
        Ok(())
    }

    pub(crate) fn poll_tts(&self) -> bool {
        let mut runtime = self.tts_runtime.borrow_mut();
        let mut changed = runtime.catalog.poll();
        for (target, job) in &mut runtime.jobs {
            let Some(task) = &mut job.task else { continue };
            while let Some(event) = task.poll() {
                match event {
                    GenerationEvent::Progress(status) => {
                        if !job.presentation.cancelling {
                            changed |= job.presentation.status != status;
                            job.presentation.status = status;
                        }
                    }
                    GenerationEvent::Done(result) => {
                        let cancelled = task.is_cancelled();
                        let result = result.and_then(|generation| {
                            if job.project_path
                                != shrimply_project_document::project::active_project_path()
                            {
                                super::remove_generated(&generation.path);
                                return Err("Generation finished after the active project changed"
                                    .to_string());
                            }
                            let InspectorTarget::Item(address) = target else {
                                unreachable!("TTS generation target must be an item")
                            };
                            self.apply_tts_generation(
                                target,
                                address.item_id(),
                                &job.model,
                                generation,
                            )
                        });
                        job.presentation = GenerationPresentation {
                            running: false,
                            cancelling: false,
                            status: if cancelled {
                                "Cancelled".to_string()
                            } else {
                                result.map_or_else(|error| error, |()| "Generated".to_string())
                            },
                        };
                        changed = true;
                    }
                }
            }
            if !job.presentation.running {
                job.task = None;
            }
        }
        changed
    }

    fn ensure_tts_editable(&self, target: &InspectorTarget) -> Result<(), String> {
        if self.tts_generation(target).running {
            return Err("Text-to-speech generation is running".into());
        }
        Ok(())
    }

    fn tts_settings(&self, target: &InspectorTarget) -> Result<(TtsSettings, bool), String> {
        let InspectorTarget::Item(address @ ItemAddress::Audio { .. }) = target else {
            return Err("Text-to-speech target is not an audio item".to_string());
        };
        let project = self.project.borrow();
        let item = project
            .audio_item(address)
            .ok_or_else(|| "Text-to-speech item is no longer available".to_string())?;
        let AudioSource::Tts(settings) = &item.source else {
            return Err("Audio item no longer uses text to speech".to_string());
        };
        Ok(((**settings).clone(), !item.file.as_os_str().is_empty()))
    }

    fn tts_model_settings(
        &self,
        target: &InspectorTarget,
        model: &TtsModel,
    ) -> Result<TtsSettings, String> {
        let (settings, _) = self.tts_settings(target)?;
        if settings.model.as_deref().is_some_and(|id| id != model.id) {
            return Err("Text-to-speech model changed; edit the current inspector".to_string());
        }
        Ok(super::synchronized_settings(&settings, model))
    }
}

#[derive(Clone, Copy)]
pub enum ConfigurationKind {
    Regular,
    Caption,
}

pub struct EditorActions<'a> {
    pub model_sensitive: bool,
    pub inputs_sensitive: bool,
    pub generate_sensitive: bool,
    pub generate_visible: bool,
    pub generate_label: &'static str,
    pub status: &'a str,
    pub busy: bool,
}

impl EditorPresentation {
    pub fn actions(&self) -> EditorActions<'_> {
        EditorActions {
            model_sensitive: !self.generation.running && !self.models.is_empty(),
            inputs_sensitive: !self.generation.running,
            generate_sensitive: self.model.is_some() && !self.generation.cancelling,
            generate_visible: self.model.is_some(),
            generate_label: if self.generation.running {
                super::CANCEL_LABEL
            } else if self.generated {
                super::REGENERATE_LABEL
            } else {
                super::GENERATE_LABEL
            },
            status: if !self.generation.status.is_empty() {
                &self.generation.status
            } else if !self.catalog_status.is_empty() {
                &self.catalog_status
            } else {
                super::READY_STATUS
            },
            busy: self.loading || self.generation.running,
        }
    }
}

impl Runtime {
    pub fn poll_catalog(&mut self) -> bool {
        self.catalog.poll()
    }
    pub fn retry_models(&mut self, server_url: &str) {
        self.catalog.retry_failed(server_url);
    }

    pub fn configuration(
        &mut self,
        settings: &mut TtsSettings,
        server_url: &str,
        remembered: &str,
        kind: ConfigurationKind,
    ) -> EditorPresentation {
        let state = self
            .catalog
            .request(server_url, None, super::available_models);
        let can_retry = matches!(&state, Some(Err(_)));
        let loading = state.is_none();
        let (mut models, mut catalog_status) = match state {
            None => (Vec::new(), "Loading text-to-speech models…".to_string()),
            Some(Err(error)) => (Vec::new(), error),
            Some(Ok(models)) => (models, String::new()),
        };
        if matches!(kind, ConfigurationKind::Caption) {
            models.retain(shrimply_tts::is_caption_model);
        }
        if !loading && !can_retry && models.is_empty() {
            catalog_status = match kind {
                ConfigurationKind::Regular => "No text-to-speech models available",
                ConfigurationKind::Caption => {
                    "The server does not provide a text-to-speech model with caption timing"
                }
            }
            .to_string();
        }
        let model = match (&settings.model, kind) {
            (Some(id), ConfigurationKind::Regular) => models.iter().find(|model| model.id == *id),
            _ => super::selected_model(&models, settings, remembered),
        }
        .cloned();
        if let Some(model) = &model {
            *settings = super::synchronized_settings(settings, model);
        } else if !models.is_empty() {
            catalog_status =
                "Saved model is unavailable. Choose a text-to-speech model.".to_string();
        }
        let controls = model.as_ref().map_or_else(Vec::new, |model| match kind {
            ConfigurationKind::Regular => super::editor_controls(settings, model),
            ConfigurationKind::Caption => super::caption_editor_controls(settings, model),
        });
        EditorPresentation {
            server_url: server_url.to_owned(),
            generation: Default::default(),
            models: models
                .iter()
                .map(|model| TtsChoice {
                    value: model.id.clone(),
                    label: model.label.clone(),
                })
                .collect(),
            model,
            controls,
            generated: false,
            loading,
            catalog_status,
            can_retry,
        }
    }

    pub fn select_configuration_model(
        &mut self,
        settings: &mut TtsSettings,
        server_url: &str,
        id: &str,
        kind: ConfigurationKind,
    ) -> Result<(), String> {
        let models = self
            .catalog
            .request(server_url, None, super::available_models)
            .ok_or("Text-to-speech models are still loading")??;
        let model = models
            .iter()
            .find(|model| {
                model.id == id
                    && (matches!(kind, ConfigurationKind::Regular)
                        || shrimply_tts::is_caption_model(model))
            })
            .ok_or("Selected text-to-speech model is unavailable")?;
        *settings = super::synchronized_settings(settings, model);
        Ok(())
    }
}
