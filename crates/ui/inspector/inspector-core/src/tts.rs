pub(crate) mod editor;
mod generation;
pub use editor::{EditorPresentation, GenerationPresentation};
pub use generation::{GenerationEvent, GenerationTask};

use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    sync::{Mutex, OnceLock},
};

use shrimply_editor_state::player_state;
use shrimply_math_core::{
    fraction_as_f64, fraction_denominator, fraction_numerator, fraction_snapped,
};
use shrimply_project_document::project::{AudioSource, ItemAddress, Time, TrackRef};
use shrimply_tts::{
    Fraction, InputDefinition, Speech, TableColumn, TtsModel, TtsSettings, TtsValue, is_visible,
};

use crate::{InspectorController, InspectorTarget};

static MODEL_CATALOGS: OnceLock<Mutex<HashMap<String, Vec<TtsModel>>>> = OnceLock::new();

pub struct TtsGeneration {
    pub path: PathBuf,
    pub duration: Time,
    pub speed_factor: Fraction,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TtsChoice {
    pub value: String,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TtsEditorControl {
    Text {
        key: String,
        label: String,
        value: String,
        multiline: bool,
        max_length: usize,
    },
    Select {
        key: String,
        label: String,
        value: String,
        choices: Vec<TtsChoice>,
    },
    Audio {
        key: String,
        label: String,
        path: Option<PathBuf>,
    },
    Toggle {
        key: String,
        label: String,
        value: bool,
    },
    Number {
        key: String,
        label: String,
        value: Fraction,
        minimum: Fraction,
        maximum: Fraction,
        step: Fraction,
        digits: usize,
    },
    Table {
        key: String,
        label: String,
        columns: Vec<TableColumn>,
        rows: Vec<BTreeMap<String, String>>,
    },
}

pub enum TtsInputEdit {
    Text(String),
    Select(String),
    Audio(Option<PathBuf>),
    Toggle(bool),
    Number(Fraction),
    TableCell {
        row: usize,
        column: usize,
        value: String,
    },
    AddTableRow,
    RemoveTableRow(usize),
}

pub fn available_models(server_url: &str) -> Result<Vec<TtsModel>, String> {
    let catalogs = MODEL_CATALOGS.get_or_init(Default::default);
    if let Some(models) = catalogs
        .lock()
        .expect("TTS model catalog cache lock is poisoned")
        .get(server_url)
        .cloned()
    {
        return Ok(models);
    }
    let advertised = shrimply_compute_client::server_status(server_url)?
        .capabilities
        .into_iter()
        .filter_map(|capability| capability.strip_prefix("tts:").map(str::to_string))
        .collect::<Vec<_>>();
    if advertised.is_empty() {
        return Err("Server does not advertise text-to-speech support".to_string());
    }
    let mut models = shrimply_tts::models(server_url)?;
    models.retain(|model| advertised.contains(&model.id));
    catalogs
        .lock()
        .expect("TTS model catalog cache lock is poisoned")
        .insert(server_url.to_string(), models.clone());
    Ok(models)
}

pub fn selected_model<'a>(
    models: &'a [TtsModel],
    settings: &TtsSettings,
    remembered: &str,
) -> Option<&'a TtsModel> {
    settings
        .model
        .as_ref()
        .and_then(|id| models.iter().find(|model| model.id == *id))
        .or_else(|| models.iter().find(|model| model.id == remembered))
        .or_else(|| models.first())
}

pub fn synchronized_settings(settings: &TtsSettings, model: &TtsModel) -> TtsSettings {
    let mut settings = settings.clone();
    shrimply_tts::sync_settings(&mut settings, model);
    settings
}

pub fn editor_controls(settings: &TtsSettings, model: &TtsModel) -> Vec<TtsEditorControl> {
    model
        .inputs
        .iter()
        .filter(|definition| is_visible(definition, &settings.inputs))
        .map(|definition| match definition {
            InputDefinition::Text {
                key,
                label,
                default,
                multiline,
                max_length,
                ..
            } => TtsEditorControl::Text {
                key: key.clone(),
                label: label.clone(),
                value: match settings.inputs.get(key) {
                    Some(TtsValue::Text { value }) => value.clone(),
                    _ => default.clone(),
                },
                multiline: *multiline,
                max_length: *max_length,
            },
            InputDefinition::Select {
                key,
                label,
                options,
                default,
                ..
            } => TtsEditorControl::Select {
                key: key.clone(),
                label: label.clone(),
                value: match settings.inputs.get(key) {
                    Some(TtsValue::Select { value }) => value.clone(),
                    _ => default.clone(),
                },
                choices: options
                    .iter()
                    .map(|option| TtsChoice {
                        value: option.value.clone(),
                        label: option.label.clone(),
                    })
                    .collect(),
            },
            InputDefinition::Audio { key, label, .. } => TtsEditorControl::Audio {
                key: key.clone(),
                label: label.clone(),
                path: match settings.inputs.get(key) {
                    Some(TtsValue::Audio { value }) => Some(value.path().to_path_buf()),
                    _ => None,
                },
            },
            InputDefinition::Toggle {
                key,
                label,
                default,
                ..
            } => TtsEditorControl::Toggle {
                key: key.clone(),
                label: label.clone(),
                value: match settings.inputs.get(key) {
                    Some(TtsValue::Toggle { value }) => *value,
                    _ => *default,
                },
            },
            InputDefinition::Number {
                key,
                label,
                default,
                minimum,
                maximum,
                step,
                ..
            } => TtsEditorControl::Number {
                key: key.clone(),
                label: label.clone(),
                value: match settings.inputs.get(key) {
                    Some(TtsValue::Number { value }) => *value,
                    _ => *default,
                },
                minimum: *minimum,
                maximum: *maximum,
                step: *step,
                digits: decimal_places(*step),
            },
            InputDefinition::Table {
                key,
                label,
                columns,
                ..
            } => TtsEditorControl::Table {
                key: key.clone(),
                label: label.clone(),
                columns: columns.clone(),
                rows: match settings.inputs.get(key) {
                    Some(TtsValue::Table { rows }) => rows.clone(),
                    _ => Vec::new(),
                },
            },
        })
        .collect()
}

pub fn edit_input(
    settings: &mut TtsSettings,
    model: &TtsModel,
    key: &str,
    edit: TtsInputEdit,
) -> Result<bool, String> {
    let definition = model
        .inputs
        .iter()
        .find(|definition| definition.key() == key)
        .ok_or_else(|| "TTS input is unavailable".to_string())?;
    let (value, live) = match (definition, edit) {
        (InputDefinition::Text { max_length, .. }, TtsInputEdit::Text(value)) => {
            if value.chars().count() > *max_length {
                return Err("TTS text exceeds the model limit".to_string());
            }
            (Some(TtsValue::Text { value }), true)
        }
        (InputDefinition::Select { options, .. }, TtsInputEdit::Select(value)) => {
            if options.iter().all(|option| option.value != value) {
                return Err("Selected TTS option is unavailable".to_string());
            }
            (Some(TtsValue::Select { value }), false)
        }
        (InputDefinition::Audio { .. }, TtsInputEdit::Audio(value)) => (
            value.map(|value| TtsValue::Audio {
                value: value.into(),
            }),
            false,
        ),
        (InputDefinition::Toggle { .. }, TtsInputEdit::Toggle(value)) => {
            (Some(TtsValue::Toggle { value }), false)
        }
        (
            InputDefinition::Number {
                minimum,
                maximum,
                step,
                ..
            },
            TtsInputEdit::Number(value),
        ) => {
            let value = value.clamp(*minimum, *maximum);
            let value = fraction_snapped(fraction_as_f64(value), *minimum, *step);
            (Some(TtsValue::Number { value }), true)
        }
        (
            InputDefinition::Table { columns, .. },
            TtsInputEdit::TableCell { row, column, value },
        ) => {
            let column = columns
                .get(column)
                .ok_or_else(|| "TTS table column is unavailable".to_string())?;
            if value.chars().count() > column.max_length {
                return Err("TTS table value exceeds the column limit".to_string());
            }
            let Some(TtsValue::Table { rows }) = settings.inputs.get_mut(key) else {
                return Err("TTS table settings are invalid".to_string());
            };
            rows.get_mut(row)
                .ok_or_else(|| "TTS table row is unavailable".to_string())?
                .insert(column.key.clone(), value);
            return Ok(true);
        }
        (InputDefinition::Table { .. }, TtsInputEdit::AddTableRow) => {
            let Some(TtsValue::Table { rows }) = settings.inputs.get_mut(key) else {
                return Err("TTS table settings are invalid".to_string());
            };
            rows.push(BTreeMap::new());
            return Ok(false);
        }
        (InputDefinition::Table { .. }, TtsInputEdit::RemoveTableRow(row)) => {
            let Some(TtsValue::Table { rows }) = settings.inputs.get_mut(key) else {
                return Err("TTS table settings are invalid".to_string());
            };
            if row >= rows.len() {
                return Err("TTS table row is unavailable".to_string());
            }
            rows.remove(row);
            return Ok(false);
        }
        _ => return Err("TTS input edit does not match its model definition".to_string()),
    };
    match value {
        Some(value) => {
            settings.inputs.insert(key.to_string(), value);
        }
        None => {
            settings.inputs.remove(key);
        }
    }
    Ok(live)
}

pub fn decimal_places(step: Fraction) -> usize {
    let mut scaled = fraction_numerator(step).unsigned_abs();
    let denominator = fraction_denominator(step).unsigned_abs();
    for digits in 0..=6 {
        if scaled.is_multiple_of(denominator) {
            return digits;
        }
        scaled = scaled.saturating_mul(10);
    }
    6
}

fn generate_in(
    directory: &std::path::Path,
    server_url: &str,
    cancellation: &shrimply_compute_client::CancellationToken,
    model: &TtsModel,
    settings: &TtsSettings,
    progress: impl Fn(&str) -> bool,
) -> Result<TtsGeneration, String> {
    let request = shrimply_tts::speech_request(model, settings)?;
    let speech = shrimply_tts::synthesize(server_url, cancellation, &request, progress)?;
    save_speech_in(speech, directory)
}

pub fn save_speech(speech: Speech) -> Result<TtsGeneration, String> {
    save_speech_in(
        speech,
        &shrimply_project_document::project::project_directory(),
    )
}

fn save_speech_in(
    speech: Speech,
    project_directory: &std::path::Path,
) -> Result<TtsGeneration, String> {
    let directory = project_directory.join("media/tts");
    let path = directory.join(format!("{}.opus", uuid::Uuid::new_v4()));
    shrimply_audio_engine::recording::save_wav_as_opus(&speech.wav, &path).map(|duration| {
        TtsGeneration {
            path,
            duration,
            speed_factor: speech.speed_factor,
        }
    })
}

impl InspectorController {
    pub fn set_tts_settings(
        &self,
        target: &InspectorTarget,
        settings: TtsSettings,
        live: bool,
    ) -> Result<(), String> {
        let InspectorTarget::Item(address @ ItemAddress::Audio { .. }) = target else {
            return Err("TTS settings target is no longer an audio item".to_string());
        };
        let mut project = self.project.borrow_mut();
        let item = project
            .audio_item_mut(address)
            .ok_or_else(|| "TTS settings item is no longer available".to_string())?;
        let AudioSource::Tts(current) = &mut item.source else {
            return Err("TTS settings item no longer uses text to speech".to_string());
        };
        if **current == settings {
            return Ok(());
        }
        **current = settings;
        if live {
            shrimply_project_document::project::commit_coalesced_edit(&project, "edit-tts");
        } else {
            shrimply_project_document::project::commit_edit(&project, "edit-tts");
        }
        Ok(())
    }

    pub fn apply_tts_generation(
        &self,
        target: &InspectorTarget,
        audio_id: uuid::Uuid,
        model: &TtsModel,
        generation: TtsGeneration,
    ) -> Result<(), String> {
        let InspectorTarget::Item(address @ ItemAddress::Audio { .. }) = target else {
            remove_generated(&generation.path);
            return Err("TTS generation target is no longer an audio item".to_string());
        };
        let mut project = self.project.borrow_mut();
        let track_address = address.track();
        let item_id = address.item_id();
        let Some(TrackRef::Audio(track)) = project.track(&track_address) else {
            drop(project);
            remove_generated(&generation.path);
            return Err("TTS generation track is no longer available".to_string());
        };
        let Some(start) = track
            .items
            .iter()
            .find(|item| item.id == item_id && item.id == audio_id)
            .map(|item| item.start)
        else {
            drop(project);
            remove_generated(&generation.path);
            return Err("TTS generation item is no longer available".to_string());
        };
        let next_start = track
            .items
            .iter()
            .filter_map(|item| (item.id != item_id && item.start > start).then_some(item.start))
            .min();
        let generated_end = start
            .saturating_add(generation.duration)
            .snapped(project.frame_step());
        if generated_end <= start {
            drop(project);
            remove_generated(&generation.path);
            return Err("TTS generation produced no usable audio".to_string());
        }
        let Some(item) = project.audio_item_mut(address) else {
            drop(project);
            remove_generated(&generation.path);
            return Err("TTS generation item is no longer available".to_string());
        };
        let AudioSource::Tts(settings) = &mut item.source else {
            drop(project);
            remove_generated(&generation.path);
            return Err("TTS generation item no longer uses text to speech".to_string());
        };
        if settings.model.as_deref() == Some(model.id.as_str()) {
            shrimply_tts::apply_speed_factor(settings, model, generation.speed_factor);
        }
        item.file = generation.path.into();
        item.track_id = 0;
        item.time_offset = Time::ZERO;
        item.source_duration = generation.duration;
        item.end = shrimply_project_document::project::clamp_item_end_to_next_start(
            generated_end,
            next_start,
        );
        let duration = project.duration();
        shrimply_project_document::project::commit_edit(&project, "generate-tts");
        drop(project);
        player_state::refresh_project(
            &self.player_state,
            player_state::ProjectChange {
                duration: Some(duration),
                audio: true,
                audio_beats: true,
                audio_waveforms: true,
                inspector: true,
                ..Default::default()
            },
        );
        Ok(())
    }
}

fn remove_generated(path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
}
