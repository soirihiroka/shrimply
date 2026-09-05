use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};

use shrimply_3dgs::{
    COLMAP_TRACKING_MODEL, CameraSource, ColmapCameraModel, ColmapQuality, TrackingCameraSource,
    TrackingSettings, VGGT_SLAM_TRACKING_MODEL,
};
use shrimply_project::project::{ItemAddress, Project, VideoItemContent};
use shrimply_video_cuda::camera_reconstruction::{self, AnalysisStatus};
use strum::IntoEnumIterator;

use crate::{
    AnalysisControlPresentation, AnalysisTooltip, ControlKind, InspectorControl,
    InspectorControlAction, InspectorController, InspectorSection, InspectorTarget, NumberSpec,
};

pub const SOURCE_PATH: &str = "/content/camera/source";
pub const MODEL_PATH: &str = "/content/camera/source/tracking/settings/model";
pub const QUALITY_PATH: &str = "/content/camera/source/tracking/settings/quality";
pub const ANALYSIS_FPS_PATH: &str = "/content/camera/source/tracking/settings/analysis_fps";
pub const CAMERA_MODEL_PATH: &str = "/content/camera/source/tracking/settings/camera_model";
pub const ANALYSIS_STATUS_PATH: &str = "/content/camera/source/tracking/analysis/status";
pub const ANALYZE_PATH: &str = "/content/camera/source/tracking/analysis/action";

const SOURCE_COMMIT: &str = "edit-camera-source";
const MODEL_COMMIT: &str = "edit-3d-tracking-model";
const QUALITY_COMMIT: &str = "edit-colmap-quality";
const ANALYSIS_FPS_COMMIT: &str = "edit-colmap-analysis-fps";
const CAMERA_MODEL_COMMIT: &str = "edit-colmap-camera-model";

pub type TrackingModels = Result<Vec<String>, String>;
type TrackingModelCatalogs = HashMap<String, TrackingModels>;

static MODEL_CATALOGS: OnceLock<Mutex<TrackingModelCatalogs>> = OnceLock::new();

#[derive(Clone, Debug, PartialEq)]
pub struct CameraSourcePresentation {
    pub custom: bool,
    pub section: InspectorSection,
}

pub fn presentation(
    project: &Project,
    address: &ItemAddress,
    source: &CameraSource,
    models: Option<&TrackingModels>,
) -> CameraSourcePresentation {
    let status = tracking_status(project, address, source);
    let running = status.as_ref().is_some_and(analysis_running);
    let mut section = InspectorSection::default();

    let mut source_values = vec!["custom".to_string()];
    let mut source_labels = vec!["Custom".to_string()];
    for (index, track) in project
        .video_tracks_for_path(address.sequence_path())
        .into_iter()
        .flat_map(|tracks| tracks.iter())
        .enumerate()
        .filter(|(_, track)| track.id != address.track_id())
    {
        source_values.push(track.id.to_string());
        source_labels.push(shrimply_i18n_core::text_args(
            "Visual track %{number}",
            &[("number", index.to_string())],
        ));
    }
    let selected_source = match source {
        CameraSource::Custom => "custom".to_string(),
        CameraSource::Tracking(source) => source.track_id.to_string(),
    };
    if !source_values.contains(&selected_source) {
        source_values.push(selected_source.clone());
        source_labels.push(shrimply_i18n_core::text_args(
            "Unavailable (%{id})",
            &[("id", selected_source.clone())],
        ));
    }
    section.add(
        crate::selector::selector(
            SOURCE_PATH,
            "Camera source",
            selected_source,
            source_values.into_iter().zip(source_labels),
        )
        .sensitive(!running)
        .immediate_commit(SOURCE_COMMIT),
    );

    let CameraSource::Tracking(source) = source else {
        return CameraSourcePresentation {
            custom: true,
            section,
        };
    };

    let available = models.and_then(|models| models.as_ref().ok());
    let mut model_values = available
        .cloned()
        .unwrap_or_else(|| vec![source.settings.model.clone()]);
    if !model_values.contains(&source.settings.model) {
        model_values.push(source.settings.model.clone());
    }
    let model_labels = model_values
        .iter()
        .map(|model| {
            let label = match model.as_str() {
                COLMAP_TRACKING_MODEL => "COLMAP".to_string(),
                VGGT_SLAM_TRACKING_MODEL => "VGGT-SLAM".to_string(),
                _ => model.clone(),
            };
            if available.is_some_and(|available| !available.contains(model)) {
                shrimply_i18n_core::text_args("%{label} (Unavailable)", &[("label", label)])
            } else {
                label
            }
        })
        .collect::<Vec<_>>();
    section.add(
        crate::selector::selector(
            MODEL_PATH,
            "Tracking method",
            source.settings.model.clone(),
            model_values.into_iter().zip(model_labels),
        )
        .sensitive(!running && available.is_some_and(|models| !models.is_empty()))
        .immediate_commit(MODEL_COMMIT),
    );

    if source.settings.model == COLMAP_TRACKING_MODEL {
        section.add(
            crate::selector::selector(
                QUALITY_PATH,
                "Quality",
                enum_value(source.settings.quality),
                enum_choices::<ColmapQuality>(),
            )
            .sensitive(!running)
            .immediate_commit(QUALITY_COMMIT),
        );
        section.add(
            crate::selector::selector(
                CAMERA_MODEL_PATH,
                "Camera model",
                enum_value(source.settings.camera_model),
                enum_choices::<ColmapCameraModel>(),
            )
            .sensitive(!running)
            .immediate_commit(CAMERA_MODEL_COMMIT),
        );
    }

    section.add(
        InspectorControl::new(ControlKind::Number, ANALYSIS_FPS_PATH, "Analysis FPS")
            .value(source.settings.analysis_fps.to_string())
            .number(NumberSpec {
                minimum: 1.0,
                maximum: 60.0,
                drag_step: 1.0,
                digits: 0,
                unit: "",
            })
            .integer()
            .sensitive(!running)
            .immediate_commit(ANALYSIS_FPS_COMMIT),
    );

    let status_text = match models {
        None => "Checking compute server...".to_string(),
        Some(Err(error)) => format!("Server unavailable: {error}"),
        Some(Ok(models)) if !models.contains(&source.settings.model) => {
            "Selected tracking method is unavailable".to_string()
        }
        _ => status_label(
            status
                .as_ref()
                .expect("tracked camera must have analysis status"),
        ),
    };
    section.add(
        InspectorControl::new(
            ControlKind::ReadOnly,
            ANALYSIS_STATUS_PATH,
            "Analysis status",
        )
        .value(status_text)
        .busy(running),
    );
    let can_analyze = available.is_some_and(|models| models.contains(&source.settings.model));
    section.add(analysis_control(
        status
            .as_ref()
            .expect("tracked camera must have analysis status"),
        camera_reconstruction::has_matching_cache(selected_item_id(project, address), source),
        can_analyze,
    ));

    CameraSourcePresentation {
        custom: false,
        section,
    }
}

pub fn cached_tracking_models(server_url: &str) -> Option<TrackingModels> {
    MODEL_CATALOGS
        .get_or_init(Mutex::default)
        .lock()
        .expect("camera tracking model cache must not be poisoned")
        .get(server_url)
        .cloned()
}

pub fn tracking_models(server_url: &str) -> TrackingModels {
    if let Some(models) = cached_tracking_models(server_url) {
        return models;
    }
    let models = shrimply_server_client::server_status(server_url).and_then(|status| {
        let models = status
            .capabilities
            .into_iter()
            .filter_map(|capability| capability.strip_prefix("3dtracking:").map(str::to_string))
            .collect::<Vec<_>>();
        (!models.is_empty())
            .then_some(models)
            .ok_or_else(|| "server does not advertise 3D tracking".to_string())
    });
    MODEL_CATALOGS
        .get_or_init(Mutex::default)
        .lock()
        .expect("camera tracking model cache must not be poisoned")
        .insert(server_url.to_string(), models.clone());
    models
}

pub fn status_label(status: &AnalysisStatus) -> String {
    match status {
        AnalysisStatus::NotAnalyzed => shrimply_i18n_core::text("Not analyzed").into_owned(),
        AnalysisStatus::OutOfDate => shrimply_i18n_core::text("Out of date").into_owned(),
        AnalysisStatus::Queued => shrimply_i18n_core::text("Queued").into_owned(),
        AnalysisStatus::Loading => shrimply_i18n_core::text("Loading tracking model").into_owned(),
        AnalysisStatus::Analyzing {
            message,
            completed_frames,
            total_frames,
        } if *total_frames != 0 => shrimply_i18n_core::text_args(
            "%{message} %{completed}/%{total}",
            &[
                ("message", shrimply_i18n_core::text(message).into_owned()),
                ("completed", completed_frames.to_string()),
                ("total", total_frames.to_string()),
            ],
        ),
        AnalysisStatus::Analyzing { message, .. } => shrimply_i18n_core::text(message).into_owned(),
        AnalysisStatus::Cancelling => shrimply_i18n_core::text("Cancelling").into_owned(),
        AnalysisStatus::Cancelled => shrimply_i18n_core::text("Cancelled").into_owned(),
        AnalysisStatus::Ready { sample_count } => shrimply_i18n_core::text_args(
            "Ready (%{count} samples)",
            &[("count", sample_count.to_string())],
        ),
        AnalysisStatus::Failed { error } => format!("Failed: {error}"),
        AnalysisStatus::MissingSourceTrack => {
            shrimply_i18n_core::text("Source track unavailable").into_owned()
        }
        AnalysisStatus::EmptySourceTrack => {
            shrimply_i18n_core::text("Source track is empty").into_owned()
        }
    }
}

impl InspectorController {
    pub fn set_camera_source_field(
        &self,
        target: &InspectorTarget,
        path: &str,
        text: &str,
    ) -> Option<Result<(), String>> {
        let commit_name = match path {
            SOURCE_PATH => SOURCE_COMMIT,
            MODEL_PATH => MODEL_COMMIT,
            QUALITY_PATH => QUALITY_COMMIT,
            ANALYSIS_FPS_PATH => ANALYSIS_FPS_COMMIT,
            CAMERA_MODEL_PATH => CAMERA_MODEL_COMMIT,
            _ => return None,
        };
        Some(self.update_camera_source(target, commit_name, |source| {
            match path {
                SOURCE_PATH => {
                    let settings = match source {
                        CameraSource::Custom => TrackingSettings::default(),
                        CameraSource::Tracking(source) => source.settings.clone(),
                    };
                    *source = if text == "custom" {
                        CameraSource::Custom
                    } else {
                        CameraSource::Tracking(TrackingCameraSource {
                            track_id: text
                                .parse()
                                .map_err(|_| format!("invalid camera source track: {text}"))?,
                            settings,
                        })
                    };
                    Ok(())
                }
                MODEL_PATH => {
                    tracking_settings(source).map(|settings| settings.model = text.to_string())
                }
                QUALITY_PATH => parse_enum(text).and_then(|value| {
                    tracking_settings(source).map(|settings| settings.quality = value)
                }),
                CAMERA_MODEL_PATH => parse_enum(text).and_then(|value| {
                    tracking_settings(source).map(|settings| settings.camera_model = value)
                }),
                ANALYSIS_FPS_PATH => text
                    .parse::<u32>()
                    .map_err(|_| format!("invalid camera analysis FPS: {text}"))
                    .and_then(|fps| {
                        tracking_settings(source)
                            .map(|settings| settings.analysis_fps = fps.clamp(1, 60))
                    }),
                _ => unreachable!(),
            }
        }))
    }

    pub fn camera_analysis_control(
        &self,
        target: &InspectorTarget,
        server_url: &str,
    ) -> Result<AnalysisControlPresentation, String> {
        self.camera_analysis_state(target, server_url)
            .map(|(presentation, _)| presentation)
    }

    pub fn camera_analysis_state(
        &self,
        target: &InspectorTarget,
        server_url: &str,
    ) -> Result<(AnalysisControlPresentation, String), String> {
        let InspectorTarget::Item(address) = target else {
            return Err("camera source target is not an item".to_string());
        };
        let project = self.project.borrow();
        let source = selected_source(&project, address)?;
        let CameraSource::Tracking(source) = source else {
            return Err("custom camera has no tracking analysis".to_string());
        };
        let status = project_status(&project, address, source);
        let models = cached_tracking_models(server_url);
        let can_analyze = models
            .as_ref()
            .and_then(|models| models.as_ref().ok())
            .is_some_and(|models| models.contains(&source.settings.model));
        Ok((
            analysis_presentation(
                &status,
                camera_reconstruction::has_matching_cache(
                    selected_item_id(&project, address),
                    source,
                ),
                can_analyze,
            ),
            status_label(&status),
        ))
    }

    pub fn toggle_camera_analysis(
        &self,
        target: &InspectorTarget,
        server_url: String,
    ) -> Result<(), String> {
        let InspectorTarget::Item(address) = target else {
            return Err("camera source target is not an item".to_string());
        };
        let project = self.project.borrow();
        let source = selected_source(&project, address)?;
        let CameraSource::Tracking(source) = source else {
            return Err("custom camera has no tracking analysis".to_string());
        };
        let source = source.clone();
        let item_id = selected_item_id(&project, address);
        let active = matches!(
            camera_reconstruction::status(item_id, &source),
            AnalysisStatus::Queued | AnalysisStatus::Loading | AnalysisStatus::Analyzing { .. }
        );
        if active {
            camera_reconstruction::cancel(item_id, &source);
        } else {
            camera_reconstruction::analyze((*project).clone(), item_id, source, server_url);
        }
        drop(project);
        self.refresh_analysis_output();
        Ok(())
    }

    fn update_camera_source(
        &self,
        target: &InspectorTarget,
        commit_name: &'static str,
        update: impl FnOnce(&mut CameraSource) -> Result<(), String>,
    ) -> Result<(), String> {
        let InspectorTarget::Item(address) = target else {
            return Err("camera source target is not an item".to_string());
        };
        let mut project = self.project.borrow_mut();
        let source = selected_source_mut(&mut project, address)?;
        let previous = source.clone();
        update(source)?;
        if *source == previous {
            return Ok(());
        }
        shrimply_project::project::commit_edit(&project, commit_name);
        drop(project);
        self.refresh_analysis_output();
        Ok(())
    }
}

fn analysis_control(
    status: &AnalysisStatus,
    matching: bool,
    can_analyze: bool,
) -> InspectorControl {
    let presentation = analysis_presentation(status, matching, can_analyze);
    let mut control = InspectorControl::new(ControlKind::Analysis, ANALYZE_PATH, "")
        .value(presentation.label.clone())
        .components(vec![
            presentation.progress.to_string(),
            u8::from(presentation.running).to_string(),
            u8::from(presentation.cancelling).to_string(),
            u8::from(presentation.suggested).to_string(),
        ])
        .sensitive(presentation.sensitive)
        .busy(presentation.active())
        .action(InspectorControlAction::ToggleCameraAnalysis);
    control.analysis = Some(presentation);
    control
}

fn analysis_presentation(
    status: &AnalysisStatus,
    matching: bool,
    can_analyze: bool,
) -> AnalysisControlPresentation {
    let running = analysis_running(status);
    let cancellable = matches!(
        status,
        AnalysisStatus::Queued | AnalysisStatus::Loading | AnalysisStatus::Analyzing { .. }
    );
    let progress = match status {
        AnalysisStatus::Analyzing {
            completed_frames,
            total_frames,
            ..
        } if *total_frames != 0 => *completed_frames as f64 / *total_frames as f64,
        _ => -1.0,
    };
    AnalysisControlPresentation {
        label: if cancellable {
            "Cancel"
        } else if matches!(status, AnalysisStatus::Cancelling) {
            "Cancelling…"
        } else if matching
            || matches!(
                status,
                AnalysisStatus::OutOfDate | AnalysisStatus::Ready { .. }
            )
        {
            "Analyze Again"
        } else {
            "Analyze"
        }
        .to_string(),
        progress,
        tooltip: AnalysisTooltip::MessageKey(""),
        sensitive: cancellable || (!running && can_analyze),
        running,
        cancelling: matches!(status, AnalysisStatus::Cancelling),
        terminal: !running,
        suggested: false,
    }
}

fn tracking_status(
    project: &Project,
    address: &ItemAddress,
    source: &CameraSource,
) -> Option<AnalysisStatus> {
    let CameraSource::Tracking(source) = source else {
        return None;
    };
    Some(project_status(project, address, source))
}

fn project_status(
    project: &Project,
    address: &ItemAddress,
    source: &TrackingCameraSource,
) -> AnalysisStatus {
    let Some(track) = project
        .video_tracks_for_path(address.sequence_path())
        .into_iter()
        .flat_map(|tracks| tracks.iter())
        .find(|track| track.id == source.track_id)
    else {
        return AnalysisStatus::MissingSourceTrack;
    };
    if address.track_id() == track.id {
        return AnalysisStatus::Failed {
            error: "A 3D item cannot track its own visual track".to_string(),
        };
    }
    if track.items.is_empty() || track.items.iter().all(|item| item.end <= item.start) {
        return AnalysisStatus::EmptySourceTrack;
    }
    camera_reconstruction::status(selected_item_id(project, address), source)
}

fn analysis_running(status: &AnalysisStatus) -> bool {
    matches!(
        status,
        AnalysisStatus::Queued
            | AnalysisStatus::Loading
            | AnalysisStatus::Analyzing { .. }
            | AnalysisStatus::Cancelling
    )
}

fn selected_item_id(project: &Project, address: &ItemAddress) -> uuid::Uuid {
    project
        .video_item(address)
        .expect("camera source presentation requires an available 3D item")
        .id
}

fn selected_source<'a>(
    project: &'a Project,
    address: &ItemAddress,
) -> Result<&'a CameraSource, String> {
    let item = project
        .video_item(address)
        .ok_or_else(|| "camera source item is no longer available".to_string())?;
    match &item.content {
        VideoItemContent::Obj(scene) => Ok(&scene.camera.source),
        VideoItemContent::Gaussian(scene) => Ok(&scene.camera.source),
        _ => Err("camera source item is not a 3D scene".to_string()),
    }
}

fn selected_source_mut<'a>(
    project: &'a mut Project,
    address: &ItemAddress,
) -> Result<&'a mut CameraSource, String> {
    let item = project
        .video_item_mut(address)
        .ok_or_else(|| "camera source item is no longer available".to_string())?;
    match &mut item.content {
        VideoItemContent::Obj(scene) => Ok(&mut scene.camera.source),
        VideoItemContent::Gaussian(scene) => Ok(&mut scene.camera.source),
        _ => Err("camera source item is not a 3D scene".to_string()),
    }
}

fn tracking_settings(source: &mut CameraSource) -> Result<&mut TrackingSettings, String> {
    let CameraSource::Tracking(source) = source else {
        return Err("camera tracking settings are no longer available".to_string());
    };
    Ok(&mut source.settings)
}

fn enum_value(value: impl serde::Serialize) -> String {
    serde_json::to_value(value)
        .expect("camera setting enum must serialize")
        .as_str()
        .expect("camera setting enum must serialize as text")
        .to_string()
}

fn enum_choices<T>() -> impl Iterator<Item = (String, String)>
where
    T: IntoEnumIterator + serde::Serialize + ToString,
{
    T::iter().map(|value| (enum_value(&value), value.to_string()))
}

fn parse_enum<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, String> {
    serde_json::from_value(serde_json::Value::String(text.to_string()))
        .map_err(|error| format!("invalid camera setting {text}: {error}"))
}
