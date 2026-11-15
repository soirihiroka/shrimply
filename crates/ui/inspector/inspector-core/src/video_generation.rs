//! One model, input and generation owner used by every native inspector.
mod controls;
mod output;
mod worker;
use crate::{InspectorController, InspectorTarget, model_catalog::ModelCatalog};
use shrimply_video_generation::{
    InputDefinition, VideoGenerationModel, VideoGenerationSettings, VideoGenerationValue,
};
use std::{
    collections::{HashMap, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
};

pub const MODEL_PATH: &str = "/@video-generation/model";
const INPUT_PREFIX: &str = "/@video-generation/input/";
#[derive(Default)]
pub(crate) struct Runtime {
    catalog: ModelCatalog<VideoGenerationModel>,
    server: String,
    jobs: HashMap<InspectorTarget, Job>,
}
struct Job {
    task: Option<worker::Task>,
    settings: VideoGenerationSettings,
    inputs: Vec<shrimply_project_document::project::AssetSnapshot>,
    project: PathBuf,
    status: String,
    cancelling: bool,
}
fn signature(value: impl std::fmt::Debug) -> u64 {
    let mut hash = DefaultHasher::new();
    format!("{value:?}").hash(&mut hash);
    hash.finish()
}
fn models(server: &str) -> Result<Vec<VideoGenerationModel>, String> {
    let advertised = shrimply_compute_client::server_status(server)?
        .capabilities
        .into_iter()
        .filter_map(|capability| {
            capability
                .strip_prefix("video-generation:")
                .map(str::to_owned)
        })
        .collect::<Vec<_>>();
    if advertised.is_empty() {
        return Err("Server does not advertise video generation".into());
    }
    let models = shrimply_video_generation::models(server)?
        .into_iter()
        .filter(|model| advertised.contains(&model.id) && model.output.video)
        .collect::<Vec<_>>();
    if models.is_empty() {
        return Err("Server has no available video-generation models".into());
    }
    Ok(models)
}
impl InspectorController {
    fn video_generation_settings(
        &self,
        target: &InspectorTarget,
    ) -> Result<(VideoGenerationSettings, bool), String> {
        let InspectorTarget::Item(address) = target else {
            return Err("Video generation requires a video item".into());
        };
        let project = self.project.borrow();
        let item = project
            .video_item(address)
            .ok_or("Video-generation item is no longer available")?;
        Ok((
            item.video_generation
                .as_deref()
                .ok_or("Item no longer uses video generation")?
                .clone(),
            !item.file.as_os_str().is_empty(),
        ))
    }
    fn video_generation_model(
        &self,
        target: &InspectorTarget,
        expected: Option<u64>,
    ) -> Result<(VideoGenerationModel, VideoGenerationSettings), String> {
        let (mut settings, _) = self.video_generation_settings(target)?;
        let mut runtime = self.video_generation_runtime.borrow_mut();
        if runtime
            .jobs
            .get(target)
            .is_some_and(|job| job.task.is_some())
        {
            return Err("Video generation is running".into());
        }
        let server = runtime.server.clone();
        let models = runtime
            .catalog
            .request(&server, None, models)
            .ok_or("Video-generation models are still loading")??;
        let model = settings
            .model
            .as_ref()
            .and_then(|id| models.iter().find(|model| &model.id == id))
            .or_else(|| models.first())
            .ok_or("Select a video-generation model")?
            .clone();
        if expected.is_some_and(|expected| expected != signature(&model)) {
            return Err("Video-generation model changed; refresh the inspector".into());
        }
        shrimply_video_generation::sync_settings(&mut settings, &model);
        Ok((model, settings))
    }
    fn set_video_generation_settings(
        &self,
        target: &InspectorTarget,
        settings: VideoGenerationSettings,
        live: bool,
    ) -> Result<(), String> {
        let InspectorTarget::Item(address) = target else {
            return Err("Video generation requires a video item".into());
        };
        let mut project = self.project.borrow_mut();
        let item = project
            .video_item_mut(address)
            .ok_or("Video-generation item is no longer available")?;
        let current = item
            .video_generation
            .as_mut()
            .ok_or("Item no longer uses video generation")?;
        if **current == settings {
            return Ok(());
        }
        **current = settings;
        if live {
            shrimply_project_document::project::commit_coalesced_edit(
                &project,
                "edit-video-generation",
            );
        } else {
            shrimply_project_document::project::commit_edit(&project, "edit-video-generation");
        }
        drop(project);
        shrimply_editor_state::player_state::refresh_project(
            &self.player_state,
            shrimply_editor_state::player_state::ProjectChange {
                inspector: !live,
                ..Default::default()
            },
        );
        Ok(())
    }
    pub(crate) fn set_video_generation_field(
        &self,
        target: &InspectorTarget,
        path: &str,
        value: &str,
    ) -> Option<Result<(), String>> {
        if path != MODEL_PATH && !path.starts_with(INPUT_PREFIX) {
            return None;
        }
        Some((|| {
            let (model, mut settings) = self.video_generation_model(target, None)?;
            if path == MODEL_PATH {
                let mut runtime = self.video_generation_runtime.borrow_mut();
                let server = runtime.server.clone();
                let models = runtime
                    .catalog
                    .request(&server, None, models)
                    .ok_or("Video-generation models are still loading")??;
                let model = models
                    .iter()
                    .find(|model| model.id == value)
                    .ok_or("Video-generation model is unavailable")?;
                shrimply_video_generation::sync_settings(&mut settings, model);
                drop(runtime);
                return self.set_video_generation_settings(target, settings, false);
            }
            let (model_id, index) = path
                .strip_prefix(INPUT_PREFIX)
                .expect("generation input path")
                .split_once('/')
                .ok_or("Invalid generation input")?;
            if model_id.parse::<u64>().ok() != Some(signature(&model)) {
                return Err("Video-generation model changed; refresh the inspector".into());
            }
            let definition = model
                .inputs
                .get(
                    index
                        .parse::<usize>()
                        .map_err(|_| "Invalid generation input")?,
                )
                .ok_or("Video-generation input is unavailable")?;
            if !shrimply_video_generation::is_visible(definition, &settings.inputs) {
                return Err("Video-generation input is no longer visible".into());
            }
            let input = match definition {
                InputDefinition::Text { max_length, .. } => {
                    if value.chars().count() > *max_length {
                        return Err("Video-generation text exceeds the model limit".into());
                    }
                    VideoGenerationValue::Text {
                        value: value.into(),
                    }
                }
                InputDefinition::Select { options, .. } => {
                    if !options.iter().any(|option| option.value == value) {
                        return Err("Video-generation choice is unavailable".into());
                    }
                    VideoGenerationValue::Select {
                        value: value.into(),
                    }
                }
                InputDefinition::Number {
                    minimum,
                    maximum,
                    step,
                    ..
                } => {
                    let value: shrimply_math_core::Fraction = value
                        .parse()
                        .map_err(|_| "Invalid video-generation number")?;
                    if !shrimply_math_core::fraction_is_finite(value) {
                        return Err("Video-generation number must be finite".into());
                    }
                    VideoGenerationValue::Number {
                        value: shrimply_math_core::fraction_snapped(
                            shrimply_math_core::fraction_as_f64(value.clamp(*minimum, *maximum)),
                            *minimum,
                            *step,
                        ),
                    }
                }
                InputDefinition::Media { .. } => {
                    return Err("Media requires the native file picker".into());
                }
            };
            settings.inputs.insert(definition.key().into(), input);
            self.set_video_generation_settings(
                target,
                settings,
                !matches!(definition, InputDefinition::Select { .. }),
            )
        })())
    }
    pub(crate) fn video_generation_action(
        &self,
        target: &InspectorTarget,
        action: crate::InspectorControlAction,
    ) -> Result<(), String> {
        use crate::InspectorControlAction as A;
        match action {
            A::RetryVideoGenerationModels => {
                let mut runtime = self.video_generation_runtime.borrow_mut();
                let server = runtime.server.clone();
                runtime.catalog.retry_failed(&server);
            }
            A::CancelVideoGeneration => {
                let mut runtime = self.video_generation_runtime.borrow_mut();
                let job = runtime
                    .jobs
                    .get_mut(target)
                    .filter(|job| job.task.is_some())
                    .ok_or("Video generation is not running")?;
                job.task.as_ref().expect("active generation").cancel();
                job.cancelling = true;
                job.status = "Cancelling…".into();
            }
            A::GenerateVideo { model } => {
                let (model, settings) = self.video_generation_model(target, Some(model))?;
                self.finish_live_edit();
                self.set_video_generation_settings(target, settings.clone(), false)?;
                let mut runtime = self.video_generation_runtime.borrow_mut();
                let inputs = model
                    .inputs
                    .iter()
                    .filter(|input| shrimply_video_generation::is_visible(input, &settings.inputs))
                    .filter_map(|input| settings.inputs.get(input.key()))
                    .filter_map(|value| match value {
                        VideoGenerationValue::Media { items } => Some(items),
                        _ => None,
                    })
                    .flatten()
                    .map(|item| item.value.snapshot())
                    .collect::<Result<Vec<_>, _>>()?;
                let task = worker::Task::start(&runtime.server, model, settings.clone())?;
                runtime.jobs.insert(
                    target.clone(),
                    Job {
                        task: Some(task),
                        settings,
                        inputs,
                        project: shrimply_project_document::project::active_project_path(),
                        status: "Sending request…".into(),
                        cancelling: false,
                    },
                );
            }
            A::SelectVideoGenerationMedia { .. } => {
                return Err("Video-generation media requires a native file picker".into());
            }
            A::RemoveVideoGenerationMedia {
                model,
                input,
                index,
                items,
            } => self.edit_generation_media(target, model, input, Some(items), |values, _| {
                if index >= values.len() {
                    return Err("Video-generation media changed".into());
                }
                values.remove(index);
                Ok(())
            })?,
            A::MoveVideoGenerationMedia {
                model,
                input,
                index,
                offset,
                items,
            } => {
                self.edit_generation_media(target, model, input, Some(items), |values, ordered| {
                    if !ordered {
                        return Err("This media input is unordered".into());
                    }
                    let next = index
                        .checked_add_signed(offset)
                        .filter(|next| *next < values.len())
                        .ok_or("Video-generation media position is unavailable")?;
                    if index >= values.len() {
                        return Err("Video-generation media changed".into());
                    }
                    values.swap(index, next);
                    Ok(())
                })?
            }
            _ => return Err("Not a video-generation action".into()),
        }
        Ok(())
    }
    fn edit_generation_media(
        &self,
        target: &InspectorTarget,
        expected: u64,
        input: usize,
        expected_items: Option<u64>,
        edit: impl FnOnce(&mut Vec<shrimply_video_generation::MediaAsset>, bool) -> Result<(), String>,
    ) -> Result<(), String> {
        let (model, mut settings) = self.video_generation_model(target, Some(expected))?;
        let definition = model
            .inputs
            .get(input)
            .filter(|input| shrimply_video_generation::is_visible(input, &settings.inputs))
            .ok_or("Video-generation media input is unavailable")?;
        let InputDefinition::Media {
            accepted,
            maximum_items,
            ordered,
            ..
        } = definition
        else {
            return Err("Input does not accept media".into());
        };
        let Some(VideoGenerationValue::Media { items }) = settings.inputs.get_mut(definition.key())
        else {
            return Err("Video-generation media settings are invalid".into());
        };
        if expected_items.is_some_and(|expected| expected != signature(&items)) {
            return Err("Video-generation media changed; refresh the inspector".into());
        }
        edit(items, *ordered)?;
        if items.len() > *maximum_items || items.iter().any(|item| !accepted.contains(&item.kind)) {
            return Err("Video-generation media is not accepted by this input".into());
        }
        self.set_video_generation_settings(target, settings, false)
    }
    pub fn add_video_generation_media(
        &self,
        target: &InspectorTarget,
        model: u64,
        input: usize,
        path: &Path,
    ) -> Result<(), String> {
        let kind = shrimply_video_generation::media_kind(path)
            .ok_or("Unsupported video-generation media file")?;
        self.edit_generation_media(target, model, input, None, |items, _| {
            items.push(shrimply_video_generation::MediaAsset {
                kind,
                value: path.into(),
            });
            Ok(())
        })
    }
    pub(crate) fn poll_video_generation(&self) -> bool {
        let mut runtime = self.video_generation_runtime.borrow_mut();
        let mut changed = runtime.catalog.poll();
        for (target, job) in &mut runtime.jobs {
            let Some(task) = &mut job.task else { continue };
            let mut finished = false;
            while let Some(event) = task.poll() {
                match event {
                    worker::Event::Progress(message) => {
                        if !job.cancelling {
                            changed |= job.status != message;
                            job.status = message;
                        }
                    }
                    worker::Event::Done(result) => {
                        finished = true;
                        let cancelled = task.cancelled();
                        let result = result.and_then(|generated| {
                            if job.project
                                != shrimply_project_document::project::active_project_path()
                            {
                                return Err(
                                    "Generation finished after the active project changed".into()
                                );
                            }
                            for input in &job.inputs {
                                input.verify_current()?;
                            }
                            self.apply_generated_video(target, &job.settings, generated)
                        });
                        job.status = if cancelled {
                            "Cancelled".into()
                        } else {
                            result.map_or_else(|error| error, |()| "Generated".into())
                        };
                        job.cancelling = false;
                        changed = true;
                    }
                }
            }
            if finished {
                job.task = None;
            }
        }
        changed
    }
}
