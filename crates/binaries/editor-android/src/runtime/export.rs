use super::Runtime;
use serde_json::{Value, json};
use shrimply_caption_export_core::{self as captions, CaptionFormat, ExportMode};
use shrimply_export_core::{
    job::Job,
    video::{self, settings},
};
use shrimply_project_document::Project;
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tempfile::TempDir;

pub(super) struct Export {
    job: Job<(TempDir, Vec<PathBuf>)>,
    pub state: Value,
    completed_directory: Option<TempDir>,
}

enum Operation {
    Video(video::ExportSettings, usize),
    Captions(captions::ExportSettings, PathBuf),
}

impl Operation {
    fn run(
        self,
        project: Project,
        cancelled: Arc<AtomicBool>,
        progress: &mut dyn FnMut(video::ExportProgress),
    ) -> Result<Vec<PathBuf>, String> {
        match self {
            Self::Video(settings, limit) => {
                let path = settings.path.clone();
                shrimply_export_core::android::export_project(
                    project, settings, limit, cancelled, progress,
                )?;
                Ok(vec![path])
            }
            Self::Captions(settings, path) => {
                progress(video::ExportProgress::SettingUp("Exporting Captions"));
                let outputs = captions::prepare_export(&project, &path, settings)?;
                if cancelled.load(Ordering::Acquire) {
                    return Err("Export cancelled".into());
                }
                captions::write_export(outputs)
            }
        }
    }
}

impl Export {
    pub fn poll(&mut self) -> bool {
        let update = self.job.poll();
        let changed = update.progress.is_some() || update.finished.is_some();
        if let Some(progress) = update.progress {
            self.state["progress"] = video::progress::localized(progress);
        }
        if let Some(result) = update.finished {
            match result {
                Ok((directory, paths)) => {
                    self.completed_directory = Some(directory);
                    let files: Vec<_> = paths
                        .into_iter()
                        .map(|path| {
                            let filename = path
                                .file_name()
                                .expect("export filename")
                                .to_string_lossy()
                                .into_owned();
                            json!({"path":path,"filename":filename})
                        })
                        .collect();
                    if !self.state["separate"].as_bool().expect("export mode") {
                        assert_eq!(files.len(), 1, "merged export has exactly one output");
                        self.state["path"] = files[0]["path"].clone();
                        self.state["filename"] = files[0]["filename"].clone();
                    }
                    self.state["files"] = json!(files);
                    self.state["status"] = json!("complete");
                }
                Err(_) if self.job.cancelled() => self.state["status"] = json!("cancelled"),
                Err(error) => {
                    self.state["status"] = json!("failed");
                    self.state["error"] = json!(error);
                }
            }
        }
        if changed {
            self.state["presentation"] = video::progress::job_view(&self.state);
        }
        changed
    }
}

impl Runtime {
    pub fn poll_export(&mut self) -> bool {
        self.export.as_mut().is_some_and(Export::poll)
    }

    pub(super) fn export_request(&mut self, request: &Value) -> Result<Value, String> {
        let operation = request["op"].as_str().expect("validated request operation");
        match operation {
            "export_schema" => {
                let project = self.backend.project().ok_or("open a project first")?;
                Ok(settings::schema_for(
                    &project,
                    shrimply_export_core::android::settings_backend(),
                    request.get("values"),
                ))
            }
            "export_validate" => {
                let project = self.backend.project().ok_or("Open a project first")?;
                settings::parse_configuration(
                    &project,
                    PathBuf::new(),
                    request,
                    shrimply_export_core::android::settings_backend(),
                )?;
                Ok(json!({}))
            }
            "caption_export_schema" => Ok(json!({
                "formats":CaptionFormat::ALL.map(|format| json!({"value":format.extension(),"label":format.label()})),
                "modes":[{"value":"merge","label":"Merge into one file"},{"value":"separate","label":"Export each track separately"}]
            })),
            "export_video" | "export_captions" => self.start_export(request),
            "export_cancel" | "export_dismiss" | "export_saved" => {
                let export = self.export.as_mut().ok_or("No export is active")?;
                if request["id"] != export.state["id"] {
                    return Err("The export changed while this action was open".into());
                }
                if operation == "export_cancel" {
                    export.job.cancel();
                    export.state["cancelling"] = json!(true);
                    export.state["presentation"] = video::progress::job_view(&export.state);
                } else {
                    export.poll();
                    if export.state["status"] == "running" {
                        return Err("Wait for the export to stop before dismissing it".into());
                    }
                    if operation == "export_saved" && export.state["status"] != "complete" {
                        return Err("Only a completed export can be saved".into());
                    }
                    if let Some(directory) = &export.completed_directory {
                        // Keep ownership if removal fails so Close can be retried.
                        match std::fs::remove_dir_all(directory.path()) {
                            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                                return Err(error.to_string());
                            }
                            _ => {}
                        }
                    }
                    self.export = None;
                }
                self.backend.snapshot()
            }
            _ => unreachable!("export request dispatcher"),
        }
    }

    fn start_export(&mut self, request: &Value) -> Result<Value, String> {
        if self
            .export
            .as_ref()
            .is_some_and(|export| export.state["status"] == "running")
        {
            return Err("An export is already running".into());
        }
        self.backend.save()?;
        let project = self
            .backend
            .project()
            .ok_or("open a project first")?
            .clone();
        let id = uuid::Uuid::new_v4().to_string();
        let exports = shrimply_path_core::cache_directory()?.join("exports");
        std::fs::create_dir_all(&exports).map_err(|error| error.to_string())?;
        let staging = TempDir::new_in(exports).map_err(|error| error.to_string())?;
        let directory = staging.path().to_path_buf();
        // Project names are presentation text. Confine actual output to this job.
        let output_path = |extension: &str| -> Result<PathBuf, String> {
            let suggested = shrimply_export_core::output::default_filename(&project, extension);
            let filename = Path::new(&suggested)
                .file_name()
                .ok_or("Invalid export filename")?;
            Ok(directory.join(filename))
        };
        let is_video = request["op"] == "export_video";
        let (operation, mime, separate) = if is_video {
            let mut settings = settings::parse_configuration(
                &project,
                directory.join("video"),
                request,
                shrimply_export_core::android::settings_backend(),
            )?
            .common;
            let mime = video::mime_type(settings.container);
            settings.path = output_path(video::extension_for_container(settings.container))?;
            let limit = shrimply_preferences_core::snapshot(&self.backend.preferences)
                .temporal_decoder_pool_size as usize;
            (Operation::Video(settings, limit), mime, false)
        } else {
            let format = CaptionFormat::ALL
                .into_iter()
                .find(|format| request["format"] == format.extension())
                .ok_or("Unknown caption export format")?;
            let mode = match request["mode"].as_str() {
                Some("merge") => ExportMode::Merge,
                Some("separate") => ExportMode::Separate,
                _ => return Err("Unknown caption export track mode".into()),
            };
            (
                Operation::Captions(
                    captions::ExportSettings { format, mode },
                    output_path(format.extension())?,
                ),
                format.mime_type(),
                mode == ExportMode::Separate,
            )
        };
        let wake = self.backend.wake.clone();
        let job = Job::spawn(
            move |cancelled, progress| {
                let paths = operation.run(project, cancelled, progress)?;
                Ok((staging, paths))
            },
            move || wake(),
        )?;
        let mut state = json!({"id":id,"status":"running","directory":directory,"separate":separate,"files":[],"mime":mime,
            "kind":if is_video {"video"} else {"captions"},"progress":null,"error":null,"cancelling":false});
        state["presentation"] = video::progress::job_view(&state);
        self.export = Some(Export {
            job,
            completed_directory: None,
            state,
        });
        self.backend.snapshot()
    }
}
