use std::{collections::HashMap, path::PathBuf};

use shrimply_project_document::project::{AudioSource, ItemAddress};
use shrimply_tts::{TtsModel, TtsSettings};

use super::{GenerationEvent, GenerationTask, TtsChoice, TtsEditorControl, TtsInputEdit};
use crate::{InspectorController, InspectorTarget, model_catalog::ModelCatalog};

#[derive(Clone, Debug, PartialEq)]
pub struct EditorPresentation {
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
pub(crate) struct Runtime {
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
        let state = runtime
            .catalog
            .request(server_url, None, super::available_models);
        let can_retry = matches!(&state, Some(Err(_)));
        let loading = state.is_none();
        let (models, mut catalog_status) = match state {
            None => (Vec::new(), "Loading text-to-speech models…".to_string()),
            Some(Err(error)) => (Vec::new(), error),
            Some(Ok(models)) if models.is_empty() => {
                (models, "No text-to-speech models available".to_string())
            }
            Some(Ok(models)) => (models, String::new()),
        };
        let model = match settings.model.as_deref() {
            Some(id) => models.iter().find(|model| model.id == id),
            None => super::selected_model(&models, &settings, remembered),
        }
        .cloned();
        if model.is_none() && !models.is_empty() {
            catalog_status =
                "Saved model is unavailable. Choose a text-to-speech model.".to_string();
        }
        let controls = model.as_ref().map_or_else(Vec::new, |model| {
            super::editor_controls(&super::synchronized_settings(&settings, model), model)
        });
        Ok(EditorPresentation {
            models: models
                .iter()
                .map(|model| TtsChoice {
                    value: model.id.clone(),
                    label: model.label.clone(),
                })
                .collect(),
            model,
            controls,
            generated,
            loading,
            catalog_status,
            can_retry,
        })
    }

    pub fn select_tts_model(
        &self,
        target: &InspectorTarget,
        server_url: &str,
        id: &str,
    ) -> Result<(), String> {
        let models = self
            .tts_runtime
            .borrow_mut()
            .catalog
            .request(server_url, None, super::available_models)
            .ok_or_else(|| "Text-to-speech models are still loading".to_string())??;
        let model = models
            .iter()
            .find(|model| model.id == id)
            .ok_or_else(|| "Selected text-to-speech model is unavailable".to_string())?;
        let (settings, _) = self.tts_settings(target)?;
        self.set_tts_settings(
            target,
            super::synchronized_settings(&settings, model),
            false,
        )
    }

    pub fn edit_tts_input(
        &self,
        target: &InspectorTarget,
        model: &TtsModel,
        key: &str,
        edit: TtsInputEdit,
    ) -> Result<bool, String> {
        let mut settings = self.tts_model_settings(target, model)?;
        if model
            .inputs
            .iter()
            .find(|input| input.key() == key)
            .is_some_and(|input| !shrimply_tts::is_visible(input, &settings.inputs))
        {
            return Err("Text-to-speech input is no longer visible".to_string());
        }
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
